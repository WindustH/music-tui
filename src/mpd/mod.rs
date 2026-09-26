//! MPD connection worker.
//!
//! Owns the [`Client`], executes commands from the app, watches connection
//! events for subsystem changes, and refreshes status (plus the queue when
//! MPD's playlist version moves) which is forwarded to the UI. Also
//! implements the "interrupt preview" lifecycle used by
//! `music-tui open --mode interrupt`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mpd_client::{
  Client,
  commands::{SingleMode, SongId},
};
use tokio::{net::TcpStream, sync::mpsc, time::sleep};
use tracing::{debug, info, warn};

use crate::{
  config::{BehaviorConfig, MpdConfig},
  event::{AsyncEvent, MpdEvent},
};

mod commands;
mod interrupt;
mod worker;

use interrupt::RestoreState;
pub use interrupt::capture_interrupt_session;

/// Upper bound for establishing a connection (TCP/socket connect plus the
/// greeting and password exchange). Without it a black-holed host stalls
/// the worker for the kernel's SYN timeout (minutes on Linux).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Reconnect delays double from `MIN_BACKOFF` up to `MAX_BACKOFF`.
const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// A session at least this long counts as healthy and resets the backoff.
const STABLE_SESSION: Duration = Duration::from_secs(10);

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
  /// Play the queue entry with this id. Ids (unlike positions) stay valid
  /// when the queue shifts between the UI's snapshot and the command.
  PlaySong(SongId),
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
  /// Shuffle the entire queue.
  Shuffle,
  /// Remove the queue entry with this id.
  Delete(SongId),
  AddUri(String),
  /// Play (or append) a local file from the library pane. The path is
  /// resolved to an MPD URI (music dir relative, `file://`, or symlink
  /// bridge) before being added to the queue.
  PlayLibrary {
    path: std::path::PathBuf,
    append: bool,
  },
  Rescan,
  /// Incremental database update for one URI (used after tag writes).
  UpdateUri(String),
  ArmInterrupt(InterruptSession),
}

#[derive(Clone)]
pub struct MpdHandle {
  tx: mpsc::UnboundedSender<MpdCommand>,
  queue_dedup: Arc<AtomicBool>,
}

impl MpdHandle {
  pub fn send(&self, command: MpdCommand) {
    let _ = self.tx.send(command);
  }

  /// Live toggle for add-time duplicate skipping in the worker.
  pub fn set_queue_dedup(&self, on: bool) {
    self.queue_dedup.store(on, Ordering::Relaxed);
  }
}

pub fn spawn_mpd_worker(
  config: MpdConfig,
  behavior: BehaviorConfig,
  events: mpsc::UnboundedSender<AsyncEvent>,
) -> MpdHandle {
  let (tx, mut rx) = mpsc::unbounded_channel();
  let queue_dedup = Arc::new(AtomicBool::new(behavior.queue_dedup));
  let worker_dedup = queue_dedup.clone();
  tokio::spawn(async move {
    let mut backoff = MIN_BACKOFF;
    // The interrupt session outlives individual connections: a dropped
    // connection mid-preview must still restore the saved queue later.
    let mut restore = RestoreState::default();
    loop {
      match connect(&config).await {
        Ok((client, mut connection_events)) => {
          let address = describe_address(&config);
          info!(%address, "connected to mpd");
          let _ = events.send(AsyncEvent::Mpd(MpdEvent::Connected(address)));
          // Commands issued while the connection was down are stale by now
          // (replaying five queued `next` presses would surprise the user).
          drain_offline_commands(&mut rx, &mut restore, &events);
          let started = tokio::time::Instant::now();
          let session = worker::Session {
            client: &client,
            commands: &mut rx,
            events: &events,
            config: &config,
            behavior: &behavior,
            dedup: &worker_dedup,
            restore: &mut restore,
          };
          if let Err(error) = session.run(&mut connection_events).await {
            warn!(%error, "mpd session ended");
          }
          let _ = events.send(AsyncEvent::Mpd(MpdEvent::ConnectionLost(
            "connection closed".to_string(),
          )));
          // Only a session that worked for a while resets the backoff: one
          // that fails right after connecting must not reconnect every
          // second forever.
          if started.elapsed() >= STABLE_SESSION {
            backoff = MIN_BACKOFF;
          }
        }
        Err(error) => {
          warn!(%error, "failed to connect to mpd");
          let _ = events.send(AsyncEvent::Mpd(MpdEvent::ConnectionLost(format!(
            "connect failed: {error}"
          ))));
        }
      }
      debug!(?backoff, "reconnecting to mpd");
      if !wait_offline(backoff, &mut rx, &mut restore, &events).await {
        break;
      }
      backoff = (backoff * 2).min(MAX_BACKOFF);
    }
  });
  MpdHandle { tx, queue_dedup }
}

/// Sleep out the reconnect backoff while still answering the app: an
/// interrupt hand-off is kept for the next session, everything else is
/// refused with a notice. Returns false once the app has shut down.
async fn wait_offline(
  backoff: Duration,
  commands: &mut mpsc::UnboundedReceiver<MpdCommand>,
  restore: &mut RestoreState,
  events: &mpsc::UnboundedSender<AsyncEvent>,
) -> bool {
  let deadline = sleep(backoff);
  tokio::pin!(deadline);
  loop {
    tokio::select! {
      _ = &mut deadline => return true,
      command = commands.recv() => match command {
        Some(command) => handle_offline_command(command, restore, events),
        None => return false,
      },
    }
  }
}

fn drain_offline_commands(
  commands: &mut mpsc::UnboundedReceiver<MpdCommand>,
  restore: &mut RestoreState,
  events: &mpsc::UnboundedSender<AsyncEvent>,
) {
  while let Ok(command) = commands.try_recv() {
    handle_offline_command(command, restore, events);
  }
}

fn handle_offline_command(
  command: MpdCommand,
  restore: &mut RestoreState,
  events: &mpsc::UnboundedSender<AsyncEvent>,
) {
  match command {
    MpdCommand::ArmInterrupt(session) => restore.arm(session),
    command => {
      debug!(?command, "dropping command while mpd is offline");
      let _ = events.send(AsyncEvent::Mpd(MpdEvent::Notice(
        "mpd is not connected".to_string(),
      )));
    }
  }
}

pub async fn connect(
  config: &MpdConfig,
) -> anyhow::Result<(Client, mpd_client::client::ConnectionEvents)> {
  tokio::time::timeout(CONNECT_TIMEOUT, connect_inner(config))
    .await
    .map_err(|_| anyhow::anyhow!("timed out after {}s", CONNECT_TIMEOUT.as_secs()))?
}

async fn connect_inner(
  config: &MpdConfig,
) -> anyhow::Result<(Client, mpd_client::client::ConnectionEvents)> {
  let host = crate::config::expand_home(&config.host);

  #[cfg(unix)]
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
  #[cfg(unix)]
  if crate::library::is_socket_host(&config.host) {
    return config.host.clone();
  }
  format!("{}:{}", config.host, config.port)
}
