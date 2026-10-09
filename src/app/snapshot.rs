//! MPD event application: connection state, notices and status/queue
//! snapshots (song-change detection, filter recompute, selection clamping).

use super::*;

impl App {
  pub fn handle_mpd_event(&mut self, event: MpdEvent) -> bool {
    match event {
      MpdEvent::Connected(address) => {
        self.connected = Some(address);
        self.connection_error = None;
        true
      }
      MpdEvent::ConnectionLost(reason) => {
        self.connected = None;
        self.connection_error = Some(crate::sanitize::sanitize_text(&reason));
        self.status = None;
        true
      }
      MpdEvent::Notice(notice) => {
        self.set_message(notice);
        true
      }
      MpdEvent::Snapshot { status, queue } => {
        self.apply_snapshot(status, queue);
        true
      }
    }
  }

  fn apply_snapshot(&mut self, status: Status, queue: Option<Arc<[SongInQueue]>>) {
    let previous_url = self.current_song_url();
    let previous_position = self.playing_position();
    // MPD's player error (an output that failed to open, say) otherwise
    // only shows as playback that never starts.
    let previous_error = self.status.as_ref().and_then(|old| old.error.as_ref());
    let new_error = status
      .error
      .as_ref()
      .filter(|error| previous_error != Some(*error))
      .map(|error| format!("MPD: {error}"));
    self.status = Some(status);
    if let Some(error) = new_error {
      self.set_message(error);
    }
    let queue_changed = queue.is_some();
    if let Some(queue) = queue {
      self.queue = queue;
    }
    // The visible rows depend on the queue and (through dedup) on which
    // copy is playing; plain progress updates leave them untouched.
    if queue_changed || self.playing_position() != previous_position {
      self.recompute_queue_filter();
      self.clamp_queue_selection();
    }
    if queue_changed
      && let Some(position) = self
        .pending_restore_selection
        .take()
        .filter(|position| *position < self.visible_len())
      && self
        .queue_state
        .selected()
        .is_none_or(|current| current == 0)
    {
      self.queue_state.select(Some(position));
      self.sync_hover_view();
    }
    if self.current_song_url() != previous_url {
      self.on_song_changed();
    }
  }
}
