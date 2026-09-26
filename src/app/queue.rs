//! Queue pane state: the visible-row mapping (filter + dedup), selection,
//! viewport scrolling, and row actions (play / delete / jump to playing).

use super::*;
use crate::strip::{matches_needle, needle};

impl App {
  /// Number of rows visible in the queue pane (filtered or not).
  pub(crate) fn visible_len(&self) -> usize {
    self.queue_filter_matches.len()
  }

  /// Map the selection (an index into the visible rows) to a queue position.
  pub(crate) fn filtered_position(&self, selected: usize) -> Option<usize> {
    self.queue_filter_matches.get(selected).copied()
  }

  /// Queue position of the playing song, if any.
  pub(crate) fn playing_position(&self) -> Option<usize> {
    self
      .status
      .as_ref()
      .and_then(|status| status.current_song)
      .map(|(position, _)| position.0)
  }

  /// The queue entry under the selection.
  pub(crate) fn selected_queue_song(&self) -> Option<&SongInQueue> {
    let row = self.queue_state.selected()?;
    self.queue.get(self.filtered_position(row)?)
  }

  pub(crate) fn recompute_queue_filter(&mut self) {
    let playing = self.playing_position();
    let positions: Vec<usize> = match self.queue_filter.as_deref() {
      None | Some("") => (0..self.queue.len()).collect(),
      Some(filter) => {
        let needles: Vec<String> = filter.split_whitespace().map(needle).collect();
        self
          .queue
          .iter()
          .enumerate()
          .filter(|(_, song)| fields_match(&song_filter_fields(&song.song), &needles))
          .map(|(position, _)| position)
          .collect()
      }
    };
    let urls: Vec<&str> = self
      .queue
      .iter()
      .map(|song| song.song.url.as_str())
      .collect();
    self.queue_filter_matches = visible_positions(
      self.queue_dedup,
      self.queue_filter.as_deref(),
      &urls,
      positions,
      playing,
    );
  }

  pub(crate) fn clamp_queue_selection(&mut self) {
    // A filter shrinking the list lands the selection on the best row
    // (row 0) instead of pinning it to the last row. The queue's
    // ListState scrolls itself, so no viewport window is applied here.
    let len = self.queue_filter_matches.len();
    match viewport::clamp_selection(self.queue_state.selected(), 0, len, len) {
      Some((selected, _)) => self.queue_state.select(Some(selected)),
      None => self.queue_state.select(None),
    }
    self.sync_hover_view();
  }

  pub(crate) fn clear_queue_filter(&mut self) {
    self.queue_filter = None;
    self.recompute_queue_filter();
    self.clamp_queue_selection();
  }

  /// Select the visible row showing the playing song (auto-follow).
  pub(crate) fn follow_playing_position(&mut self) {
    let Some(position) = self.playing_position() else {
      return;
    };
    let row = self
      .queue_filter_matches
      .iter()
      .position(|candidate| *candidate == position)
      .or(self.queue_filter.is_none().then_some(position));
    if let Some(row) = row {
      self.queue_state.select(Some(row));
      self.sync_hover_view();
    }
  }

  pub(crate) fn move_selection(&mut self, delta: i32) -> bool {
    let len = self.visible_len();
    if len == 0 {
      return false;
    }
    let next =
      (self.queue_state.selected().unwrap_or(0) as i32 + delta).clamp(0, len as i32 - 1) as usize;
    self.queue_state.select(Some(next));
    self.sync_hover_view();
    true
  }

  pub(crate) fn select_queue_row(&mut self, row: usize) {
    self
      .queue_state
      .select(Some(row.min(self.visible_len().saturating_sub(1))));
    self.sync_hover_view();
  }

  /// Flip the queue viewport one full page; the selection follows
  /// passively (same model as the mouse wheel).
  pub(crate) fn queue_page(&mut self, direction: i32) -> bool {
    let height = self.queue_viewport_height() as i32;
    self.scroll_queue_viewport(direction * height.max(1))
  }

  pub(crate) fn queue_viewport_height(&self) -> usize {
    viewport::viewport_height(&self.hit.queue_panes)
  }

  /// Scroll the queue by moving the viewport; the selection follows just
  /// enough to stay inside the visible window.
  pub(crate) fn scroll_queue_viewport(&mut self, delta: i32) -> bool {
    let len = self.visible_len();
    let height = self.queue_viewport_height();
    let changed = viewport::scroll_viewport(&mut self.queue_state, len, height, delta);
    if changed {
      // The selection may have been clamped into the new window: the
      // hovered song (sidebar sources) follows it.
      self.sync_hover_view();
    }
    changed
  }

  /// Map a scrollbar click/drag to a viewport offset.
  pub(crate) fn queue_bar_jump(&mut self, mouse: MouseEvent, track: Rect) -> bool {
    let len = self.visible_len();
    let height = self.queue_viewport_height();
    let changed = viewport::bar_jump(&mut self.queue_state, len, height, track, mouse.row);
    if changed {
      self.sync_hover_view();
    }
    changed
  }

