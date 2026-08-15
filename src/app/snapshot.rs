//! MPD event application: connection state, notices and queue snapshots
//! (song-change detection, filter recompute, selection clamping).

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
        self.connection_error = Some(reason);
        self.status = None;
        true
      }
      MpdEvent::Notice(notice) => {
        self.set_message(notice);
        true
      }
      MpdEvent::Snapshot { status, queue } => {
        let song_changed = Self::snapshot_song_url(&status, &queue).as_deref()
          != self.current_song_url().as_deref();
        self.status = Some(status);
        self.queue = queue;
        self.recompute_queue_filter();
        self.clamp_queue_selection();
        if let Some(position) = self
          .pending_restore_selection
          .take()
          .filter(|position| *position < self.queue.len())
          && self.queue_state.selected().is_none_or(|current| current == 0)
        {
          self.queue_state.select(Some(position));
        }
        if song_changed {
          self.on_song_changed();
        }
        self.sync_hover_view();
        true
      }
    }
  }

  fn snapshot_song_url(status: &Status, queue: &[SongInQueue]) -> Option<String> {
    let (position, _) = status.current_song?;
    queue.get(position.0).map(|song| song.song.url.to_string())
  }

  pub(crate) fn clamp_queue_selection(&mut self) {
    if self.queue_filter_matches.is_empty() {
      self.queue_state.select(None);
      return;
    }
    let len = self.queue_filter_matches.len();
    let current = self.queue_state.selected().unwrap_or(0).min(len - 1);
    self.queue_state.select(Some(current));
  }

  /// Number of rows visible in the queue pane (filtered or not).
  pub(crate) fn visible_len(&self) -> usize {
    self.queue_filter_matches.len()
  }

  /// Map the selection (an index into the visible rows) to a queue position.
  pub(crate) fn filtered_position(&self, selected: usize) -> Option<usize> {
    self.queue_filter_matches.get(selected).copied()
  }

  fn song_matches_filter(song: &Song, needle: &str) -> bool {
    let needle = needle.to_lowercase();
    if song.title().is_some_and(|title| title.to_lowercase().contains(&needle)) {
      return true;
    }
    if song
      .artists()
      .iter()
      .any(|artist| artist.to_lowercase().contains(&needle))
    {
      return true;
    }
    if song
      .album()
      .is_some_and(|album| album.to_lowercase().contains(&needle))
    {
      return true;
    }
    song.url.to_lowercase().contains(&needle)
  }

  pub(crate) fn recompute_queue_filter(&mut self) {
    self.queue_filter_matches = match self.queue_filter.as_deref() {
      None | Some("") => (0..self.queue.len()).collect(),
      Some(needle) => self
        .queue
        .iter()
        .enumerate()
        .filter(|(_, song)| Self::song_matches_filter(&song.song, needle))
        .map(|(position, _)| position)
        .collect(),
    };
  }

  pub(crate) fn clear_queue_filter(&mut self) {
    self.queue_filter = None;
    self.recompute_queue_filter();
    self.clamp_queue_selection();
  }

  pub(crate) fn follow_playing_position(&mut self) {
    if let Some(status) = &self.status
      && let Some((position, _)) = status.current_song
    {
      let row = self
        .queue_filter_matches
        .iter()
        .position(|candidate| *candidate == position.0)
        .or(if self.queue_filter.is_none() { Some(position.0) } else { None });
      if let Some(row) = row {
        self.queue_state.select(Some(row));
      }
    }
  }
}
