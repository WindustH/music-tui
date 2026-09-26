//! "Interrupt" preview sessions: snapshot the current queue into a stored
//! playlist, play the preview song, then restore the previous queue and
//! playback state when it finishes.

use std::time::Duration;

use mpd_client::{
  Client,
  commands::{
    self, ClearQueue, DeletePlaylist, LoadPlaylist, Play, SaveQueueAsPlaylist, Seek, SeekMode,
    SetPause, SetSingle, SongPosition,
  },
  responses::{PlayState, Status},
};
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::InterruptSession;
use crate::event::{AsyncEvent, MpdEvent};

/// How many failed restores are retried before the saved playlist is left
/// in place for manual recovery.
const MAX_RESTORE_ATTEMPTS: u32 = 3;

/// Best-effort helper used by `open --mode interrupt` to snapshot state.
pub async fn capture_interrupt_session(client: &Client) -> anyhow::Result<InterruptSession> {
  let status = client.command(commands::Status).await?;
  let playlist = if status.playlist_length > 0 {
    let stamp = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs();
    let name = format!("music-tui-preview-{stamp}");
    client.command(SaveQueueAsPlaylist(name.as_str())).await?;
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

/// The armed interrupt session (if any) plus its retry bookkeeping. Lives
/// in the worker across reconnects.
#[derive(Default)]
pub(super) struct RestoreState {
  session: Option<InterruptSession>,
  /// How many times a pending restore has failed; bounds automatic
  /// retries so a permanently missing saved playlist does not spin forever
  /// (the playlist is preserved for manual recovery).
  attempts: u32,
}

impl RestoreState {
  pub(super) fn arm(&mut self, session: InterruptSession) {
    self.session = Some(session);
    self.attempts = 0;
  }

  /// The user took over the queue: the preview no longer owns it.
  pub(super) fn disarm(&mut self) {
    self.session = None;
    self.attempts = 0;
  }

  /// A restore step failed. Keeps the session armed for a later retry (a
  /// failed command may well mean the connection dropped; the next session
  /// retries), up to [`MAX_RESTORE_ATTEMPTS`]; then gives up, leaving the
  /// saved playlist for manual recovery. Returns whether it gave up.
  fn record_failure(
    &mut self,
    error: &str,
    playlist: Option<&str>,
    events: &mpsc::UnboundedSender<AsyncEvent>,
  ) -> bool {
    self.attempts += 1;
    warn!(%error, ?playlist, attempts = self.attempts, "failed to restore the previous queue");
    let error = crate::sanitize::sanitize_text(error);
    let notice = if self.attempts < MAX_RESTORE_ATTEMPTS {
      format!("restore failed ({error}); will retry")
    } else {
      self.disarm();
      match playlist {
        Some(playlist) => format!(
          "could not restore queue from saved playlist `{}`; \
           it has been left in place so you can load it manually",
          crate::sanitize::sanitize_text(playlist)
        ),
        None => format!("could not restore the previous queue ({error})"),
      }
    };
    let _ = events.send(AsyncEvent::Mpd(MpdEvent::Notice(notice)));
    self.session.is_none()
  }

  /// After an interrupt preview stops, rebuild the previous queue and
  /// state. Returns true when a restore was performed (or given up on).
  ///
  /// Restores are retried (a bounded number of times) instead of being
  /// abandoned on failure: the saved playlist is the only copy of the
  /// original queue, so it is only deleted once the restore has succeeded.
  /// On persistent failure the playlist is left intact for manual recovery.
  pub(super) async fn maybe_restore(
    &mut self,
    client: &Client,
    status: &Status,
    events: &mpsc::UnboundedSender<AsyncEvent>,
  ) -> anyhow::Result<bool> {
    if status.state != PlayState::Stopped {
      return Ok(false);
    }
    let Some(session) = self.session.clone() else {
      return Ok(false);
    };
    info!("interrupt preview finished; restoring previous queue");
    let notice = |text: String| {
      let _ = events.send(AsyncEvent::Mpd(MpdEvent::Notice(text)));
    };

    let Some(playlist) = &session.playlist else {
      // The previous queue was empty: clearing the preview restores it.
      if let Err(error) = client.command(ClearQueue).await {
        return Ok(self.record_failure(&error.to_string(), None, events));
      }
      self.disarm();
      let _ = client.command(SetSingle(session.single)).await;
      notice("preview finished; restored previous queue".to_string());
      return Ok(true);
    };
    let loaded = match client.command(ClearQueue).await {
      Ok(_) => client.command(LoadPlaylist::name(playlist)).await.map(drop),
      Err(error) => Err(error),
    };
    if let Err(error) = loaded {
      return Ok(self.record_failure(&error.to_string(), Some(playlist), events));
    }
    // The original queue is now safely back in the live queue, so the
    // restore is committed: disarm before any later (cosmetic) step so a
    // failure there cannot re-clear the restored queue on a retry.
    self.disarm();
    let _ = client.command(DeletePlaylist(playlist.as_str())).await;

    client.command(SetSingle(session.single)).await?;
    if let Some(position) = session.position {
      let queue_len = client.command(commands::Status).await?.playlist_length;
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
    notice("preview finished; restored previous queue".to_string());
    Ok(true)
  }
}
