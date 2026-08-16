//! Configuration schema: every tunable the user can set in config.toml.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::paths::expand_home;
use crate::layout;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MpdConfig {
  /// MPD host. A path starting with `/` (or `~`) connects over a unix
  /// socket — required for playing `file://` songs outside the library.
  pub host: String,
  pub port: u16,
  /// Optional MPD password.
  pub password: Option<String>,
  /// Music library root used to read cover art and lyrics files. When empty,
  /// music-tui tries to read `music_directory` from ~/.config/mpd/mpd.conf.
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
#[serde(default)]
pub struct BehaviorConfig {
  /// Interval between MPD status refreshes while idle.
  pub tick_ms: u64,
  /// Interval between MPD status refreshes while playing.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderConfig {
  pub chafa_bin: String,
  pub auto_detect: bool,
  pub chafa_args: Vec<String>,
  pub chafa_threads: usize,
  pub passthrough: Option<String>,
  pub zellij_sixel: bool,
}

impl Default for RenderConfig {
  fn default() -> Self {
    Self {
      chafa_bin: "chafa".to_string(),
      auto_detect: true,
      chafa_args: Vec::new(),
      chafa_threads: 0,
      passthrough: None,
      zellij_sixel: false,
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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
      fifo_path: "/tmp/mpd.fifo".to_string(),
      sample_rate: 44100,
      channels: 2,
      bars: 256,
      fps: 30,
      window: 2048,
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct PlaylistConfig {
  /// Directory for `:save` exports. Empty means the XDG state home
  /// (`~/.local/state/music-tui/playlists`).
  pub save_dir: String,
}

/// Column shown in the library pane: a track field plus a width weight.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryColumn {
  /// Track field: `title` / `artist` / `album` / `genre` / `filename` /
  /// `duration`.
  pub field: String,
  /// Relative width weight (columns share the width by weight).
  pub width: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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
        LibraryColumn { field: "title".to_string(), width: 4 },
        LibraryColumn { field: "artist".to_string(), width: 3 },
        LibraryColumn { field: "album".to_string(), width: 3 },
        LibraryColumn { field: "duration".to_string(), width: 1 },
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
#[serde(default)]
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

/// Top-level tab configuration. Each `[[layout.tabs]]` entry describes one
/// tab shown in the tab bar; `detail` describes the secondary detail view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutConfig {
  pub detail: String,
  pub tabs: Vec<TabConfig>,
}

impl Default for LayoutConfig {
  fn default() -> Self {
    Self::with_default_tabs()
  }
}

impl LayoutConfig {
  fn normalize_defaults(&mut self) {
    if self.detail.trim().is_empty() {
      self.detail = layout::DEFAULT_DETAIL_LAYOUT.to_string();
    }
    if self.tabs.is_empty() {
      *self = LayoutConfig::with_default_tabs();
    }
  }

  fn with_default_tabs() -> Self {
    Self {
      detail: layout::DEFAULT_DETAIL_LAYOUT.to_string(),
      tabs: vec![
        TabConfig::playlist(),
        TabConfig::library(),
        TabConfig::playing(),
        TabConfig::metadata(),
        TabConfig::lyrics(),
        TabConfig::visualizer(),
      ],
    }
  }
}

/// One tab: a layout tree plus the pane whose keymap is active.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabConfig {
  /// Name shown in the tab bar.
  pub name: String,
  /// Layout tree, e.g. `H(2:1, queue, V(2:1, cover, metadata))`.
  pub layout: String,
  /// Pane that receives this tab's keys (`queue`, `cover`, `lyrics`,
  /// `metadata`, `visualizer`). Defaults to the first pane in the tree.
  pub main: Option<String>,
}

impl TabConfig {
  fn playlist() -> Self {
    Self {
      name: "playlist".to_string(),
      layout: "H(2:1, queue, V(2:1, cover:hovered, metadata:hovered))".to_string(),
      main: Some("queue".to_string()),
    }
  }

  fn library() -> Self {
    Self {
      name: "library".to_string(),
      layout: "H(2:1, library, V(2:1, cover:library-hovered, metadata:library-hovered))"
        .to_string(),
      main: Some("library".to_string()),
    }
  }

  fn playing() -> Self {
    Self {
      name: "playing".to_string(),
      layout: "H(1:2, cover, lyrics)".to_string(),
      main: Some("cover".to_string()),
    }
  }

  fn metadata() -> Self {
    Self {
      name: "metadata".to_string(),
      layout: "metadata".to_string(),
      main: Some("metadata".to_string()),
    }
  }

  fn lyrics() -> Self {
    Self {
      name: "lyrics".to_string(),
      layout: "lyrics".to_string(),
      main: Some("lyrics".to_string()),
    }
  }

  fn visualizer() -> Self {
    Self {
      name: "visualizer".to_string(),
      layout: "visualizer".to_string(),
      main: Some("visualizer".to_string()),
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
    self.visualizer.window = self
      .visualizer
      .window
      .clamp(256, 8192)
      .next_power_of_two();
    self.layout.normalize_defaults();
  }

  pub(super) fn validate(&self) -> Result<(), String> {
    layout::parse_detail(&self.layout.detail)?;
    layout::parse_tabs(&self.layout)?;
    Ok(())
  }
}

