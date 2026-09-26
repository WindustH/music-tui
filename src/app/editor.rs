//! Metadata editor round-trip.

use super::*;

impl App {
  /// The song `e` edits: the detail view's song when open, else the
  /// selected row of a focused queue / library, else the song shown by the
  /// focused pane (playing or hovered).
  fn metadata_edit_target(
    &self,
  ) -> Result<(String, PathBuf, Option<Vec<metadata::MetadataEntry>>), &'static str> {
    let from_view = |view: &SongView| (view.url.clone(), view.path.clone(), view.metadata.clone());
    if let Some(detail) = self.detail.as_ref() {
      return Ok(from_view(detail));
    }
    match self.main_pane() {
      PaneKind::Queue => {
        let song = self.selected_queue_song().ok_or("nothing is selected")?;
        let url = song.song.url.clone();
        let path = self
          .song_path(&url)
          .ok_or("local song path is unavailable")?;
        Ok((url, path, None))
      }
      PaneKind::Library => {
        let track = self.library_hovered_track().ok_or("nothing is selected")?;
        Ok((
          track.path.to_string_lossy().into_owned(),
          track.path.clone(),
          None,
        ))
      }
      _ => match self.song_view(self.main_pane_source()) {
        Some(view) => Ok(from_view(view)),
        None if self.main_pane_source() != PaneSource::Playing => Err("nothing is selected"),
        None if self.current_song().is_none() => Err("nothing is playing"),
        None => Err("local song path is unavailable"),
      },
    }
  }

  pub(crate) fn request_metadata_editor(&mut self) {
    let (url, path, entries) = match self.metadata_edit_target() {
      Ok(target) => target,
      Err(message) => {
        self.set_message(message);
        return;
      }
    };
    if !path.is_file() {
      self.set_message(format!("file not found: {}", path.display()));
      return;
    }
    let entries = match entries.or_else(|| metadata::read_metadata(&path).ok()) {
      Some(entries) => entries,
      None => {
        self.set_message("failed to read metadata");
        return;
      }
    };
    let draft = metadata::metadata_draft(&path, &entries);
    self.editor_request = Some(EditorRequest::Metadata {
      song_url: url,
      path,
      original: entries,
      draft,
    });
  }

  pub fn finish_metadata_editor(&mut self, request: EditorRequest, edited: Option<String>) {
    let EditorRequest::Metadata {
      song_url,
      path,
      original,
      ..
    } = request;
    let Some(edited) = edited else {
      self.set_message("metadata edit cancelled");
      return;
    };
    let changes = match metadata::metadata_changes(&original, &edited) {
      Ok(changes) => changes,
      Err(error) => {
        self.set_message(format!("metadata edit failed: {error}"));
        return;
      }
    };
    if changes.is_empty() {
      self.set_message("metadata unchanged");
      return;
    }
    self.set_message(format!("writing {} tag change(s)...", changes.len()));
    let tx = self.events.clone();
    tokio::task::spawn_blocking(move || {
      let result = metadata::write_metadata(&path, &changes);
      let _ = tx.send(AsyncEvent::MetadataWrite(MetadataWriteOutcome {
        song_url,
        changed_tags: changes.len(),
        result: result.map(|_| ()),
      }));
    });
  }
}
