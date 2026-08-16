//! Library pane state: filtering, selection, hover sync and playback.

use super::*;

/// Which list the `/` filter prompt targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum FilterTarget {
  #[default]
  Queue,
  Library,
}

impl App {
  /// Visible library rows (filtered matches or all tracks).
  pub(crate) fn library_visible_len(&self) -> usize {
    self.library_rows.len()
  }

  pub(crate) fn move_library_selection_page(&mut self, delta: i32) -> bool {
    let height = self.library_viewport_height() as i32;
    self.move_library_selection(delta * height.max(1))
  }

  pub(crate) fn recompute_library_filter(&mut self) {
    let query = self.library_filter.clone().unwrap_or_default();
    if query.trim().is_empty() {
      self.library_rows = self
        .library
        .iter()
        .map(|track| crate::library_db::TrackMatch {
          track: track.clone(),
          field: crate::library_db::TrackField::Title,
        })
        .collect();
    } else {
      // `library` keeps the FULL list; rows are just the filtered view.
      self.library_rows = crate::library_db::filter_tracks(&self.library, &query);
    }
  }

  pub(crate) fn clear_library_filter(&mut self) {
    self.library_filter = None;
    self.recompute_library_filter();
    self.clamp_library_selection();
  }

  pub(crate) fn clamp_library_selection(&mut self) {
    let len = self.library_visible_len();
    if len == 0 {
      self.library_state.select(None);
      return;
    }
    let selected = self.library_state.selected().unwrap_or(0).min(len - 1);
    let mut state = TableState::default();
    state.select(Some(selected));
    self.library_state = state;
  }

  pub(crate) fn select_library_row(&mut self, row: usize) {
    let len = self.library_visible_len();
    if len == 0 {
      return;
    }
    let row = row.min(len - 1);
    let mut state = TableState::default();
    state.select(Some(row));
    self.library_state = state;
    self.sync_library_hover();
  }

  pub(crate) fn move_library_selection(&mut self, delta: i32) -> bool {
    let len = self.library_visible_len();
    if len == 0 {
      return false;
    }
    let current = self.library_state.selected().unwrap_or(0) as i32;
    let next = (current + delta).clamp(0, len as i32 - 1) as usize;
    self.select_library_row(next);
    true
  }

  pub(crate) fn library_viewport_height(&self) -> usize {
    self
      .library_pane_areas
      .first()
      .map(|area| area.height as usize)
      .filter(|height| *height > 0)
      .unwrap_or(1)
  }

  fn set_library_viewport(&mut self, next: usize) -> bool {
    let len = self.library_visible_len();
    if len == 0 {
      return false;
    }
    let height = self.library_viewport_height();
    let max_offset = len.saturating_sub(height);
    let next = next.min(max_offset);
    if next == self.library_state.offset() {
      return false;
    }
    let selected = self.library_state.selected().unwrap_or(next);
    let selected = selected.clamp(next, (next + height - 1).min(len - 1));
    let mut state = TableState::default();
    state.select(Some(selected));
    self.library_state = state.with_offset(next);
    self.sync_library_hover();
    true
  }

  pub(crate) fn scroll_library_viewport(&mut self, delta: i32) -> bool {
    let len = self.library_visible_len();
    if len == 0 {
      return false;
    }
    let height = self.library_viewport_height();
    let next = ((self.library_state.offset() as i32) + delta)
      .clamp(0, len.saturating_sub(height) as i32)
      .max(0) as usize;
    self.set_library_viewport(next)
  }

  /// Scrollbar hit test for the library pane.
  pub(crate) fn mouse_on_library_bar(&self, mouse: MouseEvent) -> Option<Rect> {
    self.library_bar_areas.iter().copied().find(|area| {
      mouse.column >= area.x
        && mouse.column < area.x + area.width
        && mouse.row >= area.y
        && mouse.row < area.y + area.height
    })
  }

  /// Map a library scrollbar click/drag to a viewport offset.
  pub(crate) fn library_bar_jump(&mut self, mouse: MouseEvent, track: Rect) -> bool {
    let len = self.library_visible_len();
    if len == 0 {
      return false;
    }
    let height = self.library_viewport_height();
    let track_span = track.height.saturating_sub(1) as f64;
    let ratio = if track_span <= 0.0 {
      0.0
    } else {
      (mouse.row.saturating_sub(track.y) as f64 / track_span).clamp(0.0, 1.0)
    };
    let target_center = ratio * (len.saturating_sub(1)) as f64;
    let next = (target_center - height as f64 / 2.0)
      .round()
      .clamp(0.0, len.saturating_sub(height) as f64) as usize;
    self.set_library_viewport(next)
  }

