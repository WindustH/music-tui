//! Configuration schema: every tunable the user can set in config.toml.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::paths::expand_home;
use crate::layout;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MpdConfig {
  /// MPD host. A path starting with `/` (or `~`) connects over a unix
  /// socket — required for playing `file://` songs outside the library.
  pub host: String,
  pub port: u16,
  /// Optional MPD password.
  pub password: Option<String>,
  /// Local music root for resolving MPD's relative song uris. When empty,
  /// music-tui tries `music_directory` from ~/.config/mpd/mpd.conf. Songs
  /// queued as `file://` over a unix socket do not require a root.
  pub music_dir: Option<String>,
  /// Directory for the symlink bridge used to queue files outside the
  /// library on TCP connections. Empty = `<music_dir>/.music-tui-links`.
  pub link_dir: String,
}

impl Default for MpdConfig {
  fn default() -> Self {
    Self {
      host: "127.0.0.1".to_string(),
      port: 6600,
      password: None,
      music_dir: None,
      link_dir: String::new(),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BehaviorConfig {
  /// Interval between MPD status polls while paused or stopped (MPD's idle
  /// notifications report changes as they happen).
  pub tick_ms: u64,
  /// Interval between MPD status polls while playing (progress, lyrics).
  pub playing_tick_ms: u64,
  /// Hide duplicate queue entries (same URL keeps its first occurrence
  /// visible; the playing copy stays visible too).
  pub queue_dedup: bool,
}

impl Default for BehaviorConfig {
  fn default() -> Self {
    Self {
      tick_ms: 1000,
      playing_tick_ms: 200,
      queue_dedup: true,
    }
  }
}

impl BehaviorConfig {
  /// MPD status-poll periods for the idle and playing states
  /// (idle, playing), clamped so degenerate config values cannot spin the
  /// worker or stall the UI.
  pub fn refresh_durations(&self) -> (Duration, Duration) {
    (
      Duration::from_millis(self.tick_ms.clamp(100, 10_000)),
      Duration::from_millis(self.playing_tick_ms.clamp(100, 10_000)),
    )
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RenderConfig {
  pub raw_memory_cache_max_bytes: u64,
  #[serde(alias = "cache_max_bytes")]
  pub disk_cache_max_bytes: u64,
  pub chafa_bin: String,
  pub auto_detect: bool,
  pub chafa_args: Vec<String>,
  pub chafa_threads: usize,
  /// Graphics escape passthrough override (`tmux` / `screen` / `none`);
  /// unset detects the terminal multiplexer.
  pub passthrough: Option<String>,
  pub zellij_sixel: framework_tui::config::Toggle,
}

impl Default for RenderConfig {
  fn default() -> Self {
    Self {
      raw_memory_cache_max_bytes: 64 * 1024 * 1024,
      disk_cache_max_bytes: 512 * 1024 * 1024,
      chafa_bin: "chafa".to_string(),
      auto_detect: true,
      chafa_args: Vec::new(),
      chafa_threads: 0,
      passthrough: None,
      zellij_sixel: framework_tui::config::Toggle::Off,
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VisualizerConfig {
  /// MPD fifo output path, e.g. the `path` of an `audio_output { type "fifo" }` block.
  pub fifo_path: String,
  pub sample_rate: u32,
  pub channels: u16,
  /// Maximum band count; the analysis follows the visualizer pane width
  /// (one band per column) up to this cap.
  pub bars: usize,
  /// Target updates per second for the spectrum analysis.
  pub fps: u32,
  /// FFT window size in samples.
  pub window: usize,
}

impl Default for VisualizerConfig {
  fn default() -> Self {
    Self {
      #[cfg(unix)]
      fifo_path: "/tmp/mpd.fifo".to_string(),
      #[cfg(windows)]
      fifo_path: {
        let mut p = std::env::temp_dir();
        p.push("mpd.fifo");
        p.to_string_lossy().into_owned()
      },
      sample_rate: 44100,
      channels: 2,
      bars: 256,
      fps: 30,
      window: 2048,
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[derive(Default)]
pub struct PlaylistConfig {
  /// Directory for `:save` exports. Empty means the XDG state home
  /// (`~/.local/state/music-tui/playlists`).
  pub save_dir: String,
}

/// Column shown in the library pane: a track field plus a width weight.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryColumn {
  /// Track field: `title` / `artist` / `album` / `genre` / `filename` /
  /// `duration`.
  pub field: String,
  /// Relative width weight (columns share the width by weight).
  pub width: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LibraryConfig {
  /// Music source directories for the library database (music-tui scans
  /// and indexes these itself; they may differ from MPD's music dir).
  pub paths: Vec<String>,
  /// Columns shown in the library pane, in order.
  pub columns: Vec<LibraryColumn>,
  /// Scan subdirectories recursively (the common case).
  pub recursive: bool,
}

impl Default for LibraryConfig {
  fn default() -> Self {
    Self {
      paths: Vec::new(),
      columns: vec![
        LibraryColumn {
          field: "title".to_string(),
          width: 4,
        },
        LibraryColumn {
          field: "artist".to_string(),
          width: 3,
        },
        LibraryColumn {
          field: "album".to_string(),
          width: 3,
        },
        LibraryColumn {
          field: "duration".to_string(),
          width: 1,
        },
      ],
      recursive: true,
    }
  }
}

impl PlaylistConfig {
  /// Effective `:save` directory (`~` expanded; fallback: state home).
  pub fn effective_save_dir(&self) -> PathBuf {
    let configured = self.save_dir.trim();
    if configured.is_empty() {
      crate::playlist::default_save_dir()
    } else {
      expand_home(configured)
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LyricsConfig {
  /// Extra directories searched for `<artist> - <title>.lrc` files.
  pub extra_dirs: Vec<String>,
  /// Follow playback when synced lyrics are available.
  pub follow: bool,
}

impl Default for LyricsConfig {
  fn default() -> Self {
    Self {
      extra_dirs: Vec::new(),
      follow: true,
    }
  }
}

impl super::AppConfig {
  pub(super) fn normalize_defaults(&mut self) {
    if self.behavior.tick_ms == 0 {
      self.behavior.tick_ms = 1000;
    }
    if self.behavior.playing_tick_ms == 0 {
      self.behavior.playing_tick_ms = 200;
    }
    if self.visualizer.bars == 0 {
      self.visualizer.bars = 256;
    }
    if self.visualizer.fps == 0 {
      self.visualizer.fps = 30;
    }
    if self.visualizer.sample_rate == 0 {
      self.visualizer.sample_rate = 44100;
    }
    if self.visualizer.channels == 0 {
      self.visualizer.channels = 2;
    }
    self.visualizer.channels = self.visualizer.channels.clamp(1, 8);
    self.visualizer.window = self.visualizer.window.clamp(256, 8192).next_power_of_two();
    for column in &mut self.library.columns {
      if column.width == 0 {
        column.width = 1;
      }
    }
    self.layout.normalize_defaults();
  }

  pub(super) fn validate(&self) -> Result<(), String> {
    layout::parse_detail(&self.layout.detail)?;
    layout::parse_tabs(&self.layout)?;
    Ok(())
  }
}
