//! MPD connection worker.
//!
//! Owns the [`Client`], executes commands from the app, watches connection
//! events for subsystem changes, and periodically refreshes a full
//! status + queue snapshot which is forwarded to the UI. Also implements the
//! "interrupt preview" lifecycle used by `music-tui open --mode interrupt`.

use std::time::Duration;

use mpd_client::{
  client::ConnectionEvent,
  commands::{
    self, Add, ClearQueue, Delete, DeletePlaylist, LoadPlaylist, Play, Previous,
    SaveQueueAsPlaylist, Seek, SeekMode, SetConsume, SetPause, SetRandom, SetRepeat, SetSingle,
    SetVolume, Stop, SingleMode, SongPosition,
  },
  responses::{PlayState, SongInQueue, Status},
  Client,
};
use tokio::{net::TcpStream, sync::mpsc, time::sleep};
use tracing::{debug, info, warn};

use crate::{
  config::MpdConfig,
  event::{AsyncEvent, MpdEvent},
};

/// Saved playback state to restore after an interrupt preview finishes.
#[derive(Debug, Clone)]
pub struct InterruptSession {
  /// Stored playlist holding the previous queue, if the queue was non-empty.
  pub playlist: Option<String>,
  pub was_playing: bool,
  pub position: Option<u32>,
  pub elapsed_secs: Option<f64>,
  pub single: SingleMode,
}

#[derive(Debug)]
pub enum MpdCommand {
  PlayPosition(u32),
  PlayPauseToggle,
  Pause(bool),
  Stop,
  Next,
  Previous,
  SetVolume(u8),
  NudgeVolume(i16),
  SeekCurrent(f64),
  NudgeSeek(i64),
  SetRepeat(bool),
  SetRandom(bool),
  SetSingle(SingleMode),
  SetConsume(bool),
  ClearQueue,
  DeleteAt(usize),
  AddUri(String),
  Rescan,
  ArmInterrupt(InterruptSession),
}

#[derive(Clone)]
pub struct MpdHandle {
  tx: mpsc::UnboundedSender<MpdCommand>,
}

impl MpdHandle {
  pub fn send(&self, command: MpdCommand) {
    let _ = self.tx.send(command);
  }
}

pub fn spawn_mpd_worker(
  config: MpdConfig,
  events: mpsc::UnboundedSender<AsyncEvent>,
) -> MpdHandle {
  let (tx, mut rx) = mpsc::unbounded_channel();
  tokio::spawn(async move {
    let mut backoff = Duration::from_secs(1);
    loop {
      match connect(&config).await {
        Ok((client, mut connection_events)) => {
          backoff = Duration::from_secs(1);
          let address = describe_address(&config);
          info!(%address, "connected to mpd");
          let _ = events.send(AsyncEvent::Mpd(MpdEvent::Connected(address)));
          if let Err(error) = run_session(&client, &mut connection_events, &mut rx, &events).await {
            warn!(%error, "mpd session ended");
          }
          let _ = events.send(AsyncEvent::Mpd(MpdEvent::ConnectionLost(
            "connection closed".to_string(),
          )));
        }
        Err(error) => {
          warn!(%error, "failed to connect to mpd");
          let _ = events.send(AsyncEvent::Mpd(MpdEvent::ConnectionLost(format!(
            "connect failed: {error}"
          ))));
        }
      }
      debug!(?backoff, "reconnecting to mpd");
      sleep(backoff).await;
      backoff = (backoff * 2).min(Duration::from_secs(30));
    }
  });
  MpdHandle { tx }
}

pub async fn connect(config: &MpdConfig) -> anyhow::Result<(Client, mpd_client::client::ConnectionEvents)> {
  let host = crate::config::expand_home(&config.host);
  if host.to_string_lossy().starts_with('/') {
    let stream = tokio::net::UnixStream::connect(&host).await?;
    let (client, events) = Client::connect_with_password_opt(stream, config.password.as_deref())
      .await
      .map_err(|error| anyhow::anyhow!(format!("{error}")))?;
    return Ok((client, events));
  }

  let stream = TcpStream::connect((host.to_string_lossy().as_ref(), config.port)).await?;
  let (client, events) = Client::connect_with_password_opt(stream, config.password.as_deref())
    .await
    .map_err(|error| anyhow::anyhow!(format!("{error}")))?;
  Ok((client, events))
}

fn describe_address(config: &MpdConfig) -> String {
  if config.host.starts_with('/') {
    config.host.clone()
  } else {
    format!("{}:{}", config.host, config.port)
  }
}

struct SessionState {
  interrupt: Option<InterruptSession>,
  playing: bool,
  queue: Vec<SongInQueue>,
}

