//! Config/cache directory discovery, `~` expansion and mpd.conf parsing.

use std::{env, path::PathBuf};

pub(super) fn app_config_dir() -> PathBuf {
  platform_config_dir().join("music-tui")
}

pub(super) fn app_cache_dir() -> PathBuf {
  platform_cache_dir().join("music-tui")
}

pub(super) fn app_state_dir() -> PathBuf {
  platform_state_dir().join("music-tui")
}

fn platform_state_dir() -> PathBuf {
  env_path("XDG_STATE_HOME")
    .or_else(|| env_path("HOME").map(|home| home.join(".local/state")))
    .unwrap_or_else(|| PathBuf::from(".local/state"))
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
