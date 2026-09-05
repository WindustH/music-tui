//! MPD worker session: connection event loop, status/queue refresh ticks,
//! command dispatch and the interrupt-session restore hook.

use super::*;
struct SessionState {
  interrupt: Option<InterruptSession>,
  /// How many times a pending interrupt restore has failed; used to bound
  /// automatic retries so a permanently missing saved playlist does not
  /// spin forever (the playlist is preserved for manual recovery).
  restore_attempts: u32,
  playing: bool,
  queue: Vec<SongInQueue>,
  /// Playlist version of the cached queue (`Status::playlist_version`);
  /// when a status poll sees a different version, the cached queue is
  /// stale (another client changed the queue) and gets re-fetched.
  playlist_version: u32,
}

pub(super) async fn run_session(
  client: &Client,
  connection_events: &mut mpd_client::client::ConnectionEvents,
  command_rx: &mut mpsc::UnboundedReceiver<MpdCommand>,
  events: &mpsc::UnboundedSender<AsyncEvent>,
  config: &MpdConfig,
  dedup: &Arc<AtomicBool>,
) -> anyhow::Result<()> {
  let mut state = SessionState {
    interrupt: None,
    restore_attempts: 0,
    playing: false,
    queue: Vec::new(),
    playlist_version: 0,
  };

  let status = client.command(commands::Status).await?;
  state.playing = status.state == PlayState::Playing;
  refresh(client, &mut state, events).await?;

  let tick_idle = Duration::from_millis(1000);
  let tick_playing = Duration::from_millis(250);

  loop {
    let period = if state.playing {
      tick_playing
    } else {
      tick_idle
    };
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
            let dedup = dedup.load(std::sync::atomic::Ordering::Relaxed);
            run_command(client, command, config, &state.queue, dedup).await;
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
      | MpdCommand::Shuffle
      | MpdCommand::DeleteAt(_)
      | MpdCommand::AddUri(_)
      | MpdCommand::PlayLibrary { .. }
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
  state.playlist_version = status.playlist_version;
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
  if status.playlist_version != state.playlist_version {
    // The queue changed since the last full refresh (another client or
    // instance): re-fetch it instead of pairing this fresh status with
    // a stale queue snapshot.
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
///
/// Restores are retried (a bounded number of times) instead of being
/// abandoned on failure: the saved playlist is the only copy of the
/// original queue, so it is only deleted once the restore has succeeded.
/// On persistent failure the playlist is left intact for manual recovery.
async fn maybe_restore_interrupt(
  client: &Client,
  state: &mut SessionState,
  status: &Status,
  events: &mpsc::UnboundedSender<AsyncEvent>,
) -> anyhow::Result<bool> {
  if status.state != PlayState::Stopped {
    return Ok(false);
  }
  let Some(session) = state.interrupt.as_ref().cloned() else {
    return Ok(false);
  };
  info!("interrupt preview finished; restoring previous queue");

  client.command(ClearQueue).await?;

  if let Some(playlist) = &session.playlist {
    if let Err(error) = client.command(LoadPlaylist::name(playlist)).await {
      state.restore_attempts += 1;
      warn!(
        %error, playlist = %playlist, attempts = state.restore_attempts,
        "failed to restore saved playlist"
      );
      if state.restore_attempts < 3 {
        let _ = events.send(AsyncEvent::Mpd(MpdEvent::Notice(format!(
          "restore failed ({error}); will retry"
        ))));
        // Keep the session armed so a later refresh retries; the saved
        // playlist (the only copy of the original queue) is untouched.
        return Ok(false);
      }
      let _ = events.send(AsyncEvent::Mpd(MpdEvent::Notice(format!(
        "could not restore queue from saved playlist `{playlist}`; \
         it has been left in place so you can load it manually"
      ))));
      state.interrupt = None;
      state.restore_attempts = 0;
      return Ok(true);
    }
    // The original queue is now safely back in the live queue, so the
    // restore is committed: disarm before any later (cosmetic) step so a
    // failure there cannot re-clear the restored queue on a retry.
    state.interrupt = None;
    state.restore_attempts = 0;
    let _ = client.command(DeletePlaylist(playlist.as_str())).await;
  } else {
    state.interrupt = None;
    state.restore_attempts = 0;
    let _ = client.command(SetSingle(session.single)).await;
    let _ = events.send(AsyncEvent::Mpd(MpdEvent::Notice(
      "preview finished; restored previous queue".to_string(),
    )));
    return Ok(true);
  }

  client.command(SetSingle(session.single)).await?;
  if let Some(position) = session.position {
    let queue_len = client.command(commands::Queue).await?.len();
    if (position as usize) < queue_len {
      client
        .command(Play::song(SongPosition(position as usize)))
        .await?;
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

async fn run_command(
  client: &Client,
  command: MpdCommand,
  config: &MpdConfig,
  queue: &[SongInQueue],
  dedup: bool,
) {
  let outcome: Result<(), String> = match command {
    MpdCommand::PlayPosition(position) => client
      .command(Play::song(SongPosition(position as usize)))
      .await
      .map(|_| ())
      .map_err(|error| error.to_string()),
    MpdCommand::PlayPauseToggle => {
      let state = client.command(commands::Status).await.map(|s| s.state);
      match state {
        Ok(PlayState::Playing) => client
          .command(SetPause(true))
          .await
          .map(|_| ())
          .map_err(|error| error.to_string()),
        _ => client
          .command(Play::current())
          .await
          .map(|_| ())
          .map_err(|error| error.to_string()),
      }
    }
    MpdCommand::Pause(pause) => client
      .command(SetPause(pause))
      .await
      .map(|_| ())
      .map_err(|error| error.to_string()),
    MpdCommand::Stop => client
      .command(Stop)
      .await
      .map(|_| ())
      .map_err(|error| error.to_string()),
    MpdCommand::Next => client
      .command(commands::Next)
      .await
      .map(|_| ())
      .map_err(|error| error.to_string()),
    MpdCommand::Previous => client
      .command(Previous)
      .await
      .map(|_| ())
      .map_err(|error| error.to_string()),
    MpdCommand::SetVolume(volume) => client
      .command(SetVolume(volume))
      .await
      .map(|_| ())
      .map_err(|error| error.to_string()),
    MpdCommand::NudgeVolume(delta) => {
      let current = client.command(commands::Status).await.map(|s| s.volume);
      match current {
        Ok(current) => {
          let next = (i32::from(current) + i32::from(delta)).clamp(0, 100) as u8;
          client
            .command(SetVolume(next))
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
        }
        Err(error) => Err(error.to_string()),
      }
    }
    MpdCommand::SeekCurrent(seconds) => client
      .command(Seek(SeekMode::Absolute(Duration::from_secs_f64(
        if seconds.is_finite() {
          seconds.clamp(0.0, 24.0 * 60.0 * 60.0)
        } else {
          0.0
        },
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
    MpdCommand::SetRepeat(repeat) => client
      .command(SetRepeat(repeat))
      .await
      .map(|_| ())
      .map_err(|error| error.to_string()),
    MpdCommand::SetRandom(random) => client
      .command(SetRandom(random))
      .await
      .map(|_| ())
      .map_err(|error| error.to_string()),
    MpdCommand::SetSingle(mode) => client
      .command(SetSingle(mode))
      .await
      .map(|_| ())
      .map_err(|error| error.to_string()),
    MpdCommand::SetConsume(consume) => client
      .command(SetConsume(consume))
      .await
      .map(|_| ())
      .map_err(|error| error.to_string()),
    MpdCommand::ClearQueue => client
      .command(ClearQueue)
      .await
      .map(|_| ())
      .map_err(|error| error.to_string()),
    MpdCommand::Shuffle => client
      .command(Shuffle::all())
      .await
      .map(|_| ())
      .map_err(|error| error.to_string()),
    MpdCommand::DeleteAt(position) => client
      .command(Delete::range(
        SongPosition(position)..SongPosition(position + 1),
      ))
      .await
      .map(|_| ())
      .map_err(|error| error.to_string()),
    MpdCommand::AddUri(uri) => {
      if dedup
        && queue
          .iter()
          .any(|song| crate::library::same_song_uri(&song.song.url, &uri))
      {
        debug!(uri = %uri, "skip add: already queued (dedup on)");
        Ok::<(), String>(())
      } else {
        client
          .command(Add::uri(&uri))
          .await
          .map(|_| ())
          .map_err(|error| error.to_string())
      }
    }
    MpdCommand::PlayLibrary { path, append } => {
      play_library_file(client, path, append, config, queue, dedup).await
    }
    MpdCommand::Rescan => client
      .command(commands::Rescan::new())
      .await
      .map(|_| ())
      .map_err(|error| error.to_string()),
    MpdCommand::UpdateUri(uri) => client
      .command(commands::Update::new().uri(&uri))
      .await
      .map(|_| ())
      .map_err(|error| error.to_string()),
    MpdCommand::ArmInterrupt(_) => Ok(()),
  };
  if let Err(error) = outcome {
    warn!(%error, "mpd command failed");
  }
}

/// Resolve a library-pane file to an MPD uri and insert it into the queue:
/// `append` adds to the end (starting playback when idle), otherwise the
/// track is inserted right after the current song (or at the end when
/// nothing plays) and starts immediately. With dedup on, a song that is
/// already queued is never re-added — playback simply jumps to the
/// existing entry.
async fn play_library_file(
  client: &Client,
  path: std::path::PathBuf,
  append: bool,
  config: &MpdConfig,
  queue: &[SongInQueue],
  dedup: bool,
) -> Result<(), String> {
  let music_dir = crate::library::resolve_music_dir(config).ok();
  let uri = crate::open::resolve_open_uri(client, &path, config, music_dir.as_deref())
    .await
    .map_err(|error| error.to_string())?;

  if dedup
    && let Some(position) = queue
      .iter()
      .position(|song| crate::library::same_song_uri(&song.song.url, &uri))
  {
    // Already queued: skip the add and reuse the existing entry.
    if append {
      return crate::open::maybe_start_if_idle(client)
        .await
        .map_err(|error| error.to_string());
    }
    return client
      .command(Play::song(SongPosition(position)))
      .await
      .map(|_| ())
      .map_err(|error| error.to_string());
  }

  if append {
    client
      .command(Add::uri(&uri))
      .await
      .map_err(|error| error.to_string())?;
    crate::open::maybe_start_if_idle(client)
      .await
      .map_err(|error| error.to_string())?;
    return Ok(());
  }

  let status = client
    .command(commands::Status)
    .await
    .map_err(|error| error.to_string())?;
  let add = match status.current_song {
    Some((_, _)) => Add::uri(&uri).after_current(0),
    None => Add::uri(&uri),
  };
  let id = client
    .command(add)
    .await
    .map_err(|error| error.to_string())?;
  client
    .command(Play::song(id))
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}
