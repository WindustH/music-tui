use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::{MpdConfig, detect_music_dir, expand_home};

const AUDIO_EXTENSIONS: &[&str] = &[
  "flac", "mp3", "ogg", "oga", "opus", "m4a", "mp4", "aac", "wav", "wv", "ape", "dsf", "dff",
  "aif", "aiff", "mid", "mod", "it", "xm", "s3m", "sid",
];

/// Resolve the music library root: config override first, then mpd.conf.
pub fn resolve_music_dir(config: &MpdConfig) -> Result<PathBuf> {
  if let Some(dir) = configured_music_dir(config) {
    return Ok(dir);
  }
  detect_music_dir().context("music_dir is not configured and music_directory was not found in ~/.config/mpd/mpd.conf")
}

fn configured_music_dir(config: &MpdConfig) -> Option<PathBuf> {
  config
    .music_dir
    .as_deref()
    .filter(|dir| !dir.trim().is_empty())
    .map(expand_home)
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
      if recursive && !has_nomedia(&path) {
        visit_audio_files(&path, recursive, files)?;
      }
    } else if is_audio_file(&path) {
      files.push(path);
    }
  }
  Ok(())
}

/// Android convention: a `.nomedia` marker file inside a directory makes
/// recursive media scans skip that directory and everything below it.
/// An explicitly requested root is always scanned, only its children are
/// filtered, so `music-tui open <dir>` never comes back empty by accident.
pub fn has_nomedia(dir: &Path) -> bool {
  dir.join(".nomedia").exists()
}

