//! Mouse handling: queue/lyrics viewport scrolling, tab and band hit tests.

use super::*;

impl App {
  pub(crate) fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
    if self.show_help {
      match mouse.kind {
        MouseEventKind::Down(_) => {
          self.show_help = false;
          true
        }
        MouseEventKind::ScrollUp => self.scroll_help(-3),
        MouseEventKind::ScrollDown => self.scroll_help(3),
        _ => false,
      }
    } else {
      self.handle_mouse_on_interface(mouse)
    }
  }

  fn handle_mouse_on_interface(&mut self, mouse: MouseEvent) -> bool {
    // Clicking a tab label in the tab bar switches to that tab.
    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
      && let Some(index) = self
        .tab_hit_areas
        .iter()
        .position(|area| {
          mouse.row == area.y
            && mouse.column >= area.x
            && mouse.column < area.x + area.width
        })
      && index != self.tab
    {
      self.goto_tab(index);
      return true;
    }
    // Clicking a synced lyric line seeks to its timestamp (only when the
    // click lands inside a lyrics pane, both rows and columns).
    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
      && let Some(index) = self.lyrics_index_at(mouse)
    {
      let _ = self.lyrics_seek_to(index);
      return true;
    }
    match mouse.kind {
      MouseEventKind::Down(MouseButton::Left) => {
        if let Some(row) = self.queue_row_index(mouse) {
          // Click selects; clicking the already-selected row (or a quick
          // second click) plays it — double-click without the timer.
          let selected = self.queue_state.selected();
          if selected == Some(row) {
            self.play_selected_queue_row();
          } else {
            self.select_queue_row(row);
          }
          return true;
        }
        self.band_scrubbing = self.mouse_on_band(mouse);
        if self.band_scrubbing {
          return self.seek_to_band_column(mouse.column);
        }
        false
      }
      MouseEventKind::Drag(MouseButton::Left) => {
        if self.band_scrubbing {
          return self.seek_to_band_column(mouse.column);
        }
        false
      }
      MouseEventKind::Up(MouseButton::Left) => {
        let was_scrubbing = self.band_scrubbing;
        self.band_scrubbing = false;
        was_scrubbing
      }
      MouseEventKind::ScrollUp => {
        if self.mouse_on_band(mouse) {
          self.mpdc(MpdCommand::NudgeSeek(-5));
          true
        } else if self.mouse_on_queue(mouse).is_some() {
          self.scroll_queue_viewport(-3)
        } else if self.mouse_on_lyrics(mouse).is_some() {
          self.scroll_lyrics_viewport(-3)
        } else {
          false
        }
      }
      MouseEventKind::ScrollDown => {
        if self.mouse_on_band(mouse) {
          self.mpdc(MpdCommand::NudgeSeek(5));
          true
        } else if self.mouse_on_queue(mouse).is_some() {
          self.scroll_queue_viewport(3)
        } else if self.mouse_on_lyrics(mouse).is_some() {
          self.scroll_lyrics_viewport(3)
        } else {
          false
        }
      }
      MouseEventKind::Down(MouseButton::Middle) => {
        if let Some(row) = self.queue_row_index(mouse) {
          self.select_queue_row(row);
          self.play_selected_queue_row();
          return true;
        }
        false
      }
      _ => false,
    }
  }

  fn mouse_on_lyrics(&self, mouse: MouseEvent) -> Option<Rect> {
    self.lyrics_pane_areas.iter().copied().find(|area| {
      mouse.row >= area.y
        && mouse.row < area.y + area.height
        && mouse.column >= area.x
        && mouse.column < area.x + area.width
    })
  }

  /// Map a mouse position to the visible lyric line under it (rows and
  /// columns must both be inside a lyrics pane).
  fn lyrics_index_at(&self, mouse: MouseEvent) -> Option<usize> {
    let area = self.mouse_on_lyrics(mouse)?;
    Some((mouse.row - area.y) as usize + self.lyrics_scroll)
  }

  /// Scroll the queue by moving the viewport and letting the selection
  /// follow just enough to stay inside the visible window — the mouse
  /// scrolling convention the user asked for.
  fn scroll_queue_viewport(&mut self, delta: i32) -> bool {
    let len = self.visible_len();
    if len == 0 {
      return false;
    }
    let height = self
      .queue_pane_areas
      .first()
      .map(|area| area.height as usize)
      .filter(|height| *height > 0)
      .unwrap_or(1);
    let offset = self.queue_state.offset() as i32;
    let max_offset = len.saturating_sub(height) as i32;
    let next = (offset + delta).clamp(0, max_offset.max(0)) as usize;
    if next == self.queue_state.offset() {
      return false;
    }
    // Selection follows the viewport: clamp it into the new window.
    let selected = self.queue_state.selected().unwrap_or(next);
    let selected = selected.clamp(next, (next + height - 1).min(len - 1));
    let mut state = ratatui::widgets::ListState::default();
    state.select(Some(selected));
    self.queue_state = state.with_offset(next);
    true
  }

  /// Wheel-scroll over lyrics: leaves follow mode and pans the text window.
  /// Max inner height of the lyrics panes in the current tab (viewport
  /// height for scroll clamping), recorded at draw time.
  pub fn lyrics_view_height(&self) -> u16 {
    self
      .lyrics_pane_areas
      .iter()
      .map(|area| area.height)
      .max()
      .unwrap_or(0)
  }

  /// Scroll the lyrics viewport (wheel): the offset moves, the pointer
  /// passively follows and is clamped back inside the new viewport — same
  /// semantics as the queue view.
  fn scroll_lyrics_viewport(&mut self, delta: i32) -> bool {
    self.lyrics_follow = false;
    let line_count = self
      .lyrics
      .as_ref()
      .map(Lyrics::line_count)
      .unwrap_or(0);
    if line_count == 0 {
      return false;
    }
    let height = usize::from(self.lyrics_view_height().max(1));
    let max_scroll = line_count.saturating_sub(height);
    let base = self.lyrics_scroll;
    let next = if delta < 0 {
      base.saturating_sub(delta.unsigned_abs() as usize)
    } else {
      base.saturating_add(delta.unsigned_abs() as usize)
    }
    .min(max_scroll);
    self.lyrics_scroll = next;

    // The pointer only moves as much as needed to stay inside the viewport.
    let pointer = self
      .lyrics_cursor
      .unwrap_or_else(|| self.active_lyrics_index().unwrap_or(0));
    let last_visible = (next + height).saturating_sub(1).min(line_count.saturating_sub(1));
    self.lyrics_cursor = Some(pointer.clamp(next, last_visible));
    true
  }

  /// Scroll the f1 help dialog, clamped to the range computed at draw time.
  fn scroll_help(&mut self, delta: i32) -> bool {
    if self.max_help_scroll == 0 {
      return false;
    }
    let next = if delta < 0 {
      self.help_scroll.saturating_sub(delta.unsigned_abs() as usize)
    } else {
      self.help_scroll.saturating_add(delta as usize)
    };
    let next = next.min(self.max_help_scroll);
    if next == self.help_scroll {
      return false;
    }
    self.help_scroll = next;
    true
  }

  fn mouse_on_band(&self, mouse: MouseEvent) -> bool {
    self.progress_band_area.is_some_and(|area| {
      mouse.row == area.y && mouse.column >= area.x && mouse.column < area.x + area.width
    })
  }

  fn mouse_on_queue(&self, mouse: MouseEvent) -> Option<Rect> {
    self.queue_pane_areas.iter().copied().find(|area| {
      mouse.row >= area.y
        && mouse.row < area.y + area.height
        && mouse.column >= area.x
        && mouse.column < area.x + area.width
    })
  }

  /// Map a screen position to the visible queue row under it.
  fn queue_row_index(&self, mouse: MouseEvent) -> Option<usize> {
    let area = self.mouse_on_queue(mouse)?;
    let row = (mouse.row - area.y) as usize + self.queue_state.offset();
    (row < self.visible_len()).then_some(row)
  }

  pub(crate) fn select_queue_row(&mut self, row: usize) {
    self.queue_state.select(Some(row.min(self.visible_len().saturating_sub(1))));
  }

  fn play_selected_queue_row(&mut self) {
    if let Some(position) = self
      .queue_state
      .selected()
      .and_then(|row| self.filtered_position(row))
    {
      self.mpdc(MpdCommand::PlayPosition(position as u32));
    }
  }

  /// Seek to a synced lyric line and return whether anything happened.
  pub(crate) fn lyrics_seek_to(&mut self, index: usize) -> bool {
    let Some(Lyrics::Synced(lines)) = self.lyrics.as_ref() else {
      return false;
    };
    let Some(line) = lines.get(index) else {
      return false;
    };
    self.mpdc(MpdCommand::SeekCurrent(line.time_secs.max(0.0)));
    self.lyrics_follow = true;
    self.lyrics_cursor = None;
    self.set_message(format!("seek to {}", format_time(line.time_secs)));
    true
  }

    fn seek_to_band_column(&mut self, column: u16) -> bool {
    let Some(area) = self.progress_band_area else {
      return false;
    };
    let Some(duration) = self.duration().filter(|duration| *duration > 0.0) else {
      return false;
    };
    let ratio = (f64::from(column.saturating_sub(area.x)) + 0.5) / f64::from(area.width);
    let position = (ratio.clamp(0.0, 1.0) * duration).max(0.0);
    self.mpdc(MpdCommand::SeekCurrent(position));
    true
  }

}
