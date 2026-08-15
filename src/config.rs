use std::{
  collections::BTreeSet,
  env,
  fmt::Write as FmtWrite,
  path::{Path, PathBuf},
  time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::layout;

pub use crate::keymap::KeymapConfig;
pub use crate::theme::ThemeConfig;

use crate::keymap::format_keymap_toml;

#[derive(Debug, Clone)]
pub struct Settings {
  pub config: AppConfig,
  pub keymap: KeymapConfig,
  pub theme: ThemeConfig,
  pub cache_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AppConfig {
  pub mpd: MpdConfig,
  pub behavior: BehaviorConfig,
  pub render: RenderConfig,
  pub visualizer: VisualizerConfig,
  pub lyrics: LyricsConfig,
  pub playlist: PlaylistConfig,
  pub layout: LayoutConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MpdConfig {
  /// MPD host. A path starting with `/` connects over a unix socket.
  pub host: String,
  pub port: u16,
  /// Optional MPD password.
  pub password: Option<String>,
  /// Music library root used to read cover art and lyrics files. When empty,
  /// music-tui tries to read `music_directory` from ~/.config/mpd/mpd.conf.
  pub music_dir: Option<String>,
}

impl Default for MpdConfig {
  fn default() -> Self {
    Self {
      host: "127.0.0.1".to_string(),
      port: 6600,
      password: None,
      music_dir: None,
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
}

impl Default for BehaviorConfig {
  fn default() -> Self {
    Self {
      tick_ms: 1000,
      playing_tick_ms: 200,
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
  /// Maximum bar count. The analysis follows the visualizer pane width
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
pub struct PlaylistConfig {
  /// Directory for `:save` exports. Empty means the XDG state home
  /// (`~/.local/state/music-tui/playlists`).
  pub save_dir: String,
}

impl Default for PlaylistConfig {
  fn default() -> Self {
    Self {
      save_dir: String::new(),
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
      layout: "H(2:1, queue, V(2:1, cover, metadata))".to_string(),
      main: Some("queue".to_string()),
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

impl AppConfig {
  fn normalize_defaults(&mut self) {
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

  fn validate(&self) -> Result<(), String> {
    layout::parse_detail(&self.layout.detail)?;
    layout::parse_tabs(&self.layout)?;
    Ok(())
  }
}

pub async fn load_or_create() -> Result<Settings> {
  let config_dir = app_config_dir();
  let cache_dir = app_cache_dir();

  fs::create_dir_all(&config_dir)
    .await
    .with_context(|| format!("failed to create {}", config_dir.display()))?;
  fs::create_dir_all(&cache_dir)
    .await
    .with_context(|| format!("failed to create {}", cache_dir.display()))?;

  let config_path = config_dir.join("config.toml");
  let config = read_or_write_default(&config_path, AppConfig::default()).await?;
  let keymap =
    read_or_write_keymap_default(&config_dir.join("keymap.toml"), KeymapConfig::default()).await?;
  let theme = read_or_write_default(&config_dir.join("theme.toml"), ThemeConfig::default()).await?;

  Ok(Settings {
    config,
    keymap,
    theme,
    cache_dir,
  })
}

fn app_config_dir() -> PathBuf {
  platform_config_dir().join("music-tui")
}

fn app_cache_dir() -> PathBuf {
  platform_cache_dir().join("music-tui")
}

fn platform_config_dir() -> PathBuf {
  env_path("XDG_CONFIG_HOME")
    .or_else(|| env_path("HOME").map(|home| home.join(".config")))
    .unwrap_or_else(|| PathBuf::from(".config"))
}

fn platform_cache_dir() -> PathBuf {
  env_path("XDG_CACHE_HOME")
    .or_else(|| env_path("HOME").map(|home| home.join(".cache")))
    .unwrap_or_else(|| PathBuf::from(".cache"))
}

fn env_path(name: &str) -> Option<PathBuf> {
  env::var_os(name)
    .filter(|value| !value.is_empty())
    .map(PathBuf::from)
}

async fn read_or_write_keymap_default(path: &Path, default: KeymapConfig) -> Result<KeymapConfig> {
  if !path.exists() {
    return write_keymap_default(path, default).await;
  }
  let body = fs::read_to_string(path)
    .await
    .with_context(|| format!("failed to read {}", path.display()))?;
  let mut parsed: KeymapConfig = match toml::from_str(&body) {
    Ok(parsed) => parsed,
    Err(_) => return backup_and_write_keymap_default(path, default).await,
  };
  parsed.normalize_defaults();
  let normalized = format_keymap_toml(&parsed);
  write_back_if_toml_changed(path, &body, &normalized).await?;
  Ok(parsed)
}

async fn read_or_write_default<T>(path: &Path, default: T) -> Result<T>
where
  T: Serialize + for<'de> Deserialize<'de> + Clone + NormalizeConfigDefaults,
{
  if !path.exists() {
    return write_default_config(path, default).await;
  }
  let body = fs::read_to_string(path)
    .await
    .with_context(|| format!("failed to read {}", path.display()))?;
  let mut parsed: T = match toml::from_str(&body) {
    Ok(parsed) => parsed,
    Err(_) => return backup_and_write_default_config(path, default).await,
  };
  parsed.normalize_defaults();
  if parsed.validate().is_err() {
    return backup_and_write_default_config(path, default).await;
  }
  let normalized = parsed.to_config_toml()?;
  write_back_if_toml_changed(path, &body, &normalized).await?;
  Ok(parsed)
}

trait NormalizeConfigDefaults {
  fn normalize_defaults(&mut self);

  fn validate(&self) -> Result<(), String> {
    Ok(())
  }

  fn to_config_toml(&self) -> Result<String>
  where
    Self: Serialize + Sized,
  {
    toml::to_string_pretty(self).map_err(Into::into)
  }
}

impl NormalizeConfigDefaults for AppConfig {
  fn normalize_defaults(&mut self) {
    AppConfig::normalize_defaults(self);
  }

  fn validate(&self) -> Result<(), String> {
    AppConfig::validate(self)
  }

  fn to_config_toml(&self) -> Result<String> {
    app_config_toml(self)
  }
}

impl NormalizeConfigDefaults for ThemeConfig {
  fn normalize_defaults(&mut self) {
    if self.which_key_columns == 0 {
      self.which_key_columns = 3;
    }
  }
}

async fn write_keymap_default(path: &Path, default: KeymapConfig) -> Result<KeymapConfig> {
  let body = format_keymap_toml(&default);
  write_bytes_atomic(path, body.as_bytes())
    .await
    .with_context(|| format!("failed to write {}", path.display()))?;
  Ok(default)
}

async fn backup_and_write_keymap_default(
  path: &Path,
  default: KeymapConfig,
) -> Result<KeymapConfig> {
  backup_config_file(path).await?;
  write_keymap_default(path, default).await
}

async fn write_default_config<T>(path: &Path, mut default: T) -> Result<T>
where
  T: Serialize + NormalizeConfigDefaults,
{
  default.normalize_defaults();
  let body = default.to_config_toml()?;
  write_bytes_atomic(path, body.as_bytes())
    .await
    .with_context(|| format!("failed to write {}", path.display()))?;
  Ok(default)
}

async fn backup_and_write_default_config<T>(path: &Path, default: T) -> Result<T>
where
  T: Serialize + NormalizeConfigDefaults,
{
  backup_config_file(path).await?;
  write_default_config(path, default).await
}

async fn backup_config_file(path: &Path) -> Result<PathBuf> {
  let backup_path = next_backup_path(path);
  fs::rename(path, &backup_path).await.with_context(|| {
    format!(
      "failed to back up incompatible config {} to {}",
      path.display(),
      backup_path.display()
    )
  })?;
  Ok(backup_path)
}

fn next_backup_path(path: &Path) -> PathBuf {
  let file_name = path
    .file_name()
    .and_then(|name| name.to_str())
    .unwrap_or("config.toml");
  let stamp = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs();
  for index in 0..1000 {
    let suffix = if index == 0 {
      format!(".bak.{stamp}")
    } else {
      format!(".bak.{stamp}.{index}")
    };
    let candidate = path.with_file_name(format!("{file_name}{suffix}"));
    if !candidate.exists() {
      return candidate;
    }
  }
  path.with_file_name(format!("{file_name}.bak.{stamp}.overflow"))
}

async fn write_back_if_toml_changed(path: &Path, original: &str, normalized: &str) -> Result<()> {
  if toml_semantic_value(original) != toml_semantic_value(normalized) {
    write_bytes_atomic(path, normalized.as_bytes())
      .await
      .with_context(|| format!("failed to update {}", path.display()))?;
  }
  Ok(())
}

fn toml_semantic_value(body: &str) -> Option<toml::Value> {
  toml::from_str(body).ok()
}

pub async fn write_bytes_atomic(path: &Path, body: &[u8]) -> Result<()> {
  let temp = path.with_extension("tmp");
  fs::write(&temp, body)
    .await
    .with_context(|| format!("failed to write {}", temp.display()))?;
  fs::rename(&temp, path)
    .await
    .with_context(|| format!("failed to rename {} to {}", temp.display(), path.display()))?;
  Ok(())
}

pub fn config_comment(key: &str) -> Option<&'static str> {
  match key {
    "mpd" => Some("Connection settings for the MPD daemon."),
    "mpd.host" => Some("MPD host. A path starting with / connects over a unix socket."),
    "mpd.port" => Some("MPD TCP port."),
    "mpd.password" => Some("Optional MPD password."),
    "mpd.music_dir" => Some(
      "Music library root for cover art and lyrics files. Empty reads music_directory from mpd.conf.",
    ),
    "behavior" => Some("Interactive behavior settings."),
    "behavior.tick_ms" => Some("Status refresh interval while idle."),
    "behavior.playing_tick_ms" => Some("Status refresh interval while playing."),
    "render" => Some("Cover art rendering settings."),
    "render.chafa_bin" => Some("Command used to render cover art when no graphics protocol is available."),
    "render.auto_detect" => Some("Detect terminal graphics capability automatically."),
    "render.chafa_args" => Some("Extra arguments passed to Chafa."),
    "render.chafa_threads" => Some("Threads requested per Chafa render job."),
    "render.passthrough" => Some("Optional Chafa passthrough mode, such as tmux."),
    "render.zellij_sixel" => Some("Zellij SIXEL handling mode."),
    "visualizer" => Some("Spectrum visualizer settings."),
    "visualizer.fifo_path" => Some("MPD fifo output path feeding the visualizer."),
    "visualizer.sample_rate" => Some("Sample rate of the fifo audio_output format."),
    "visualizer.channels" => Some("Channel count of the fifo audio_output format."),
    "visualizer.bars" => Some("Maximum bar count; the spectrum follows the pane width (one band per column) up to this cap."),
    "visualizer.fps" => Some("Spectrum analysis updates per second."),
    "visualizer.window" => Some("FFT window size in samples."),
    "lyrics" => Some("Lyrics loading settings."),
    "lyrics.extra_dirs" => Some("Extra directories searched for `<song>.lrc` and `<artist> - <title>.lrc` files."),
    "lyrics.follow" => Some("Follow playback when synced lyrics are available."),
    "playlist" => Some("Playlist file handling (`:save`, `open` on .m3u/.pls/.txt files)."),
    "playlist.save_dir" => Some("Directory for `:save` exports; empty uses ~/.local/state/music-tui/playlists. Bare `:save` names resolve here."),
    "layout" => Some("Tab layout. Each tab is a layout tree like H(2:1, queue, V(2:1, cover, metadata)) with a main pane that receives its keys."),
    "layout.detail" => Some("Secondary detail view (i) layout over the cover and metadata panes, e.g. H(2:1, cover, metadata)."),
    "layout.tabs" => Some("Tabs shown in the tab bar, switched with left/right."),
    _ => None,
  }
}

pub fn app_config_toml(config: &AppConfig) -> Result<String> {
  let body = toml::to_string_pretty(config)?;
  Ok(add_app_config_comments(
    &body,
    &[
      "music-tui main configuration.",
      "Missing fields are rewritten with defaults when the app loads this file.",
    ],
    config_comment,
  ))
}

fn add_app_config_comments(
  body: &str,
  header: &[&str],
  comment_for: fn(&str) -> Option<&'static str>,
) -> String {
  let mut out = String::new();
  let mut seen_comments = BTreeSet::new();
  for line in header {
    push_toml_comment(&mut out, line);
  }
  out.push('\n');

  let mut table = String::new();
  for line in body.lines() {
    let trimmed = line.trim();
    if let Some(header) = toml_table_header(trimmed) {
      table = header.to_string();
      if seen_comments.insert(table.clone())
        && let Some(comment) = comment_for(&table)
      {
        push_toml_comment(&mut out, comment);
      }
    } else if let Some(key) = toml_field_key(trimmed) {
      let comment_key = if table.is_empty() {
        key.to_string()
      } else {
        format!("{table}.{key}")
      };
      if seen_comments.insert(comment_key.clone())
        && let Some(comment) = comment_for(&comment_key)
      {
        push_toml_comment(&mut out, comment);
      }
    }
    out.push_str(line);
    out.push('\n');
  }
  out
}

fn push_toml_comment(out: &mut String, comment: &str) {
  for line in comment.lines() {
    let _ = writeln!(out, "# {line}");
  }
}

fn toml_table_header(line: &str) -> Option<&str> {
  if line.starts_with("[[") {
    return None;
  }
  line.strip_prefix('[')?.strip_suffix(']')
}

fn toml_field_key(line: &str) -> Option<&str> {
  if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
    return None;
  }
  let (key, _) = line.split_once('=')?;
  let key = key.trim();
  (!key.is_empty()).then_some(key)
}

/// Expand a leading `~` in a path using $HOME.
pub fn expand_home(path: &str) -> PathBuf {
  if let Some(rest) = path.strip_prefix("~/")
    && let Some(home) = env_path("HOME") {
      return home.join(rest);
    }
  PathBuf::from(path)
}

/// Try to read `music_directory` from the user's mpd.conf.
pub fn detect_music_dir() -> Option<PathBuf> {
  let config_path = platform_config_dir().join("mpd").join("mpd.conf");
  let body = std::fs::read_to_string(config_path).ok()?;
  for line in body.lines() {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix("music_directory") {
      let rest = rest.trim_start();
      if let Some(value) = rest.strip_prefix('"')
        && let Some(value) = value.strip_suffix('"')
      {
        return Some(expand_home(value));
      }
    }
  }
  None
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn app_config_toml_writes_parseable_commented_defaults() {
    let body = app_config_toml(&AppConfig::default()).expect("default config should serialize");
    toml::from_str::<AppConfig>(&body).expect("commented default config should parse");
    assert!(body.contains("# music-tui main configuration."));
    assert!(body.contains("# Connection settings for the MPD daemon."));
  }
}
