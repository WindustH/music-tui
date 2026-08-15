//! Persistent UI state: restored when music-tui starts with no subcommand,
//! saved whenever it changes (atomic writes, crash-safe).
//!
//! Deliberately minimal — song-dependent values (scroll offsets, previews)
//! are transient; MPD itself restores the queue via its own state file.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::app::App;

pub const STATE_FILE: &str = "state.toml";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct PersistedState {
  /// Active tab index (clamped to the configured tabs on restore).
  pub tab: usize,
  /// Lyrics auto-follow preference; `None` keeps the default (on).
  pub lyrics_follow: Option<bool>,
  /// Queue selection to restore once the queue arrives from mpd.
  pub queue_selected: Option<usize>,
}


impl PersistedState {
  pub fn load(cache_dir: &Path) -> Self {
    let path = cache_dir.join(STATE_FILE);
    match std::fs::read_to_string(&path) {
      Ok(text) => match toml::from_str(&text) {
        Ok(state) => state,
        Err(error) => {
          tracing::warn!(%error, path = %path.display(), "invalid state file, using defaults");
          Self::default()
        }
      },
      Err(_) => Self::default(),
    }
  }

  /// Write atomically (temp file + rename) so a crash never corrupts it.
  pub fn save(&self, cache_dir: &Path) {
    if let Err(error) = save_inner(self, cache_dir) {
      tracing::warn!(%error, "failed to save state");
    }
  }
}

fn save_inner(state: &PersistedState, cache_dir: &Path) -> std::io::Result<()> {
  std::fs::create_dir_all(cache_dir)?;
  let path = cache_dir.join(STATE_FILE);
  let temp = cache_dir.join(format!("{STATE_FILE}.tmp"));
  let text = toml::to_string_pretty(state).map_err(|error| {
    std::io::Error::other(format!("serialize state: {error}"))
  })?;
  std::fs::write(&temp, text.as_bytes())?;
  std::fs::rename(&temp, &path)?;
  Ok(())
}

impl App {
  /// Capture the current UI state for persistence.
  pub fn snapshot_state(&self) -> PersistedState {
    PersistedState {
      tab: self.tab,
      lyrics_follow: Some(self.lyrics_follow),
      queue_selected: self.queue_state.selected(),
    }
  }

  /// Apply a previously persisted state (called once at startup).
  pub fn restore_state(&mut self, state: PersistedState) {
    if !self.tabs.is_empty() {
      self.tab = state.tab.min(self.tabs.len() - 1);
    }
    if let Some(follow) = state.lyrics_follow {
      self.lyrics_follow = follow;
    }
    self.pending_restore_selection = state.queue_selected;
  }
}
