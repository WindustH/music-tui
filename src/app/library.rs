//! Library pane state: filtering, selection, hover sync and playback.

use super::viewport::PaneState;
use super::*;
use crate::library_db::{LibraryTrack, all_rows, filter_tracks};

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
    let query = self
      .library_filter
      .as_deref()
      .map(str::trim)
      .unwrap_or_default()
      .to_string();
    self.library_rows = if query.is_empty() {
      all_rows(&self.library)
    } else if !self.library_rows_query.is_empty() && query.starts_with(&self.library_rows_query) {
      // Typing more of the query only narrows the result: every track that
      // matches the longer query also matched the shorter one.
      let previous: Vec<usize> = self.library_rows.iter().map(|row| row.index).collect();
      filter_tracks(&self.library, previous, &query)
    } else {
      filter_tracks(&self.library, 0..self.library.len(), &query)
    };
    self.library_rows_query = query;
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
    viewport::viewport_height(&self.hit.library_panes)
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

  /// Map a screen position to the visible library row under it.
  pub(crate) fn library_row_index(&self, mouse: MouseEvent) -> Option<usize> {
    let area = viewport::hit_pane(&self.hit.library_panes, mouse)?;
    viewport::row_at(
      area,
      mouse,
      self.library_state.offset(),
      self.library_visible_len(),
    )
  }

  /// The track behind a visible library row.
  pub(crate) fn library_row_track(&self, row: usize) -> Option<&LibraryTrack> {
    self.library.get(self.library_rows.get(row)?.index)
  }

  /// The track hovered (selected) in the library pane.
  pub(crate) fn library_hovered_track(&self) -> Option<&LibraryTrack> {
    self.library_row_track(self.library_state.selected()?)
  }

  /// Feed `:library-hovered` panes from the library selection. Mirrors
  /// `sync_hover_view` for the queue.
  pub(crate) fn sync_library_hover(&mut self) {
    if !self.has_library_hover_panes {
      return;
    }
    let Some(track) = self.library_hovered_track() else {
      self.library_hover = None;
      return;
    };
    let url = track.path.to_string_lossy().into_owned();
    if self
      .library_hover
      .as_ref()
      .is_some_and(|hover| hover.url == url)
    {
      return;
    }
    let path = track.path.clone();
    let title = title_of(track);
    let artist = (!track.artist.is_empty()).then(|| track.artist.clone());
    self.library_hover = Some(SongView::new(url.clone(), path.clone(), title.clone()));
    self.spawn_song_view_loads(url, &path, artist, &title, true);
  }

  /// Scanner events: progress, the finished track list, or a failure.
  pub fn handle_library_event(&mut self, event: crate::event::LibraryEvent) -> bool {
    use crate::event::LibraryEvent;
    match event {
      LibraryEvent::Scanning { scanned, changed } => {
        self.library_scanning = Some((scanned, changed));
      }
      LibraryEvent::Loaded(tracks) => {
        self.library_loaded(tracks);
        // The startup scan stays quiet: its message would replace startup
        // notices (config warnings, `open` results) almost immediately.
        if std::mem::take(&mut self.library_rescan_requested) {
          self.set_message("library ready");
        }
      }
      LibraryEvent::Failed { error, tracks } => {
        // The database still holds the previous scan: keep showing it.
        if let Some(tracks) = tracks {
          self.library_loaded(tracks);
        } else {
          self.library_scanning = None;
        }
        self.set_message(format!("library scan failed: {error}"));
      }
    }
    self.pane_visible(PaneKind::Library) || self.message.is_some()
  }

  /// Scan finished: swap in the new track list.
  fn library_loaded(&mut self, tracks: Vec<LibraryTrack>) {
    self.library_scanning = None;
    self.library = tracks;
    self.library_rows_query.clear();
    self.recompute_library_filter();
    self.clamp_library_selection();
  }

  /// `enter` in the library: play the selected track now (inserted right
  /// after the current song; appended and started when idle).
  pub(crate) fn library_play_selected(&mut self) -> bool {
    self.library_queue_selected(false)
  }

  /// `a` in the library: append the selected track to the queue (starts
  /// playing when idle).
  pub(crate) fn library_append_selected(&mut self) -> bool {
    self.library_queue_selected(true)
  }

  fn library_queue_selected(&mut self, append: bool) -> bool {
    let Some(track) = self.library_hovered_track() else {
      return false;
    };
    let path = track.path.clone();
    let title = title_of(track);
    self.mpdc(MpdCommand::PlayLibrary { path, append });
    self.set_message(if append {
      format!("queued {title}")
    } else {
      format!("playing {title}")
    });
    true
  }

  /// `i` in the library: open the detail view for the selected track.
  pub(crate) fn open_library_detail(&mut self) -> bool {
    let Some(track) = self.library_hovered_track() else {
      return false;
    };
    let url = track.path.to_string_lossy().into_owned();
    let path = track.path.clone();
    let title = title_of(track);
    self.open_detail_for(url, path, title)
  }

  /// `u` in the library: ask the scanner thread to rescan.
  pub(crate) fn library_rescan(&mut self) -> bool {
    if let Some(tx) = &self.library_scan_tx {
      let _ = tx.send(());
      self.library_rescan_requested = true;
      self.set_message("rescanning library…");
    } else {
      self.set_message("library is not configured ([library] paths)");
    }
    true
  }
}

fn title_of(track: &LibraryTrack) -> String {
  if track.title.is_empty() {
    track.filename.clone()
  } else {
    track.title.clone()
  }
}
