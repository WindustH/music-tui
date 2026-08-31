//! Library pane state: filtering, selection, hover sync and playback.

use super::viewport::PaneState;
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

  /// Flip the library viewport one full page (selection follows
  /// passively, like the queue's paging).
  pub(crate) fn library_page(&mut self, direction: i32) -> bool {
    let height = self.library_viewport_height() as i32;
    self.scroll_library_viewport(direction * height.max(1))
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
    match viewport::clamp_selection(
      self.library_state.selected(),
      self.library_state.offset(),
      len,
      self.library_viewport_height(),
    ) {
      Some((selected, offset)) => self.library_state.install(offset, selected),
      None => self.library_state.select(None),
    }
    self.sync_library_hover();
  }

  pub(crate) fn select_library_row(&mut self, row: usize) {
    let len = self.library_visible_len();
    if len == 0 {
      return;
    }
    let row = row.min(len - 1);
    // Select in place: rebuilding the state would reset the viewport
    // offset to 0 and the table render would jump back to the top.
    self.library_state.select(Some(row));
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
    viewport::viewport_height(&self.library_pane_areas)
  }

  pub(crate) fn scroll_library_viewport(&mut self, delta: i32) -> bool {
    let len = self.library_visible_len();
    let height = self.library_viewport_height();
    let changed = viewport::scroll_viewport(&mut self.library_state, len, height, delta);
    if changed {
      self.sync_library_hover();
    }
    changed
  }

  /// Scrollbar hit test for the library pane.
  pub(crate) fn mouse_on_library_bar(&self, mouse: MouseEvent) -> Option<Rect> {
    viewport::hit_pane(&self.library_bar_areas, mouse)
  }

  /// Map a library scrollbar click/drag to a viewport offset.
  pub(crate) fn library_bar_jump(&mut self, mouse: MouseEvent, track: Rect) -> bool {
    let len = self.library_visible_len();
    let height = self.library_viewport_height();
    let changed = viewport::bar_jump(&mut self.library_state, len, height, track, mouse.row);
    if changed {
      self.sync_library_hover();
    }
    changed
  }

  pub(crate) fn mouse_on_library(&self, mouse: MouseEvent) -> Option<Rect> {
    viewport::hit_pane(&self.library_pane_areas, mouse)
  }

  /// Map a screen position to the visible library row under it.
  pub(crate) fn library_row_index(&self, mouse: MouseEvent) -> Option<usize> {
    let area = self.mouse_on_library(mouse)?;
    viewport::row_at(
      area,
      mouse,
      self.library_state.offset(),
      self.library_visible_len(),
    )
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
    let title = title_of(&track);
    let artist = (!track.artist.is_empty()).then(|| track.artist.clone());
    let lyric_title = title.clone();
    self.library_hover = Some(SongView::new(url.clone(), track.path.clone(), title));
    self.spawn_song_view_loads(url, &track.path, artist, &lyric_title, true);
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
    let title = title_of(&track);
    self.open_detail_for(url, track.path.clone(), title)
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
