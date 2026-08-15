//! Secondary detail view for a queue entry (`i`).

use super::*;

/// Data view for a song that is *not* playing: fed by the queue's hovered
/// (selected) row through `:hovered` panes. Lyrics here have no playback
/// state — no sync highlight, no auto-follow, no click-to-seek.
pub struct HoverView {
  pub url: String,
  #[allow(dead_code)] // kept for parity with DetailView (future editor support)
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

/// gallery-tui's image detail view: the sidebar always shows the playing
/// song, details open as their own full-screen surface.
pub struct DetailView {
  pub url: String,
  pub path: PathBuf,
  pub title: String,
  pub metadata: Option<Vec<metadata::MetadataEntry>>,
  pub metadata_error: Option<String>,
  pub metadata_scroll: usize,
  pub cover: Option<PathBuf>,
  pub cover_dims: Option<(u32, u32)>,
  pub cover_error: Option<String>,
}

impl App {
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
      self.set_message("song is not under music_dir");
      return true;
    };
    let title = song_title(&song.song)
      .map(str::to_string)
      .unwrap_or_else(|| url.clone());
    self.detail = Some(DetailView {
      url: url.clone(),
      path: path.clone(),
      title,
      metadata: None,
      metadata_error: None,
      metadata_scroll: 0,
      cover: None,
      cover_dims: None,
      cover_error: None,
    });
    self.spawn_metadata_read(url.clone(), path.clone());
    self.spawn_cover_read(url, path);
    true
  }

  pub(crate) fn close_detail(&mut self) {
    self.detail = None;
  }

  /// `g` / `c` in the queue: jump the selection (and view) to the song that
  /// is currently playing.
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