async fn run_session(
  client: &Client,
  connection_events: &mut mpd_client::client::ConnectionEvents,
  command_rx: &mut mpsc::UnboundedReceiver<MpdCommand>,
  events: &mpsc::UnboundedSender<AsyncEvent>,
) -> anyhow::Result<()> {
  let mut state = SessionState {
    interrupt: None,
    playing: false,
    queue: Vec::new(),
  };

  let status = client.command(commands::Status).await?;
  state.playing = status.state == PlayState::Playing;
  refresh(client, &mut state, events).await?;

  let tick_idle = Duration::from_millis(1000);
  let tick_playing = Duration::from_millis(250);

  loop {
    let period = if state.playing { tick_playing } else { tick_idle };
    tokio::select! {
      event = connection_events.next() => match event {
        Some(ConnectionEvent::SubsystemChange(subsystem)) => {
          debug!(subsystem = subsystem.as_str(), "mpd subsystem changed");
          refresh(client, &mut state, events).await?;
        }
        Some(ConnectionEvent::ConnectionClosed(error)) => {
          return Err(anyhow::anyhow!(format!("{error}")));
        }
        None => {
          return Err(anyhow::anyhow!("connection event stream ended"));
        }
      },
      command = command_rx.recv() => {
        let Some(command) = command else {
          return Err(anyhow::anyhow!("command channel closed"));
        };
        match command {
          MpdCommand::ArmInterrupt(session) => {
            state.interrupt = Some(session);
          }
          command => {
            if command_touches_queue(&command) {
              state.interrupt = None;
            }
            run_command(client, command).await;
          }
        }
        refresh(client, &mut state, events).await?;
      }
      _ = sleep(period) => {
        refresh_status(client, &mut state, events).await?;
      }
    }
  }
}

fn command_touches_queue(command: &MpdCommand) -> bool {
  matches!(
    command,
    MpdCommand::PlayPosition(_)
      | MpdCommand::PlayPauseToggle
      | MpdCommand::Stop
      | MpdCommand::Next
      | MpdCommand::Previous
      | MpdCommand::ClearQueue
      | MpdCommand::DeleteAt(_)
      | MpdCommand::AddUri(_)
  )
}

async fn refresh(
  client: &Client,
  state: &mut SessionState,
  events: &mpsc::UnboundedSender<AsyncEvent>,
) -> anyhow::Result<()> {
  let status = client.command(commands::Status).await?;
  let queue = client.command(commands::Queue).await?;
  state.playing = status.state == PlayState::Playing;
  state.queue = queue.clone();
  maybe_restore_interrupt(client, state, &status, events).await?;
  let _ = events.send(AsyncEvent::Mpd(MpdEvent::Snapshot { status, queue }));
  Ok(())
}

async fn refresh_status(
  client: &Client,
  state: &mut SessionState,
  events: &mpsc::UnboundedSender<AsyncEvent>,
) -> anyhow::Result<()> {
  let status = client.command(commands::Status).await?;
  state.playing = status.state == PlayState::Playing;
  if maybe_restore_interrupt(client, state, &status, events).await? {
    refresh(client, state, events).await?;
    return Ok(());
  }
  let _ = events.send(AsyncEvent::Mpd(MpdEvent::Snapshot {
    status,
    queue: state.queue.clone(),
  }));
  Ok(())
}

/// After an interrupt preview stops, rebuild the previous queue and state.
/// Returns true when a restore was performed.
async fn maybe_restore_interrupt(
  client: &Client,
  state: &mut SessionState,
  status: &Status,
  events: &mpsc::UnboundedSender<AsyncEvent>,
) -> anyhow::Result<bool> {
  if status.state != PlayState::Stopped {
    return Ok(false);
  }
  let Some(session) = state.interrupt.take() else {
    return Ok(false);
  };
  info!("interrupt preview finished; restoring previous queue");

  client.command(ClearQueue).await?;
  if let Some(playlist) = &session.playlist {
    if let Err(error) = client.command(LoadPlaylist::name(playlist)).await {
      warn!(%error, playlist = %playlist, "failed to restore saved playlist");
    }
    let _ = client
      .command(DeletePlaylist(playlist.as_str()))
      .await;
  }
  let _ = client.command(SetSingle(session.single)).await;
  if let Some(position) = session.position {
    let queue_len = client.command(commands::Queue).await?.len();
    if (position as usize) < queue_len {
      client.command(Play::song(SongPosition(position as usize))).await?;
      if let Some(elapsed) = session.elapsed_secs.filter(|secs| *secs > 0.5) {
        let _ = client
          .command(Seek(SeekMode::Absolute(Duration::from_secs_f64(elapsed))))
          .await;
      }
      if !session.was_playing {
        let _ = client.command(SetPause(true)).await;
      }
    }
  }
  let _ = events.send(AsyncEvent::Mpd(MpdEvent::Notice(
    "preview finished; restored previous queue".to_string(),
  )));
  Ok(true)
}

