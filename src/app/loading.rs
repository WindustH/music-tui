//! Song-view loading pipeline: the playing song, the hovered queue row and
//! the hovered library row each get a [`SongView`] whose lyrics, metadata
//! and cover are read off the UI thread.

use super::*;

impl App {
  /// The playing song changed: rebuild its view and start the reads.
  pub(crate) fn on_song_changed(&mut self) {
    self.lyrics_cursor = None;
    if self.follow_current {
      self.follow_playing_position();
    }
    let Some(song) = self.current_song() else {
      self.playing = None;
      return;
    };
    let url = song.song.url.clone();
    let title = song_title(&song.song).unwrap_or_else(|| crate::sanitize::sanitize_text(&url));
    let artist = song_artist(&song.song);
    let Some(path) = self.song_path(&url) else {
      // Remote stream, or a relative URI without a music directory.
      self.playing = None;
      return;
    };
    self.playing = Some(SongView::new(url.clone(), path.clone(), title.clone()));
    self.spawn_song_view_loads(url, &path, artist, &title, true);
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

  fn spawn_lyrics_load(
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
    let Some(song) = self.selected_queue_song() else {
      self.hover = None;
      return;
    };
    if self
      .hover
      .as_ref()
      .is_some_and(|hover| hover.url == song.song.url)
    {
      return;
    }
    let url = song.song.url.clone();
    let title = song_title(&song.song).unwrap_or_else(|| crate::sanitize::sanitize_text(&url));
    let artist = song_artist(&song.song);
    let Some(path) = self.song_path(&url) else {
      self.hover = None;
      return;
    };
    self.hover = Some(SongView::new(url.clone(), path.clone(), title.clone()));
    self.spawn_song_view_loads(url, &path, artist, &title, true);
  }
}