  /// The track hovered (selected) in the library pane.
  pub(crate) fn library_hovered_track(&self) -> Option<&crate::library_db::LibraryTrack> {
    let row = self.library_state.selected()?;
    self.library_rows.get(row).map(|matched| &matched.track)
  }

  /// Feed `:library-hovered` panes from the library selection. Mirrors
  /// `sync_hover_view` for the queue.
  pub(crate) fn sync_library_hover(&mut self) {
    if !self.has_library_hover_panes {
      return;
    }
    let Some(track) = self.library_hovered_track().cloned() else {
      self.library_hover = None;
      return;
    };
    if self
      .library_hover
      .as_ref()
      .is_some_and(|hover| hover.url == track.path.to_string_lossy())
    {
      return;
    }
    let url = track.path.to_string_lossy().to_string();
    let title = if track.title.is_empty() {
      track.filename.clone()
    } else {
      track.title.clone()
    };
    let artist = (!track.artist.is_empty()).then(|| track.artist.clone());
    let lyric_title = title.clone();
    self.library_hover = Some(HoverView {
      url: url.clone(),
      path: track.path.clone(),
      title,
      metadata: None,
      metadata_error: None,
      metadata_scroll: 0,
      cover: None,
      cover_dims: None,
      cover_error: None,
      lyrics: None,
      lyrics_error: None,
      lyrics_scroll: 0,
    });
    self.spawn_metadata_read(url.clone(), track.path.clone());
    self.spawn_cover_read(url.clone(), track.path.clone());
    self.spawn_lyrics_load(url, track.path.clone(), artist, Some(lyric_title));
  }

  /// Scan finished: swap in the new track list.
  pub(crate) fn library_loaded(&mut self, tracks: Vec<crate::library_db::LibraryTrack>) {
    self.library_scanning = None;
    self.library = tracks;
    self.recompute_library_filter();
    self.clamp_library_selection();
    self.sync_library_hover();
  }

  /// `enter` in the library: play the selected track now (inserted right
  /// after the current song; appended and started when idle).
  pub(crate) fn library_play_selected(&mut self) -> bool {
    let Some(track) = self.library_hovered_track().cloned() else {
      return false;
    };
    let title = if track.title.is_empty() {
      track.filename.clone()
    } else {
      track.title.clone()
    };
    self.mpdc(MpdCommand::PlayLibrary {
      path: track.path.clone(),
      append: false,
    });
    self.set_message(format!("playing {title}"));
    true
  }

  /// `a` in the library: append the selected track to the queue (starts
  /// playing when idle).
  pub(crate) fn library_append_selected(&mut self) -> bool {
    let Some(track) = self.library_hovered_track().cloned() else {
      return false;
    };
    self.mpdc(MpdCommand::PlayLibrary {
      path: track.path.clone(),
      append: true,
    });
    self.set_message(format!("queued {}", title_of(&track)));
    true
  }

  /// `i` in the library: open the detail view for the selected track.
  pub(crate) fn open_library_detail(&mut self) -> bool {
    let Some(track) = self.library_hovered_track().cloned() else {
      return false;
    };
    let url = track.path.to_string_lossy().to_string();
    if self.detail.as_ref().is_some_and(|detail| detail.url == url) {
      self.close_detail();
      return true;
    }
    let title = if track.title.is_empty() {
      track.filename.clone()
    } else {
      track.title.clone()
    };
    self.detail = Some(DetailView {
      url: url.clone(),
      path: track.path.clone(),
      title,
      metadata: None,
      metadata_error: None,
      metadata_scroll: 0,
      cover: None,
      cover_dims: None,
      cover_error: None,
    });
    self.spawn_metadata_read(url.clone(), track.path.clone());
    self.spawn_cover_read(url, track.path.clone());
    true
  }

  /// `u` in the library: ask the scanner thread to rescan.
  pub(crate) fn library_rescan(&mut self) -> bool {
    if let Some(tx) = &self.library_scan_tx {
      let _ = tx.send(());
      self.set_message("rescanning library…");
    } else {
      self.set_message("library is not configured ([library] paths)");
    }
    true
  }
}

fn title_of(track: &crate::library_db::LibraryTrack) -> String {
  if track.title.is_empty() {
    track.filename.clone()
  } else {
    track.title.clone()
  }
}
