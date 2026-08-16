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

  fn song_matches_filter(song: &Song, terms: &[String]) -> bool {
    // Every space-separated term must match somewhere (AND); field text
    // matches with spaces ignored ("Love Story" ~ "lovestory").
    terms.iter().all(|term| {
      song_title(song).is_some_and(|value| StrippedText::new(value).matches(term))
        || song_artist(song).is_some_and(|value| StrippedText::new(value).matches(term))
        || song_album(song).is_some_and(|value| StrippedText::new(value).matches(term))
        || StrippedText::new(&song.url).matches(term)
    })
  }

  pub(crate) fn recompute_queue_filter(&mut self) {
    self.queue_filter_matches = match self.queue_filter.as_deref() {
      None | Some("") => (0..self.queue.len()).collect(),
      Some(needle) => {
        let terms: Vec<String> = needle.split_whitespace().map(str::to_string).collect();
        self
          .queue
          .iter()
          .enumerate()
          .filter(|(_, song)| Self::song_matches_filter(&song.song, &terms))
          .map(|(position, _)| position)
          .collect()
      }
    };
    if self.queue_dedup {
      let urls: Vec<&str> = self.queue.iter().map(|song| song.song.url.as_str()).collect();
      let playing = self
        .status
        .as_ref()
        .and_then(|status| status.current_song)
        .map(|(position, _)| position.0);
      self.queue_filter_matches = {
        let positions = std::mem::take(&mut self.queue_filter_matches);
        dedup_positions(&urls, positions, playing)
      };
    }
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

/// Keep the first occurrence of each URL; a playing copy of a duplicate
/// stays visible so the ▶ marker follows the actual playback position.
fn dedup_positions(urls: &[&str], positions: Vec<usize>, playing: Option<usize>) -> Vec<usize> {
  let mut seen = std::collections::HashSet::new();
  let mut out = Vec::with_capacity(positions.len());
  for position in positions {
    if playing == Some(position) || seen.insert(urls[position]) {
      out.push(position);
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn dedup_keeps_first_occurrence_of_each_url() {
    let urls = ["a", "b", "a", "c", "b"];
    let positions = vec![0, 1, 2, 3, 4];
    assert_eq!(dedup_positions(&urls, positions, None), vec![0, 1, 3]);
  }

  #[test]
  fn dedup_keeps_the_playing_copy_visible() {
    let urls = ["a", "a"];
    let positions = vec![0, 1];
    assert_eq!(dedup_positions(&urls, positions, Some(1)), vec![0, 1]);
  }

  #[test]
  fn dedup_applies_after_text_filtering() {
    let urls = ["a", "x", "a"];
    // positions pre-filtered to [0, 2]
    assert_eq!(dedup_positions(&urls, vec![0, 2], None), vec![0]);
  }
}