/// True when any directory from `root` down to the track (exclusive of the
/// root itself) carries a `.nomedia` marker. Used to drop tracks from the
/// library database after a marker appears.
pub fn is_excluded_by_nomedia(root: &Path, rel: &Path) -> bool {
  let mut current = root.to_path_buf();
  for component in rel.components() {
    current.push(component);
    if has_nomedia(&current) {
      return true;
    }
  }
  false
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

/// Map an MPD uri back to a local path. `file://` uris decode without a
/// configured music directory; relative MPD uris require one.
pub fn uri_to_path(music_dir: Option<&Path>, uri: &str) -> Option<PathBuf> {
  if let Some(path) = local_uri_to_path(uri) {
    return Some(path);
  }
  let relative = uri.trim_start_matches('/');
  music_dir.map(|dir| dir.join(relative))
}

/// Decode a local URI as returned by MPD. MPD accepts `file://` over a
/// Unix socket, then normalizes it to a plain absolute path in queue data.
pub fn local_uri_to_path(uri: &str) -> Option<PathBuf> {
  file_uri_to_path(uri).or_else(|| Path::new(uri).is_absolute().then(|| PathBuf::from(uri)))
}

/// Compare MPD song URIs while accounting for its `file://` normalization.
pub fn same_song_uri(left: &str, right: &str) -> bool {
  left == right
    || local_uri_to_path(left)
      .zip(local_uri_to_path(right))
      .is_some_and(|(left, right)| left == right)
}

/// True when the host selects a UNIX socket connection — the only
/// transport MPD accepts `file://` uris on.
pub fn is_socket_host(host: &str) -> bool {
  expand_home(host).to_string_lossy().starts_with('/')
}

/// Prefix an absolute path as a `file://` uri for MPD. MPD's local-file
/// handler expects the filesystem path verbatim; it does not URL-decode
/// percent escapes.
pub fn file_uri(path: &Path) -> String {
  format!("file://{}", path.to_string_lossy())
}

/// Strip MPD's `file://` prefix back into a filesystem path.
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
  let rest = uri.strip_prefix("file://")?;
  if rest.is_empty() {
    return None;
  }
  Some(PathBuf::from(rest))
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
  // Publish atomically: create the symlink under a per-process temp name,
  // then rename it into place. A concurrent instance (or MPD reading the
  // bridge dir mid-update) never observes a missing or half-replaced link;
  // a racing writer produces the same target, so last rename wins cleanly.
  #[cfg(unix)]
  {
    let temp = dir.join(format!(".{hash}-{}.{}.tmp", name.to_string_lossy(), std::process::id()));
    let _ = std::fs::remove_file(&temp);
    std::os::unix::fs::symlink(target, &temp)
      .with_context(|| format!("failed to link {} -> {}", temp.display(), target.display()))?;
    if let Err(error) = std::fs::rename(&temp, &link) {
      let _ = std::fs::remove_file(&temp);
      // Someone else may have renamed an identical link into place already.
      if std::fs::read_link(&link).is_ok_and(|current| current == target) {
        return Ok(link);
      }
      return Err(error).with_context(|| format!("failed to publish {}", link.display()));
    }
  }
  Ok(link)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn blank_music_dir_is_not_configured() {
    let config = MpdConfig {
      music_dir: Some("  ".to_string()),
      ..MpdConfig::default()
    };
    assert_eq!(configured_music_dir(&config), None);
  }

  #[test]
  fn file_uri_resolves_without_music_dir() {
    let path = Path::new("/tmp/音乐/100% a song.flac");
    assert_eq!(file_uri(path), "file:///tmp/音乐/100% a song.flac");
    assert_eq!(uri_to_path(None, &file_uri(path)), Some(path.to_path_buf()));
    assert_eq!(uri_to_path(None, path.to_str().unwrap()), Some(path.to_path_buf()));
    assert!(same_song_uri(&file_uri(path), path.to_str().unwrap()));
  }

  #[test]
  fn relative_uri_requires_music_dir() {
    assert_eq!(uri_to_path(None, "Artist/song.flac"), None);
    assert_eq!(
      uri_to_path(Some(Path::new("/music")), "Artist/song.flac"),
      Some(PathBuf::from("/music/Artist/song.flac")),
    );
  }

  #[test]
  fn collect_audio_files_skips_nomedia_directories() {
    let dir = std::env::temp_dir().join(format!("music-tui-nomedia-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("visible/ok")).unwrap();
    std::fs::create_dir_all(dir.join("hidden/sub")).unwrap();
    std::fs::write(dir.join("hidden/.nomedia"), b"").unwrap();
    std::fs::write(dir.join("d.wav"), b"").unwrap();
    std::fs::write(dir.join("visible/a.mp3"), b"").unwrap();
    std::fs::write(dir.join("visible/ok/b.flac"), b"").unwrap();
    std::fs::write(dir.join("hidden/c.mp3"), b"").unwrap();
    std::fs::write(dir.join("hidden/sub/e.mp3"), b"").unwrap();

    let files = collect_audio_files(&dir, true).unwrap();
    let names: Vec<String> = files
      .iter()
      .map(|file| file.file_name().unwrap().to_string_lossy().into_owned())
      .collect();
    assert_eq!(names, ["d.wav", "a.mp3", "b.flac"]);

    // Non-recursive scans never descend, so markers are irrelevant.
    let flat = collect_audio_files(&dir, false).unwrap();
    assert_eq!(flat.len(), 1);
    assert!(flat[0].ends_with("d.wav"));

    // An explicitly requested root is scanned even when it carries the
    // marker itself; only its children are filtered.
    std::fs::write(dir.join(".nomedia"), b"").unwrap();
    let rooted = collect_audio_files(&dir, true).unwrap();
    let names: Vec<String> = rooted
      .iter()
      .map(|file| file.file_name().unwrap().to_string_lossy().into_owned())
      .collect();
    assert_eq!(names, ["d.wav", "a.mp3", "b.flac"]);

    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn nomedia_exclusion_covers_nested_paths() {
    let dir = std::env::temp_dir().join(format!("music-tui-nomedia-db-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a/b")).unwrap();
    std::fs::write(dir.join("a/.nomedia"), b"").unwrap();

    assert!(is_excluded_by_nomedia(&dir, Path::new("a/song.mp3")));
    assert!(is_excluded_by_nomedia(&dir, Path::new("a/b/song.mp3")));
    assert!(!is_excluded_by_nomedia(&dir, Path::new("top.mp3")));
    assert!(!is_excluded_by_nomedia(&dir, Path::new("c/other.mp3")));

    let _ = std::fs::remove_dir_all(&dir);
  }
}
