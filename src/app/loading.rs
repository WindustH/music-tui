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
    let (artist, title) = self.current_song_tags();
    self.spawn_lyrics_load(url, path, artist, title);
  }

  /// Kick off the async reads for a freshly created song view
  /// (metadata + cover, plus lyrics when `with_lyrics` is set).
  pub(crate) fn spawn_song_view_loads(
    &self,
    url: String,
    path: &Path,
    artist: Option<String>,
    title: &str,
    with_lyrics: bool,
  ) {
    self.spawn_metadata_read(url.clone(), path.to_path_buf());
    self.spawn_cover_read(url.clone(), path.to_path_buf());
    if with_lyrics {
      self.spawn_lyrics_load(url, path.to_path_buf(), artist, Some(title.to_string()));
    }
  }

  pub(crate) fn spawn_lyrics_load(
    &self,
    url: String,
    path: PathBuf,
    artist: Option<String>,
    title: Option<String>,
  ) {
    let extra_dirs: Vec<PathBuf> = self
      .settings
      .config
      .lyrics
      .extra_dirs
      .iter()
      .map(|dir| expand_home(dir))
      .collect();
    let tx = self.events.clone();
    tokio::task::spawn_blocking(move || {
      let result = lyrics::load(&path, &extra_dirs, artist.as_deref(), title.as_deref());
      let _ = tx.send(AsyncEvent::Lyrics(LyricsOutcome {
        song_url: url,
        result,
      }));
    });
  }

  pub(crate) fn current_song_tags(&self) -> (Option<String>, Option<String>) {
    let song = self.current_song();
    (
      song.and_then(|song| song_artist(&song.song).map(str::to_string)),
      song.map(|song| {
        song_title(&song.song)
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
      let _ = tx.send(AsyncEvent::Metadata(MetadataOutcome {
        song_url: url,
        result,
      }));
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
      let _ = tx.send(AsyncEvent::Cover(CoverOutcome {
        song_url: url,
        result,
        dims,
      }));
    });
  }

  pub(crate) fn request_cover(&mut self, url: String, path: PathBuf) {
    self.spawn_cover_read(url, path);
  }

  pub(crate) fn song_path(&self, url: &str) -> Option<PathBuf> {
    uri_to_path(self.music_dir.as_deref(), url)
  }

  /// Refresh the `:hovered` data view for the queue's selected row. Cheap
  /// no-op when the hovered song has not changed; loads metadata / cover /
  /// lyrics lazily and only when some pane actually uses the source.
  pub(crate) fn sync_hover_view(&mut self) {
    if !self.has_hover_panes {
      return;
    }
    let hovered = self
      .queue_state
      .selected()
      .and_then(|row| self.filtered_position(row))
      .and_then(|index| self.queue.get(index));
    let Some(song) = hovered else {
      self.hover = None;
      return;
    };
    let url = song.song.url.to_string();
    if self.hover.as_ref().is_some_and(|hover| hover.url == url) {
      return;
    }
    let Some(path) = self.song_path(&url) else {
      self.hover = None;
      return;
    };
    let title = song_title(&song.song)
      .map(str::to_string)
      .unwrap_or_else(|| url.clone());
    let artist = song_artist(&song.song).map(str::to_string);
    let lyric_title = title.clone();
    self.hover = Some(SongView::new(url.clone(), path.clone(), title));
    self.spawn_song_view_loads(url, &path, artist, &lyric_title, true);
  }
}