async fn run_command(client: &Client, command: MpdCommand) {
  let outcome: Result<(), String> = match command {
    MpdCommand::PlayPosition(position) => client
      .command(Play::song(SongPosition(position as usize)))
      .await
      .map(|_| ()).map_err(|error| error.to_string()),
    MpdCommand::PlayPauseToggle => {
      let state = client.command(commands::Status).await.map(|s| s.state);
      match state {
        Ok(PlayState::Playing) => client.command(SetPause(true)).await.map(|_| ()).map_err(|error| error.to_string()),
        _ => client.command(Play::current()).await.map(|_| ()).map_err(|error| error.to_string()),
      }
    }
    MpdCommand::Pause(pause) => client.command(SetPause(pause)).await.map(|_| ()).map_err(|error| error.to_string()),
    MpdCommand::Stop => client.command(Stop).await.map(|_| ()).map_err(|error| error.to_string()),
    MpdCommand::Next => client.command(commands::Next).await.map(|_| ()).map_err(|error| error.to_string()),
    MpdCommand::Previous => client.command(Previous).await.map(|_| ()).map_err(|error| error.to_string()),
    MpdCommand::SetVolume(volume) => client.command(SetVolume(volume)).await.map(|_| ()).map_err(|error| error.to_string()),
    MpdCommand::NudgeVolume(delta) => {
      let current = client.command(commands::Status).await.map(|s| s.volume);
      match current {
        Ok(current) => {
          let next = (i32::from(current) + i32::from(delta)).clamp(0, 100) as u8;
          client.command(SetVolume(next)).await.map(|_| ()).map_err(|error| error.to_string())
        }
        Err(error) => Err(error.to_string()),
      }
    }
    MpdCommand::SeekCurrent(seconds) => client
      .command(Seek(SeekMode::Absolute(Duration::from_secs_f64(
        seconds.max(0.0),
      ))))
      .await
      .map(|_| ())
      .map_err(|error| error.to_string()),
    MpdCommand::NudgeSeek(delta) => {
      let mode = if delta >= 0 {
        SeekMode::Forward(Duration::from_secs(delta.unsigned_abs()))
      } else {
        SeekMode::Backward(Duration::from_secs(delta.unsigned_abs()))
      };
      client
        .command(Seek(mode))
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
    }
    MpdCommand::SetRepeat(repeat) => client.command(SetRepeat(repeat)).await.map(|_| ()).map_err(|error| error.to_string()),
    MpdCommand::SetRandom(random) => client.command(SetRandom(random)).await.map(|_| ()).map_err(|error| error.to_string()),
    MpdCommand::SetSingle(mode) => client.command(SetSingle(mode)).await.map(|_| ()).map_err(|error| error.to_string()),
    MpdCommand::SetConsume(consume) => client.command(SetConsume(consume)).await.map(|_| ()).map_err(|error| error.to_string()),
    MpdCommand::ClearQueue => client.command(ClearQueue).await.map(|_| ()).map_err(|error| error.to_string()),
    MpdCommand::DeleteAt(position) => {
      client
        .command(Delete::range(
        SongPosition(position)..SongPosition(position + 1),
      ))
        .await
        .map(|_| ()).map_err(|error| error.to_string())
    }
    MpdCommand::AddUri(uri) => client
      .command(Add::uri(&uri))
      .await
      .map(|_| ()).map_err(|error| error.to_string()),
    MpdCommand::Rescan => client
      .command(commands::Rescan::new())
      .await
      .map(|_| ()).map_err(|error| error.to_string()),
    MpdCommand::ArmInterrupt(_) => Ok(()),
  };
  if let Err(error) = outcome {
    warn!(%error, "mpd command failed");
  }
}

/// Best-effort helper used by `open --mode interrupt` to snapshot state.
pub async fn capture_interrupt_session(client: &Client) -> anyhow::Result<InterruptSession> {
  let status = client.command(commands::Status).await?;
  let playlist = if status.playlist_length > 0 {
    let stamp = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs();
    let name = format!("music-tui-preview-{stamp}");
    client
      .command(SaveQueueAsPlaylist(name.as_str()))
      .await?;
    Some(name)
  } else {
    None
  };
  Ok(InterruptSession {
    playlist,
    was_playing: status.state == PlayState::Playing,
    position: status.current_song.map(|(pos, _)| pos.0 as u32),
    elapsed_secs: status.elapsed.map(|elapsed| elapsed.as_secs_f64()),
    single: status.single,
  })
}


