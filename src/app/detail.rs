//! Song views (playing song, detail view, hovered sidebars) and opening /
//! closing the full-screen detail view.

use super::*;

/// Data view for one song that is not necessarily playing: cover,
/// metadata and lyrics slots fed by async reads.
///
/// The same type backs four surfaces:
/// - `playing` — the song MPD is playing (`:playing` panes);
/// - `detail` — the full-screen view opened with `i`;
/// - `hover` — the queue's selected row feeding `:queue-hovered` panes;
/// - `library_hover` — the library's selected row feeding
///   `:library-hovered` panes.
///
/// Lyrics in a non-playing view have no playback state: no sync
/// highlight, no auto-follow, no click-to-seek. For the playing view,
/// `lyrics_scroll` is the viewport offset driven by follow mode.
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
    let Some(song) = self.selected_queue_song() else {
      return false;
    };
    let url = song.song.url.clone();
    let title = song_title(&song.song).unwrap_or_else(|| crate::sanitize::sanitize_text(&url));
    let Some(path) = self.song_path(&url) else {
      if self.detail.as_ref().is_some_and(|detail| detail.url == url) {
        self.close_detail();
      } else {
        self.set_message("local song path is unavailable");
      }
      return true;
    };
    self.open_detail_for(url, path, title)
  }

  /// Open the detail view for a song, or close it when it already shows
  /// that song (`i` toggles).
  pub(crate) fn open_detail_for(&mut self, url: String, path: PathBuf, title: String) -> bool {
    if self.detail.as_ref().is_some_and(|detail| detail.url == url) {
      self.close_detail();
      return true;
    }
    self.detail = Some(SongView::new(url.clone(), path.clone(), title));
    // The detail layout shows cover + metadata only; no lyrics load.
    self.spawn_metadata_read(url.clone(), path.clone());
    self.spawn_cover_read(url, path);
    true
  }

  pub(crate) fn close_detail(&mut self) {
    self.detail = None;
  }
}
