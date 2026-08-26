//! Secondary song views: the full-screen detail view and the hovered
//! sidebars (`:queue-hovered` / `:library-hovered` panes).

use super::*;

/// Data view for one song that is not necessarily playing: cover,
/// metadata and lyrics slots fed by async reads.
///
/// The same type backs three surfaces:
/// - `detail` — the full-screen view opened with `i`;
/// - `hover` — the queue's selected row feeding `:queue-hovered` panes;
/// - `library_hover` — the library's selected row feeding
///   `:library-hovered` panes.
///
/// Lyrics in a non-playing view have no playback state: no sync
/// highlight, no auto-follow, no click-to-seek.
pub struct SongView {
  pub url: String,
  pub path: PathBuf,
  pub title: String,
  pub metadata: Option<Vec<metadata::MetadataEntry>>,
  pub metadata_error: Option<String>,
  pub metadata_scroll: usize,
  pub cover: Option<PathBuf>,
  pub cover_dims: Option<(u32, u32)>,
  pub cover_error: Option<String>,
  pub lyrics: Option<crate::lyrics::Lyrics>,
  pub lyrics_error: Option<String>,
  pub lyrics_scroll: usize,
}

impl SongView {
  pub(crate) fn new(url: String, path: PathBuf, title: String) -> Self {
    Self {
      url,
      path,
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
    }
  }
}

impl App {
  /// gallery-tui's image detail view pattern: the sidebar always shows
  /// the playing song, details open as their own full-screen surface.
  pub(crate) fn open_detail(&mut self) -> bool {
    let Some(index) = self.queue_state.selected() else {
      return false;
    };
    let Some(index) = self.filtered_position(index) else {
      return false;
    };
    let Some(song) = self.queue.get(index) else {
      return false;
    };
    let url = song.song.url.to_string();
    if self.detail.as_ref().is_some_and(|detail| detail.url == url) {
      self.close_detail();
      return true;
    }
    let Some(path) = self.song_path(&url) else {
      self.set_message("local song path is unavailable");
      return true;
    };
    let title = song_title(&song.song)
      .map(str::to_string)
      .unwrap_or_else(|| url.clone());
    self.open_detail_view(url, path, title);
    true
  }

  /// `i` in the library: same detail view, sourced from the library row.
  pub(crate) fn open_detail_for(&mut self, url: String, path: PathBuf, title: String) -> bool {
    if self.detail.as_ref().is_some_and(|detail| detail.url == url) {
      self.close_detail();
      return true;
    }
    self.open_detail_view(url, path, title);
    true
  }

  fn open_detail_view(&mut self, url: String, path: PathBuf, title: String) {
    self.detail = Some(SongView::new(url.clone(), path.clone(), title));
    // The detail layout shows cover + metadata only; no lyrics load.
    self.spawn_metadata_read(url.clone(), path.clone());
    self.spawn_cover_read(url, path);
  }

  pub(crate) fn close_detail(&mut self) {
    self.detail = None;
  }

  /// `g` / `c` in the queue: jump the selection (and view) to the song
  /// that is currently playing.
  pub(crate) fn goto_playing(&mut self) -> bool {
    let Some(position) = self.status.as_ref().and_then(|status| status.current_song) else {
      self.set_message("nothing is playing");
      return true;
    };
    let row = self
      .queue_filter_matches
      .iter()
      .position(|candidate| *candidate == position.0 .0)
      .unwrap_or(position.0 .0);
    self.select_queue_row(row);
    true
  }
}
