//! song-change loading pipeline.

use super::*;

impl App {
  pub(crate) fn on_song_changed(&mut self) {
    self.lyrics = None;
    self.lyrics_error = None;
    self.lyrics_scroll = 0;
    self.lyrics_cursor = None;
    self.metadata_entries = None;
    self.metadata_error = None;
    self.metadata_scroll = 0;
    self.cover_path = None;
    self.cover_dims = None;
    self.cover_error = None;
    if self.follow_current {
      self.follow_playing_position();
    }
    if let (Some(url), Some(path)) = (self.current_song_url(), self.current_song_path()) {
      self.request_lyrics(url.clone(), path.clone());
      self.request_metadata(url.clone(), path.clone());
      self.request_cover(url, path);
    }
  }

  pub(crate) fn request_lyrics(&mut self, url: String, path: PathBuf) {
    self.lyrics_url = url.clone();
    let extra_dirs: Vec<PathBuf> = self
      .settings
      .config
      .lyrics
      .extra_dirs
      .iter()
      .map(|dir| expand_home(dir))
      .collect();
    let (artist, title) = self.current_song_tags();
    let tx = self.events.clone();
    tokio::task::spawn_blocking(move || {
      let result = lyrics::load(&path, &extra_dirs, artist.as_deref(), title.as_deref());
      let _ = tx.send(AsyncEvent::Lyrics(LyricsOutcome { song_url: url, result }));
    });
  }

  pub(crate) fn current_song_tags(&self) -> (Option<String>, Option<String>) {
    let song = self.current_song();
    (
      song.and_then(|song| song.song.artists().first().cloned()),
      song.map(|song| {
        song
          .song
          .title()
          .map(str::to_string)
          .unwrap_or_else(|| song.song.url.clone())
      }),
    )
  }

  pub(crate) fn request_metadata(&mut self, url: String, path: PathBuf) {
    self.metadata_url = url.clone();
    self.spawn_metadata_read(url, path);
  }

  pub(crate) fn spawn_metadata_read(&self, url: String, path: PathBuf) {
    let tx = self.events.clone();
    tokio::task::spawn_blocking(move || {
      let result = metadata::read_metadata(&path);
      let _ = tx.send(AsyncEvent::Metadata(MetadataOutcome { song_url: url, result }));
    });
  }

  pub(crate) fn spawn_cover_read(&self, url: String, path: PathBuf) {
    let cache_dir = self.settings.cache_dir.join("covers");
    let tx = self.events.clone();
    tokio::task::spawn_blocking(move || {
      let result = cover::find_cover(&path, &cache_dir);
      let dims = result
        .as_ref()
        .ok()
        .and_then(|path| image::image_dimensions(path).ok());
      let _ = tx.send(AsyncEvent::Cover(CoverOutcome { song_url: url, result, dims }));
    });
  }

  pub(crate) fn request_cover(&mut self, url: String, path: PathBuf) {
    self.spawn_cover_read(url, path);
  }

  pub(crate) fn song_path(&self, url: &str) -> Option<PathBuf> {
    self.music_dir.as_ref().map(|dir| uri_to_path(dir, url))
  }

}
