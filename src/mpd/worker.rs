//! MPD worker session: connection event loop, status/queue refresh ticks,
//! command dispatch and the interrupt-session restore hook.

use super::*;
struct SessionState {
  interrupt: Option<InterruptSession>,
  playing: bool,
  queue: Vec<SongInQueue>,
}

pub(super) async fn run_session(
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

