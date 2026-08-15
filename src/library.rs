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
/// `file://` uris (songs outside the library) decode to their own path.
pub fn uri_to_path(music_dir: &Path, uri: &str) -> PathBuf {
  if let Some(path) = file_uri_to_path(uri) {
    return path;
  }
  let relative = uri.trim_start_matches('/');
  music_dir.join(relative)
}

/// True when the host selects a UNIX socket connection — the only
/// transport MPD accepts `file://` uris on.
pub fn is_socket_host(host: &str) -> bool {
  expand_home(host).to_string_lossy().starts_with('/')
}

/// Percent-encode an absolute path as a `file://` uri for MPD.
pub fn file_uri(path: &Path) -> String {
  let mut uri = String::from("file://");
  for byte in path.to_string_lossy().bytes() {
    match byte {
      b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
        uri.push(byte as char)
      }
      _ => uri.push_str(&format!("%{byte:02X}")),
    }
  }
  uri
}

/// Decode a `file://` uri back into a filesystem path.
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
  let rest = uri.strip_prefix("file://")?;
  if rest.is_empty() {
    return None;
  }
  let mut path = String::with_capacity(rest.len());
  let mut bytes = rest.bytes();
  while let Some(byte) = bytes.next() {
    if byte == b'%' {
      let high = bytes.next()?;
      let low = bytes.next()?;
      let value = u8::from_str_radix(&format!("{}{}", high as char, low as char), 16).ok()?;
      path.push(value as char);
    } else {
      path.push(byte as char);
    }
  }
  Some(PathBuf::from(path))
}

/// Directory holding the symlink bridge for files outside the library.
pub fn links_dir(music_dir: &Path, configured: &str) -> PathBuf {
  let trimmed = configured.trim();
  if trimmed.is_empty() {
    music_dir.join(".music-tui-links")
  } else {
    expand_home(trimmed)
  }
}

/// Ensure a symlink under `dir` pointing at `target` (reused when intact);
/// returns the link path. MPD follows outside symlinks by default.
pub fn ensure_link(dir: &Path, target: &Path) -> Result<PathBuf> {
  use sha2::{Digest, Sha256};
  std::fs::create_dir_all(dir)
    .with_context(|| format!("failed to create {}", dir.display()))?;
  let mut hasher = Sha256::new();
  hasher.update(target.to_string_lossy().as_bytes());
  let hash = hex::encode(&hasher.finalize()[..4]);
  let name = target.file_name().unwrap_or_default();
  let link = dir.join(format!("{hash}-{}", name.to_string_lossy()));
  if std::fs::read_link(&link).is_ok_and(|current| current == target) {
    return Ok(link);
  }
  let _ = std::fs::remove_file(&link);
  #[cfg(unix)]
  std::os::unix::fs::symlink(target, &link)
    .with_context(|| format!("failed to link {} -> {}", link.display(), target.display()))?;
  Ok(link)
}
