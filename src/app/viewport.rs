//! Shared viewport math for stateful panes (the queue's `ListState`,
//! the library's `TableState`): wheel scrolling moves the viewport while
//! the selection passively follows, scrollbar drags jump it, and mouse
//! hit tests locate panes and rows.

use super::*;
use ratatui::widgets::{ListState, TableState};

/// A ratatui state object driving a scrollable pane.
pub(crate) trait PaneState {
  fn pane_offset(&self) -> usize;
  fn pane_selected(&self) -> Option<usize>;
  /// Reinstall offset + selected (ratatui only exposes builders).
  fn install(&mut self, offset: usize, selected: usize);
}

impl PaneState for ListState {
  fn pane_offset(&self) -> usize {
    self.offset()
  }

  fn pane_selected(&self) -> Option<usize> {
    self.selected()
  }

  fn install(&mut self, offset: usize, selected: usize) {
    let mut state = ListState::default();
    state.select(Some(selected));
    *self = state.with_offset(offset);
  }
}

impl PaneState for TableState {
  fn pane_offset(&self) -> usize {
    self.offset()
  }

  fn pane_selected(&self) -> Option<usize> {
    self.selected()
  }

  fn install(&mut self, offset: usize, selected: usize) {
    let mut state = TableState::default();
    state.select(Some(selected));
    *self = state.with_offset(offset);
  }
}

/// Viewport height of the first recorded pane area (>= 1).
pub(crate) fn viewport_height(areas: &[Rect]) -> usize {
  areas
    .first()
    .map(|area| area.height as usize)
    .filter(|height| *height > 0)
    .unwrap_or(1)
}

/// Clamp a pane's `(selected, offset)` pair after its row list changed
/// length (filtering, refresh).
///
/// When the selection falls outside the new list (a filter shrank it),
/// land on the best row — the top of the list — instead of pinning the
/// selection to the last row. Returns `None` for an empty list.
pub(crate) fn clamp_selection(
  selected: Option<usize>,
  offset: usize,
  len: usize,
  height: usize,
) -> Option<(usize, usize)> {
  if len == 0 {
    return None;
  }
  let height = height.max(1);
  let offset = offset.min(len.saturating_sub(height));
  let selected = match selected {
    Some(selected) if selected < len => selected.clamp(offset, (offset + height - 1).min(len - 1)),
    // No selection yet, or it fell off the shortened list.
    _ => return Some((0, 0)),
  };
  Some((selected, offset))
}

/// Set the viewport offset absolutely, clamping the selection into the
/// new window (the selection follows the viewport).
pub(crate) fn set_viewport<S: PaneState>(
  state: &mut S,
  len: usize,
  height: usize,
  next: usize,
) -> bool {
  if len == 0 {
    return false;
  }
  let max_offset = len.saturating_sub(height);
  let next = next.min(max_offset);
  if next == state.pane_offset() {
    return false;
  }
  let selected = state.pane_selected().unwrap_or(next);
  let selected = selected.clamp(next, (next + height - 1).min(len - 1));
  state.install(next, selected);
  true
}

/// Wheel scroll: move the viewport by `delta` rows.
pub(crate) fn scroll_viewport<S: PaneState>(
  state: &mut S,
  len: usize,
  height: usize,
  delta: i32,
) -> bool {
  if len == 0 {
    return false;
  }
  let next =
    ((state.pane_offset() as i32) + delta).clamp(0, len.saturating_sub(height) as i32) as usize;
  set_viewport(state, len, height, next)
}

/// Whether the pointer lies inside `area` (row AND column).
pub(crate) fn contains(area: Rect, mouse: MouseEvent) -> bool {
  area.contains(ratatui::layout::Position::new(mouse.column, mouse.row))
}

/// First pane area containing the pointer.
pub(crate) fn hit_pane(areas: &[Rect], mouse: MouseEvent) -> Option<Rect> {
  areas.iter().copied().find(|area| contains(*area, mouse))
}

/// Index of the first area containing the pointer.
pub(crate) fn hit_index(areas: &[Rect], mouse: MouseEvent) -> Option<usize> {
  areas.iter().position(|area| contains(*area, mouse))
}

/// Map a pointer on a pane to the data row under it.
pub(crate) fn row_at(area: Rect, mouse: MouseEvent, offset: usize, len: usize) -> Option<usize> {
  let row = (mouse.row - area.y) as usize + offset;
  (row < len).then_some(row)
}

/// Map a scrollbar click/drag to a viewport offset: the thumb center
/// follows the pointer, proportionally over the whole content.
pub(crate) fn bar_jump<S: PaneState>(
  state: &mut S,
  len: usize,
  height: usize,
  track: Rect,
  row: u16,
) -> bool {
  if len == 0 {
    return false;
  }
  let track_span = track.height.saturating_sub(1) as f64;
  let ratio = if track_span <= 0.0 {
    0.0
  } else {
    (row.saturating_sub(track.y) as f64 / track_span).clamp(0.0, 1.0)
  };
  let target_center = ratio * (len.saturating_sub(1)) as f64;
  let next = (target_center - height as f64 / 2.0)
    .round()
    .clamp(0.0, len.saturating_sub(height) as f64) as usize;
  set_viewport(state, len, height, next)
}
