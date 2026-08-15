//! async outcome handlers.

use super::*;

impl App {
  pub fn handle_lyrics_outcome(&mut self, outcome: LyricsOutcome) -> bool {
    if let Some(hover) = self.hover.as_mut()
      && hover.url == outcome.song_url
    {
      match outcome.result {
        Ok(lyrics) => {
          hover.lyrics = Some(lyrics);
          hover.lyrics_error = None;
        }
        Err(error) => {
          hover.lyrics = None;
          hover.lyrics_error = Some(error);
        }
      }
      return true;
    }
    if outcome.song_url != self.lyrics_url {
      return false;
    }
    match outcome.result {
      Ok(lyrics) => {
        self.lyrics = Some(lyrics);
        self.lyrics_error = None;
      }
      Err(error) => {
        self.lyrics = None;
        self.lyrics_error = Some(error);
      }
    }
    true
  }

  pub fn handle_metadata_outcome(&mut self, outcome: MetadataOutcome) -> bool {
    let mut handled = false;
    if let Some(hover) = self.hover.as_mut()
      && hover.url == outcome.song_url
    {
      match &outcome.result {
        Ok(entries) => {
          hover.metadata = Some(entries.clone());
          hover.metadata_error = None;
        }
        Err(error) => {
          hover.metadata = None;
          hover.metadata_error = Some(error.clone());
        }
      }
      handled = true;
    }
    if let Some(detail) = self.detail.as_mut()
      && detail.url == outcome.song_url
    {
      match &outcome.result {
        Ok(entries) => {
          detail.metadata = Some(entries.clone());
          detail.metadata_error = None;
        }
        Err(error) => {
          detail.metadata = None;
          detail.metadata_error = Some(error.clone());
        }
      }
      handled = true;
    }
    if outcome.song_url == self.metadata_url {
      match outcome.result {
        Ok(entries) => {
          self.metadata_entries = Some(entries);
          self.metadata_error = None;
        }
        Err(error) => {
          self.metadata_entries = None;
          self.metadata_error = Some(error);
        }
      }
      handled = true;
    }
    handled
  }

  pub fn handle_metadata_write_outcome(&mut self, outcome: MetadataWriteOutcome) -> bool {
    match outcome.result {
      Ok(()) => {
        self.set_message(format!("metadata updated: {} tag(s)", outcome.changed_tags));
        // Refresh every slot that shows this song: the playing pane, the
        // detail view and the hovered data view (the editor can target any
        // of them).
        if outcome.song_url == self.current_song_url().unwrap_or_default() {
          self.metadata_entries = None;
          if let Some(path) = self.current_song_path() {
            self.request_metadata(outcome.song_url.clone(), path);
          }
        }
        if let Some(detail) = self.detail.as_mut()
          && detail.url == outcome.song_url
        {
          detail.metadata = None;
          let path = detail.path.clone();
          self.spawn_metadata_read(outcome.song_url.clone(), path);
        }
        if let Some(hover) = self.hover.as_mut()
          && hover.url == outcome.song_url
        {
          hover.metadata = None;
          let path = hover.path.clone();
          self.spawn_metadata_read(outcome.song_url.clone(), path);
        }
        // Ask MPD to re-read the file so its database (and the queue
        // labels) pick up the corrected tags without a manual :update.
        if !outcome.song_url.starts_with("file://") {
          self.mpdc(MpdCommand::UpdateUri(outcome.song_url));
        }
        true
      }
      Err(error) => {
        self.set_message(format!("metadata write failed: {error}"));
        true
      }
    }
  }

  pub fn handle_cover_outcome(&mut self, outcome: CoverOutcome) -> bool {
    let mut handled = false;
    if let Some(hover) = self.hover.as_mut()
      && hover.url == outcome.song_url
    {
      match &outcome.result {
        Ok(path) => {
          hover.cover_dims = outcome.dims;
          hover.cover = Some(path.clone());
          hover.cover_error = None;
        }
        Err(error) => {
          hover.cover = None;
          hover.cover_error = Some(error.clone());
        }
      }
      handled = true;
    }
    if let Some(detail) = self.detail.as_mut()
      && detail.url == outcome.song_url
    {
      match &outcome.result {
        Ok(path) => {
          detail.cover_dims = outcome.dims;
          detail.cover = Some(path.clone());
          detail.cover_error = None;
        }
        Err(error) => {
          detail.cover = None;
          detail.cover_error = Some(error.clone());
        }
      }
      handled = true;
    }
    // App-level sidebar cover tracks the *current* song only — a cover
    // outcome for the detail-view song must not clobber it.
    if outcome.song_url == self.current_song_url().unwrap_or_default() {
      match outcome.result {
        Ok(path) => {
          self.cover_dims = outcome.dims;
          self.cover_path = Some((outcome.song_url.clone(), path));
          self.cover_error = None;
        }
        Err(error) => {
          self.cover_path = None;
          self.cover_error = Some(error);
        }
      }
    }
    handled || self.tab_contains(PaneKind::Cover)
  }

  pub fn handle_spectrum(&mut self, bars: Vec<u8>) -> bool {
    self.spectrum = bars;
    self.request_visualizer_frame();
    self.tab_contains(PaneKind::Visualizer)
  }

  pub fn handle_visualizer_frame(&mut self, lines: Vec<ratatui::text::Line<'static>>) -> bool {
    self.visualizer_lines = Some(lines);
    self.tab_contains(PaneKind::Visualizer)
  }

  /// Record the visualizer pane size observed while drawing; a change
  /// re-renders the band lines off-thread.
  pub(crate) fn note_visualizer_geometry(&mut self, width: u16, height: u16) {
    if self.visualizer_geometry != Some((width, height)) {
      self.visualizer_geometry = Some((width, height));
      self.request_visualizer_frame();
    }
  }

  /// Hand the latest spectrum to the band-render worker (no-op while the
  /// pane has no size or no data yet).
  pub(crate) fn request_visualizer_frame(&mut self) {
    let Some(renderer) = self.visualizer_renderer.as_ref() else {
      return;
    };
    let Some((width, height)) = self.visualizer_geometry else {
      return;
    };
    if self.spectrum.is_empty() {
      return;
    }
    let theme = &self.settings.theme;
    let colors = crate::visualizer::VisualizerColors {
      low: theme.color(&theme.visualizer_low),
      mid: theme.color(&theme.visualizer_mid),
      high: theme.color(&theme.visualizer_high),
    };
    renderer.render(width, height, self.spectrum.clone(), colors);
  }

  pub fn handle_tick(&mut self) -> bool {
    if let Some((_, at)) = self.message
      && at.elapsed() > Duration::from_secs(4)
    {
      self.message = None;
      return true;
    }
    false
  }

  // --- input --------------------------------------------------------------
}
