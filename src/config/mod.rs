//! Configuration loading: `~/.config/music-tui/{config,keymap,theme}.toml`
//! with commented defaults, `.bak` backups on incompatible rewrites and
//! missing-field normalization.

use std::{
  path::{Path, PathBuf},
  time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::fs;


pub use crate::keymap::KeymapConfig;
pub use crate::theme::ThemeConfig;

use crate::keymap::format_keymap_toml;

mod comments;
mod paths;
mod schema;

pub use comments::app_config_toml;
pub use paths::{detect_music_dir, expand_home};
use paths::{app_cache_dir, app_config_dir};
pub use schema::{
  BehaviorConfig, LayoutConfig, LibraryColumn, LibraryConfig, LyricsConfig, MpdConfig,
  PlaylistConfig, RenderConfig, TabConfig, VisualizerConfig,
};

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
  pub library: LibraryConfig,
  pub layout: LayoutConfig,
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
  let theme =
    read_or_write_theme_default(&config_dir.join("theme.toml"), ThemeConfig::default()).await?;

  Ok(Settings {
    config,
    keymap,
    theme,
    cache_dir,
  })
}
async fn write_theme_default(path: &Path, default: ThemeConfig) -> Result<ThemeConfig> {
  let body = crate::theme::format_theme_toml(&default);
  write_bytes_atomic(path, body.as_bytes())
    .await
    .with_context(|| format!("failed to write {}", path.display()))?;
  Ok(default)
}

async fn backup_and_write_theme_default(path: &Path, default: ThemeConfig) -> Result<ThemeConfig> {
  backup_config_file(path).await?;
  write_theme_default(path, default).await
}

async fn read_or_write_theme_default(path: &Path, default: ThemeConfig) -> Result<ThemeConfig> {
  if !path.exists() {
    return write_theme_default(path, default).await;
  }
  let body = fs::read_to_string(path)
    .await
    .with_context(|| format!("failed to read {}", path.display()))?;
  let mut parsed: ThemeConfig = match toml::from_str(&body) {
    Ok(parsed) => parsed,
    Err(_) => return backup_and_write_theme_default(path, default).await,
  };
  parsed.normalize_defaults();
  let normalized = crate::theme::format_theme_toml(&parsed);
  write_back_if_toml_changed(path, &body, &normalized).await?;
  Ok(parsed)
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
    if self.which_key.columns == 0 {
      self.which_key.columns = 3;
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