  /// Map a screen position to the visible queue row under it.
  pub(crate) fn queue_row_index(&self, mouse: MouseEvent) -> Option<usize> {
    let area = viewport::hit_pane(&self.hit.queue_panes, mouse)?;
    viewport::row_at(area, mouse, self.queue_state.offset(), self.visible_len())
  }

  pub(crate) fn play_selected_queue_row(&mut self) -> bool {
    if let Some(song) = self.selected_queue_song() {
      self.mpdc(MpdCommand::PlaySong(song.id));
    } else if self.queue_state.selected().is_some() {
      self.set_message("selection is no longer in the queue");
    }
    true
  }

  pub(crate) fn delete_selected_queue_row(&mut self) -> bool {
    if let Some(song) = self.selected_queue_song() {
      let id = song.id;
      let title =
        song_title(&song.song).unwrap_or_else(|| crate::sanitize::sanitize_text(&song.song.url));
      self.mpdc(MpdCommand::Delete(id));
      self.set_message(format!("deleted: {title}"));
    } else if self.queue_state.selected().is_some() {
      self.set_message("selection is no longer in the queue");
    }
    true
  }

  /// `g c` in the queue: jump the selection (and view) to the song that
  /// is currently playing.
  pub(crate) fn goto_playing(&mut self) -> bool {
    let Some(position) = self.playing_position() else {
      self.set_message("nothing is playing");
      return true;
    };
    match self
      .queue_filter_matches
      .iter()
      .position(|candidate| *candidate == position)
    {
      Some(row) => self.select_queue_row(row),
      None => self.set_message("the playing song is hidden by the current filter"),
    }
    true
  }
}

/// The texts a queue filter searches: title, artist, album and the URL.
fn song_filter_fields(song: &Song) -> Vec<String> {
  [song_title(song), song_artist(song), song_album(song)]
    .into_iter()
    .flatten()
    .chain(std::iter::once(song.url.clone()))
    .collect()
}

/// Every prepared term must match somewhere (AND); field text matches
/// with spaces ignored ("Love Story" ~ "lovestory").
fn fields_match(fields: &[String], needles: &[String]) -> bool {
  needles
    .iter()
    .all(|needle| fields.iter().any(|field| matches_needle(field, needle)))
}

/// Dedup hides extra copies of a song only in the unfiltered view;
/// while a filter is active every matching copy stays visible so
/// search results never lose rows mid-filter.
fn visible_positions(
  dedup: bool,
  filter: Option<&str>,
  urls: &[&str],
  positions: Vec<usize>,
  playing: Option<usize>,
) -> Vec<usize> {
  if dedup && filter.is_none_or(str::is_empty) {
    dedup_positions(urls, positions, playing)
  } else {
    positions
  }
}

/// Hide extra copies of a song (same URL): the first occurrence stays
/// visible, and so does the playing copy.
fn dedup_positions(urls: &[&str], positions: Vec<usize>, playing: Option<usize>) -> Vec<usize> {
  let mut seen = std::collections::HashSet::new();
  let mut out = Vec::with_capacity(positions.len());
  for position in positions {
    // Mark every copy as seen, the playing one included: when the first
    // copy plays, later copies must stay hidden.
    let first = seen.insert(urls[position]);
    if first || playing == Some(position) {
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
  fn dedup_hides_later_copies_while_the_first_plays() {
    let urls = ["a", "b", "a"];
    assert_eq!(dedup_positions(&urls, vec![0, 1, 2], Some(0)), vec![0, 1]);
  }

  #[test]
  fn dedup_applies_after_text_filtering() {
    let urls = ["a", "x", "a"];
    // positions pre-filtered to [0, 2]
    assert_eq!(dedup_positions(&urls, vec![0, 2], None), vec![0]);
  }

  #[test]
  fn dedup_not_applied_while_filtering() {
    let urls = ["a", "x", "a"];
    // No filter: duplicate hidden.
    assert_eq!(
      visible_positions(true, None, &urls, vec![0, 1, 2], None),
      vec![0, 1]
    );
    assert_eq!(
      visible_positions(true, Some(""), &urls, vec![0, 1, 2], None),
      vec![0, 1]
    );
    // Filter active: every matching copy stays visible.
    assert_eq!(
      visible_positions(true, Some("a"), &urls, vec![0, 2], None),
      vec![0, 2]
    );
    // Dedup off: never hidden.
    assert_eq!(
      visible_positions(false, None, &urls, vec![0, 2], None),
      vec![0, 2]
    );
  }

  #[test]
  fn filter_matches_every_term_across_fields() {
    let fields = vec![
      "Love Story".to_string(),
      "Taylor Swift".to_string(),
      "music/love-story.flac".to_string(),
    ];
    let needles = |query: &str| query.split_whitespace().map(needle).collect::<Vec<_>>();
    assert!(fields_match(&fields, &needles("taylor love")));
    assert!(fields_match(&fields, &needles("TAYLORSWIFT")));
    assert!(fields_match(&fields, &needles("lovestory")));
    assert!(!fields_match(&fields, &needles("taylor nope")));
  }
}
