//! metadata editor round-trip.

use super::*;

impl App {
  pub(crate) fn request_metadata_editor(&mut self) {
    // Editor target: the detailed song in the detail view, the selected
    // queue row when the queue is focused, otherwise the playing song.
    let (url, path, entries) = if let Some(detail) = self.detail.as_ref() {
      (
        detail.url.clone(),
        detail.path.clone(),
        detail.metadata.clone(),
      )
    } else if self.main_pane() == PaneKind::Queue
      && let Some(position) = self
        .queue_state
        .selected()
        .and_then(|row| self.filtered_position(row))
      && let Some(song) = self.queue.get(position)
    {
      let url = song.song.url.to_string();
      let Some(path) = self.song_path(&url) else {
        self.set_message("local song path is unavailable");
        return;
      };
      (url, path, None)
    } else {
      let Some(url) = self.current_song_url() else {
        self.set_message("nothing is playing");
        return;
      };
      let Some(path) = self.current_song_path() else {
        self.set_message("local song path is unavailable");
        return;
      };
      (url, path, self.metadata_entries.clone())
    };
    if !path.is_file() {
      self.set_message(format!("file not found: {}", path.display()));
      return;
    }
    let entries = match entries.or_else(|| metadata::read_metadata(&path).ok()) {
      Some(entries) => entries,
      None => {
        self.set_message("failed to read metadata".to_string());
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

  // --- draw-time helpers ---------------------------------------------------
}
