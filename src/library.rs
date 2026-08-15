use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::{MpdConfig, detect_music_dir, expand_home};

const AUDIO_EXTENSIONS: &[&str] = &[
  "flac", "mp3", "ogg", "oga", "opus", "m4a", "mp4", "aac", "wav", "wv", "ape", "dsf", "dff",
  "aif", "aiff", "mid", "mod", "it", "xm", "s3m", "sid",
];

/// Resolve the music library root: config override first, then mpd.conf.
pub fn resolve_music_dir(config: &MpdConfig) -> Result<PathBuf> {
  if let Some(dir) = &config.music_dir {
    return Ok(expand_home(dir));
  }
  detect_music_dir().context("music_dir is not configured and music_directory was not found in ~/.config/mpd/mpd.conf")
}

pub fn is_audio_file(path: &Path) -> bool {
  path
    .extension()
    .and_then(|ext| ext.to_str())
    .is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Collect audio files under `root`, sorted by path.
pub fn collect_audio_files(root: &Path, recursive: bool) -> Result<Vec<PathBuf>> {
  let mut files = Vec::new();
  visit_audio_files(root, recursive, &mut files)?;
  files.sort();
  Ok(files)
}

fn visit_audio_files(dir: &Path, recursive: bool, files: &mut Vec<PathBuf>) -> Result<()> {
  let entries = std::fs::read_dir(dir)
    .with_context(|| format!("failed to read directory {}", dir.display()))?;
  for entry in entries {
    let entry = entry.with_context(|| format!("failed to read entry in {}", dir.display()))?;
    let path = entry.path();
    let name = entry.file_name();
    let name = name.to_string_lossy();
    if name.starts_with('.') {
      continue;
    }
    let file_type = entry.file_type().context("failed to read file type")?;
    if file_type.is_dir() {
      if recursive {
        visit_audio_files(&path, recursive, files)?;
      }
    } else if is_audio_file(&path) {
      files.push(path);
    }
  }
  Ok(())
}

/// Convert an absolute library path to an MPD uri relative to the music dir.
pub fn path_to_uri(music_dir: &Path, path: &Path) -> Result<String> {
  let relative = path
    .strip_prefix(music_dir)
    .with_context(|| format!("{} is outside the music directory {}", path.display(), music_dir.display()))?;
  if relative.as_os_str().is_empty() {
    bail!("cannot play the music directory itself");
  }
  Ok(relative.to_string_lossy().replace('\\', "/"))
}

/// Map an MPD uri back to an absolute path inside the library.
pub fn uri_to_path(music_dir: &Path, uri: &str) -> PathBuf {
  music_dir.join(uri)
}
