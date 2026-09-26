//! Mouse handling: tab/pane/band hit tests, clicks, drags and the wheel.
//! Pane geometry comes from [`HitAreas`], recorded at draw time.

use super::*;

impl App {
  pub(crate) fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
    if self.show_help {
      return match mouse.kind {
        MouseEventKind::Down(_) => {
          self.show_help = false;
          true
        }
        MouseEventKind::ScrollUp => self.scroll_help(-3),
        MouseEventKind::ScrollDown => self.scroll_help(3),
        _ => false,
      };
    }
    match mouse.kind {
      MouseEventKind::Down(MouseButton::Left) => self.left_click(mouse),
      MouseEventKind::Drag(MouseButton::Left) => self.left_drag(mouse),
      MouseEventKind::Up(MouseButton::Left) => self.drag.take().is_some(),
      MouseEventKind::ScrollUp => self.handle_pane_wheel(mouse, -3),
      MouseEventKind::ScrollDown => self.handle_pane_wheel(mouse, 3),
      MouseEventKind::Down(MouseButton::Middle) => self.middle_click(mouse),
      _ => false,
    }
  }

  fn left_click(&mut self, mouse: MouseEvent) -> bool {
    // Clicking a tab label in the tab bar switches to that tab.
    if let Some(index) = viewport::hit_index(&self.hit.tabs, mouse)
      && index != self.tab
    {
      return self.goto_tab(index);
    }
    // Clicking a synced lyric line seeks to its timestamp.
    if let Some((area, source)) = self.lyrics_pane_at(mouse) {
      self.click_lyrics(area, source, mouse);
      return true;
    }
    // Scrollbars: jump the viewport proportionally and arm dragging (the
    // thumb follows the pointer while held).
    if let Some(track) = viewport::hit_pane(&self.hit.queue_bars, mouse) {
      self.drag = Some(Drag::QueueBar);
      return self.queue_bar_jump(mouse, track);
    }
    if let Some(track) = viewport::hit_pane(&self.hit.library_bars, mouse) {
      self.drag = Some(Drag::LibraryBar);
      return self.library_bar_jump(mouse, track);
    }
    // Click selects; clicking the already-selected row plays it —
    // double-click without the timer.
    if let Some(row) = self.queue_row_index(mouse) {
      if self.queue_state.selected() == Some(row) {
        self.play_selected_queue_row();
      } else {
        self.select_queue_row(row);
      }
      return true;
    }
    if let Some(row) = self.library_row_index(mouse) {
      if self.library_state.selected() == Some(row) {
        self.library_play_selected();
      } else {
        self.select_library_row(row);
      }
      return true;
    }
    if self.mouse_on_band(mouse) {
      self.drag = Some(Drag::Band);
      return self.seek_to_band_column(mouse.column);
    }
    false
  }

  fn left_drag(&mut self, mouse: MouseEvent) -> bool {
    match self.drag {
      Some(Drag::Band) => self.seek_to_band_column(mouse.column),
      Some(Drag::QueueBar) => match viewport::hit_pane(&self.hit.queue_bars, mouse) {
        Some(track) => self.queue_bar_jump(mouse, track),
        None => false,
      },
      Some(Drag::LibraryBar) => match viewport::hit_pane(&self.hit.library_bars, mouse) {
        Some(track) => self.library_bar_jump(mouse, track),
        None => false,
      },
      None => false,
    }
  }

  fn middle_click(&mut self, mouse: MouseEvent) -> bool {
    if let Some(row) = self.queue_row_index(mouse) {
      self.select_queue_row(row);
      return self.play_selected_queue_row();
    }
    if let Some(row) = self.library_row_index(mouse) {
      self.select_library_row(row);
      return self.library_play_selected();
    }
    false
  }

  /// Wheel over the interface: the seek band nudges playback; queue,
  /// library and lyrics panes scroll their viewports.
  fn handle_pane_wheel(&mut self, mouse: MouseEvent, delta: i32) -> bool {
    if self.mouse_on_band(mouse) {
      self.mpdc(MpdCommand::NudgeSeek(i64::from(delta.signum() * 5)));
      true
    } else if viewport::hit_pane(&self.hit.queue_panes, mouse).is_some() {
      self.scroll_queue_viewport(delta)
    } else if viewport::hit_pane(&self.hit.library_panes, mouse).is_some() {
      self.scroll_library_viewport(delta)
    } else if let Some((_, source)) = self.lyrics_pane_at(mouse) {
      self.scroll_lyrics_wheel(source, delta)
    } else {
      false
    }
  }

  /// Scroll the f1 help dialog, clamped to the range computed at draw time.
  fn scroll_help(&mut self, delta: i32) -> bool {
    let next = self
      .help_scroll
      .saturating_add_signed(delta as isize)
      .min(self.max_help_scroll);
    if next == self.help_scroll {
      return false;
    }
    self.help_scroll = next;
    true
  }

  fn mouse_on_band(&self, mouse: MouseEvent) -> bool {
    self
      .hit
      .progress_band
      .is_some_and(|area| viewport::contains(area, mouse))
  }

  /// Seek to the playback position under a screen column of the progress band.
  fn seek_to_band_column(&mut self, column: u16) -> bool {
    let Some(area) = self.hit.progress_band.filter(|area| area.width > 0) else {
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
