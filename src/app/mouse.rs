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
      && let Some(index) = self.tab_hit_areas.iter().position(|area| {
        mouse.row == area.y && mouse.column >= area.x && mouse.column < area.x + area.width
      })
      && index != self.tab
    {
      self.goto_tab(index);
      return true;
    }
    // Clicking a synced lyric line seeks to its timestamp (only when the
    // click lands inside a lyrics pane, both rows and columns).
    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
      && let Some(area) = self.mouse_on_lyrics(mouse)
    {
      if self.hover_lyrics_pane(area) {
        // Hovered lyrics have no playback state — nothing to seek.
        self.set_message("hovered lyrics: song is not playing");
      } else if let Some(index) = self.lyrics_index_at(mouse) {
        let _ = self.lyrics_seek_to(index);
      }
      return true;
    }
    match mouse.kind {
      MouseEventKind::Down(MouseButton::Left) => {
        // Scrollbars: jump the viewport proportionally and arm dragging
        // (the thumb follows the pointer while held).
        if let Some(track) = self.mouse_on_queue_bar(mouse) {
          self.queue_bar_dragging = true;
          return self.queue_bar_jump(mouse, track);
        }
        if let Some(track) = self.mouse_on_library_bar(mouse) {
          self.library_bar_dragging = true;
          return self.library_bar_jump(mouse, track);
        }
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
        if let Some(row) = self.library_row_index(mouse) {
          let selected = self.library_state.selected();
          if selected == Some(row) {
            self.library_play_selected();
          } else {
            self.select_library_row(row);
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
        let bar_areas = if self.queue_bar_dragging {
          Some(&self.queue_bar_areas)
        } else if self.library_bar_dragging {
          Some(&self.library_bar_areas)
        } else {
          None
        };
        if let Some(areas) = bar_areas
          && let Some(track) = viewport::hit_pane(areas, mouse)
        {
          let queue = self.queue_bar_dragging;
          return if queue {
            self.queue_bar_jump(mouse, track)
          } else {
            self.library_bar_jump(mouse, track)
          };
        }
        false
      }
      MouseEventKind::Up(MouseButton::Left) => {
        let was_active =
          self.band_scrubbing || self.queue_bar_dragging || self.library_bar_dragging;
        self.band_scrubbing = false;
        self.queue_bar_dragging = false;
        self.library_bar_dragging = false;
        was_active
      }
      MouseEventKind::ScrollUp => self.handle_pane_wheel(mouse, -3),
      MouseEventKind::ScrollDown => self.handle_pane_wheel(mouse, 3),
      MouseEventKind::Down(MouseButton::Middle) => {
        if let Some(row) = self.queue_row_index(mouse) {
          self.select_queue_row(row);
          self.play_selected_queue_row();
          return true;
        }
        if let Some(row) = self.library_row_index(mouse) {
          self.select_library_row(row);
          self.library_play_selected();
          return true;
        }
        false
      }
      _ => false,
    }
  }

  /// Wheel over the interface: the seek band nudges playback; queue,
  /// library and lyrics panes scroll their viewports.
  fn handle_pane_wheel(&mut self, mouse: MouseEvent, delta: i32) -> bool {
    if self.mouse_on_band(mouse) {
      self.mpdc(MpdCommand::NudgeSeek(i64::from(delta.signum() * 5)));
      true
    } else if self.mouse_on_queue(mouse).is_some() {
      self.scroll_queue_viewport(delta)
    } else if self.mouse_on_library(mouse).is_some() {
      self.scroll_library_viewport(delta)
    } else if self.mouse_on_lyrics(mouse).is_some() {
      self.scroll_lyrics_wheel(delta)
    } else {
      false
    }
  }

  fn mouse_on_lyrics(&self, mouse: MouseEvent) -> Option<Rect> {
    viewport::hit_pane(&self.lyrics_pane_areas, mouse)
  }

  /// Whether the lyrics pane at `area` shows the hovered song (recorded at
  /// draw time together with the area).
  fn hover_lyrics_pane(&self, area: Rect) -> bool {
    self
      .lyrics_pane_sources
      .iter()
      .zip(self.lyrics_pane_areas.iter())
      .any(|(source, pane)| {
        matches!(
          source,
          PaneSource::QueueHovered | PaneSource::LibraryHovered
        ) && *pane == area
      })
  }

  /// Wheel on a lyrics pane: scroll the hovered lyrics when that pane is
  /// the hovered source, otherwise the playing lyrics.
  fn scroll_lyrics_wheel(&mut self, delta: i32) -> bool {
    let hovered_pane = self.lyrics_pane_sources.iter().any(|source| {
      matches!(
        source,
        PaneSource::QueueHovered | PaneSource::LibraryHovered
      )
    });
    if hovered_pane
      && (self
        .hover
        .as_ref()
        .is_some_and(|hover| hover.lyrics.is_some())
        || self
          .library_hover
          .as_ref()
          .is_some_and(|hover| hover.lyrics.is_some()))
    {
      self.scroll_hover_lyrics(delta);
      true
    } else {
      self.scroll_lyrics_viewport(delta)
    }
  }

  /// Map a mouse position to the visible lyric line under it (rows and
  /// columns must both be inside a lyrics pane).
  fn lyrics_index_at(&self, mouse: MouseEvent) -> Option<usize> {
    let area = self.mouse_on_lyrics(mouse)?;
    Some((mouse.row - area.y) as usize + self.lyrics_scroll)
  }

  // --- queue viewport (shared math in `viewport`) -------------------------

  /// Scroll the queue by moving the viewport; the selection follows just
  /// enough to stay inside the visible window — the mouse scrolling
  /// convention the user asked for.
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

  pub(crate) fn queue_viewport_height(&self) -> usize {
    viewport::viewport_height(&self.queue_pane_areas)
  }

  fn mouse_on_queue_bar(&self, mouse: MouseEvent) -> Option<Rect> {
    viewport::hit_pane(&self.queue_bar_areas, mouse)
  }

  fn queue_bar_jump(&mut self, mouse: MouseEvent, track: Rect) -> bool {
    let len = self.visible_len();
    let height = self.queue_viewport_height();
    let changed = viewport::bar_jump(&mut self.queue_state, len, height, track, mouse.row);
    if changed {
      self.sync_hover_view();
    }
    changed
  }

  /// Map a screen position to the visible queue row under it.
  fn queue_row_index(&self, mouse: MouseEvent) -> Option<usize> {
    let area = viewport::hit_pane(&self.queue_pane_areas, mouse)?;
    viewport::row_at(area, mouse, self.queue_state.offset(), self.visible_len())
  }

  fn mouse_on_queue(&self, mouse: MouseEvent) -> Option<Rect> {
    viewport::hit_pane(&self.queue_pane_areas, mouse)
  }

  // --- lyrics / misc -------------------------------------------------------

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
    let line_count = self.lyrics.as_ref().map(Lyrics::line_count).unwrap_or(0);
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
    let last_visible = (next + height)
      .saturating_sub(1)
      .min(line_count.saturating_sub(1));
    self.lyrics_cursor = Some(pointer.clamp(next, last_visible));
    true
  }

  /// Scroll the f1 help dialog, clamped to the range computed at draw time.
  fn scroll_help(&mut self, delta: i32) -> bool {
    if self.max_help_scroll == 0 {
      return false;
    }
    let next = if delta < 0 {
      self
        .help_scroll
        .saturating_sub(delta.unsigned_abs() as usize)
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

  pub(crate) fn select_queue_row(&mut self, row: usize) {
    self
      .queue_state
      .select(Some(row.min(self.visible_len().saturating_sub(1))));
    self.sync_hover_view();
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
