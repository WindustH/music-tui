//! Config/cache directory discovery, `~` expansion and mpd.conf parsing.

use std::path::PathBuf;

pub(super) fn app_config_dir() -> PathBuf {
  platform_config_dir().join("music-tui")
}

pub(super) fn app_cache_dir() -> PathBuf {
  platform_cache_dir().join("music-tui")
}

pub fn app_state_dir() -> PathBuf {
  platform_state_dir().join("music-tui")
}

pub use framework_tui::paths::expand_home;
pub(super) use framework_tui::paths::{
  cache_dir as platform_cache_dir, config_dir as platform_config_dir, env_path,
  state_dir as platform_state_dir,
};

/// Try to read `music_directory` from the user's mpd.conf.
pub fn detect_music_dir() -> Option<PathBuf> {
  for config_path in mpd_config_paths() {
    let Ok(body) = std::fs::read_to_string(config_path) else {
      continue;
    };
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
  }
  None
}

#[cfg(unix)]
pub(super) fn mpd_config_paths() -> Vec<PathBuf> {
  let mut paths = vec![platform_config_dir().join("mpd").join("mpd.conf")];
  if let Some(home) = env_path("HOME") {
    paths.push(home.join(".mpd").join("mpd.conf"));
    paths.push(home.join(".mpdconf"));
  }
  paths
}

#[cfg(windows)]
pub(super) fn mpd_config_paths() -> Vec<PathBuf> {
  let mut paths = vec![platform_config_dir().join("mpd").join("mpd.conf")];
  if let Some(appdata) = env_path("APPDATA") {
    paths.push(appdata.join("mpd").join("mpd.conf"));
  }
  paths
}
