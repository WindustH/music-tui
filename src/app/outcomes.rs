//! Async outcome handlers: lyrics / metadata / cover reads land in every
//! song view showing that song; visualizer frames and message expiry.

use super::*;

impl App {
  /// Apply `apply` to every song view (playing / detail / queue hover /
  /// library hover) whose url matches. Returns whether any view was hit.
  fn for_each_song_view(&mut self, url: &str, mut apply: impl FnMut(&mut SongView)) -> bool {
    let mut handled = false;
    for view in [
      self.playing.as_mut(),
      self.detail.as_mut(),
      self.hover.as_mut(),
      self.library_hover.as_mut(),
    ]
    .into_iter()
    .flatten()
    {
      if view.url == url {
        apply(view);
        handled = true;
      }
    }
    handled
  }

  pub fn handle_lyrics_outcome(&mut self, outcome: LyricsOutcome) -> bool {
    self.for_each_song_view(&outcome.song_url, |view| match &outcome.result {
      Ok(lyrics) => {
        view.lyrics = Some(lyrics.clone());
        view.lyrics_error = None;
      }
      Err(error) => {
        view.lyrics = None;
        view.lyrics_error = Some(error.clone());
      }
    })
  }

  pub fn handle_metadata_outcome(&mut self, outcome: MetadataOutcome) -> bool {
    self.for_each_song_view(&outcome.song_url, |view| match &outcome.result {
      Ok(entries) => {
        view.metadata = Some(entries.clone());
        view.metadata_error = None;
      }
      Err(error) => {
        view.metadata = None;
        view.metadata_error = Some(error.clone());
      }
    })
  }

  pub fn handle_metadata_write_outcome(&mut self, outcome: MetadataWriteOutcome) -> bool {
    if let Err(error) = outcome.result {
      self.set_message(format!("metadata write failed: {error}"));
      return true;
    }
    self.set_message(format!("metadata updated: {} tag(s)", outcome.changed_tags));
    // Re-read metadata for every view showing this song (the editor can
    // target any of them).
    let mut paths = Vec::new();
    self.for_each_song_view(&outcome.song_url, |view| {
      view.metadata = None;
      paths.push(view.path.clone());
    });
    paths.dedup();
    for path in paths {
      self.spawn_metadata_read(outcome.song_url.clone(), path);
    }
    // Ask MPD to re-read the file so its database (and the queue labels)
    // pick up the corrected tags without a manual :update.
    if crate::library::local_uri_to_path(&outcome.song_url).is_none() {
      self.mpdc(MpdCommand::UpdateUri(outcome.song_url));
    }
    true
  }

  pub fn handle_cover_outcome(&mut self, outcome: CoverOutcome) -> bool {
    self.for_each_song_view(&outcome.song_url, |view| match &outcome.result {
      Ok(path) => {
        view.cover_dims = outcome.dims;
        view.cover = Some(path.clone());
        view.cover_error = None;
      }
      Err(error) => {
        view.cover = None;
        view.cover_error = Some(error.clone());
      }
    })
  }

  #[cfg(unix)]
  pub fn handle_spectrum(&mut self, bars: Vec<u8>) -> bool {
    self.spectrum = bars;
    self.request_visualizer_frame();
    false
  }

  pub fn handle_visualizer_frame(&mut self, lines: Vec<ratatui::text::Line<'static>>) -> bool {
    self.visualizer_lines = Some(lines);
    self.pane_visible(PaneKind::Visualizer)
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
  fn request_visualizer_frame(&mut self) {
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
      low: theme.color(&theme.visualizer.low),
      mid: theme.color(&theme.visualizer.mid),
      high: theme.color(&theme.visualizer.high),
    };
    renderer.render(width, height, self.spectrum.clone(), colors);
  }

  /// `visualizer_reset`: drop the current bars until fresh audio arrives.
  pub(crate) fn reset_visualizer(&mut self) -> bool {
    self.spectrum.fill(0);
    self.request_visualizer_frame();
    true
  }
}
