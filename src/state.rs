//! Persistent UI state: restored at startup, saved shortly after it
//! changes and on exit (atomic writes, crash-safe).
//!
//! Deliberately minimal — song-dependent values (scroll offsets, previews)
//! are transient; MPD itself restores the queue via its own state file.

use std::{
  path::{Path, PathBuf},
  time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

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
  pub fn load(state_dir: &Path) -> Self {
    let path = state_dir.join(STATE_FILE);
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
  pub fn save(&self, state_dir: &Path) {
    if let Err(error) = save_inner(self, state_dir) {
      tracing::warn!(%error, "failed to save state");
    }
  }
}

/// Coalesces state saves: a change is written once it has been pending
/// for [`StateSaver::DELAY`], so holding `j` in the queue costs one fsync'd
/// write per second instead of one per keypress.
pub struct StateSaver {
  dir: PathBuf,
  saved: PersistedState,
  pending: Option<(PersistedState, Instant)>,
}

impl StateSaver {
  const DELAY: Duration = Duration::from_secs(1);

  pub fn new(dir: &Path, saved: PersistedState) -> Self {
    Self {
      dir: dir.to_path_buf(),
      saved,
      pending: None,
    }
  }

  /// Record the current state; schedules a save when it differs from the
  /// last saved one.
  pub fn update(&mut self, current: PersistedState, now: Instant) {
    if current == self.saved {
      self.pending = None;
      return;
    }
    let due = self
      .pending
      .as_ref()
      .map_or(now + Self::DELAY, |(_, due)| *due);
    self.pending = Some((current, due));
  }

  /// When the pending save is due.
  pub fn deadline(&self) -> Option<Instant> {
    self.pending.as_ref().map(|(_, due)| *due)
  }

  /// Write the pending state if it is due (or unconditionally on `force`).
  pub fn flush(&mut self, now: Instant, force: bool) {
    let due = self.deadline().is_some_and(|due| force || due <= now);
    if due && let Some((state, _)) = self.pending.take() {
      state.save(&self.dir);
      self.saved = state;
    }
  }
}

fn save_inner(state: &PersistedState, state_dir: &Path) -> std::io::Result<()> {
  std::fs::create_dir_all(state_dir)?;
  let path = state_dir.join(STATE_FILE);
  let text = toml::to_string_pretty(state)
    .map_err(|error| std::io::Error::other(format!("serialize state: {error}")))?;
  crate::fsutil::atomic_write_bytes(&path, text.as_bytes())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn saver_coalesces_changes_until_due() {
    let dir = std::env::temp_dir().join(format!("music-tui-state-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let start = Instant::now();
    let mut saver = StateSaver::new(&dir, PersistedState::default());
    let changed = |tab| PersistedState {
      tab,
      ..PersistedState::default()
    };

    saver.update(changed(1), start);
    let due = saver.deadline().expect("pending save");
    saver.update(changed(2), start + Duration::from_millis(500));
    assert_eq!(
      saver.deadline(),
      Some(due),
      "first change sets the deadline"
    );
    saver.flush(start + Duration::from_millis(600), false);
    assert!(!dir.join(STATE_FILE).exists(), "not due yet");

    saver.flush(due, false);
    assert_eq!(PersistedState::load(&dir), changed(2));
    assert_eq!(saver.deadline(), None);

    // Reverting to the saved state cancels a pending write.
    saver.update(changed(3), due);
    saver.update(changed(2), due);
    assert_eq!(saver.deadline(), None);
    let _ = std::fs::remove_dir_all(&dir);
  }
}
