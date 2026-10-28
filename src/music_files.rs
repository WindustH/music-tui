//! Local music paths, MPD URI conversion and the external-file link bridge.
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::{MpdConfig, detect_music_dir};

const AUDIO_EXTENSIONS: &[&str] = &[
  "flac", "mp3", "ogg", "oga", "opus", "m4a", "mp4", "aac", "wav", "wv", "ape", "dsf", "dff",
  "aif", "aiff", "mid", "mod", "it", "xm", "s3m", "sid",
];

/// Resolve the music library root: config override first, then mpd.conf.
pub fn resolve_music_dir(config: &MpdConfig) -> Result<PathBuf> {
  if let Some(dir) = configured_music_dir(config) {
    return Ok(dir);
  }
  detect_music_dir().context(
    "music_dir is not configured and music_directory was not found in ~/.config/mpd/mpd.conf",
  )
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
  let mut visited = HashSet::new();
  visit_audio_files(root, recursive, &mut visited, &mut files)?;
  files.sort();
  Ok(files)
}

/// Recursively collect audio files under `dir`. Directory cycles are cut by
/// tracking every resolved directory once: a junction or symlink that points
/// back towards an ancestor resolves to a path already in the set, so the
/// scan always terminates even when the tree contains a loop (a Windows
/// junction into a parent directory otherwise recurses forever). The same
/// bookkeeping skips directories reachable through two links, collecting
/// each physical directory's files exactly once.
///
/// Only an unreadable `dir` itself is an error: unreadable subdirectories
/// (a root-owned `lost+found` on a music drive, a permission-locked
/// folder) are skipped so they cannot hide the rest of the tree.
fn visit_audio_files(
  dir: &Path,
  recursive: bool,
  visited: &mut HashSet<PathBuf>,
  files: &mut Vec<PathBuf>,
) -> Result<()> {
  if !visited.insert(canonical_or_self(dir)) {
    return Ok(());
  }
  let entries = std::fs::read_dir(dir)
    .with_context(|| format!("failed to read directory {}", dir.display()))?;
  for entry in entries.flatten() {
    let path = entry.path();
    if entry.file_name().to_string_lossy().starts_with('.') {
      continue;
    }
    let Ok(file_type) = entry.file_type() else {
      continue;
    };
    if file_type.is_dir() {
      if recursive
        && !has_nomedia(&path)
        && let Err(error) = visit_audio_files(&path, recursive, visited, files)
      {
        tracing::warn!("skipping {}: {error:#}", path.display());
      }
    } else if is_audio_file(&path) {
      files.push(path);
    }
  }
  Ok(())
}

/// Identity used to detect re-entering the same directory through a junction
/// or symlink: the fully resolved path when available, otherwise the path as
/// given. `canonicalize` follows link targets, so a link pointing at an
/// ancestor produces the ancestor's (already recorded) path.
fn canonical_or_self(path: &Path) -> PathBuf {
  std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Android convention: a `.nomedia` marker file inside a directory makes
/// recursive media scans skip that directory and everything below it.
/// An explicitly requested root is always scanned, only its children are
/// filtered, so `music-tui open <dir>` never comes back empty by accident.
pub fn has_nomedia(dir: &Path) -> bool {
  dir.join(".nomedia").exists()
}

/// Convert an absolute library path to an MPD uri relative to the music dir.
pub fn path_to_uri(music_dir: &Path, path: &Path) -> Result<String> {
  let relative = path.strip_prefix(music_dir).with_context(|| {
    format!(
      "{} is outside the music directory {}",
      path.display(),
      music_dir.display()
    )
  })?;
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
  let dir = music_dir?;
  if !relative_stays_within(relative) {
    return None;
  }
  Some(dir.join(relative))
}

/// True when a uri (`Album/song.flac`) stays inside its root: `a/../b`
/// stays and is permitted, but a `..` that would climb above the root (or
/// the whole music dir) is rejected so a hostile uri cannot read arbitrary
/// files outside the library.
fn relative_stays_within(relative: &str) -> bool {
  let mut depth = 0usize;
  for component in relative.split(['/', '\\']) {
    match component {
      "" | "." => {}
      ".." => {
        if depth == 0 {
          return false;
        }
        depth -= 1;
      }
      _ => depth += 1,
    }
  }
  true
}

/// Decode a local URI as returned by MPD. MPD accepts `file://` over a
/// Unix socket, then normalizes it to a plain absolute path in queue data.
pub fn local_uri_to_path(uri: &str) -> Option<PathBuf> {
  file_uri_to_path(uri).or_else(|| Path::new(uri).is_absolute().then(|| PathBuf::from(uri)))
}

/// Identity of an MPD song URI for duplicate detection. MPD normalizes
/// `file://` URIs to plain absolute paths in queue data, so local files
/// compare by path and everything else by the raw URI. Two URIs name the
/// same song exactly when their keys are equal, which lets dedup checks
/// use hash lookups instead of pairwise comparisons.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SongKey {
  Local(PathBuf),
  Uri(String),
}

pub fn song_key(uri: &str) -> SongKey {
  match local_uri_to_path(uri) {
    Some(path) => SongKey::Local(path),
    None => SongKey::Uri(uri.to_string()),
  }
}

/// True when the host selects a UNIX socket connection — the only
/// transport MPD accepts `file://` uris on.
pub fn is_socket_host(_host: &str) -> bool {
  #[cfg(unix)]
  {
    expand_home(_host).to_string_lossy().starts_with('/')
  }
  #[cfg(not(unix))]
  {
    false
  }
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

/// Ensure a bridge entry under `dir` for `target` (reused when current).
/// Unix uses a symlink; Windows uses an atomically published cached copy.
pub fn ensure_link(dir: &Path, target: &Path) -> Result<PathBuf> {
  use sha2::{Digest, Sha256};
  std::fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
  let mut hasher = Sha256::new();
  hasher.update(target.to_string_lossy().as_bytes());
  let hash = hex::encode(&hasher.finalize()[..8]);
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
    let temp = dir.join(format!(
      ".{hash}-{}.{}.tmp",
      name.to_string_lossy(),
      std::process::id()
    ));
    let _ = std::fs::remove_file(&temp);
    std::os::unix::fs::symlink(target, &temp)
      .with_context(|| format!("failed to link {} -> {}", temp.display(), target.display()))?;
    if let Err(error) = crate::fsutil::replace_path(&temp, &link) {
      let _ = std::fs::remove_file(&temp);
      // Someone else may have renamed an identical link into place already.
      if std::fs::read_link(&link).is_ok_and(|current| current == target) {
        return Ok(link);
      }
      return Err(error).with_context(|| format!("failed to publish {}", link.display()));
    }
  }
  #[cfg(windows)]
  {
    // Windows symlinks require admin or developer mode, so publish a copy.
    // Comparing modification time as well as size prevents stale same-size
    // files from being reused after the source is edited.
    if same_file_version(&link, target) {
      return Ok(link);
    }

    let temp = dir.join(format!(
      ".{hash}-{}.{}.tmp",
      name.to_string_lossy(),
      std::process::id(),
    ));
    let _ = std::fs::remove_file(&temp);

    // Retry once when the source changes while it is being copied. The
    // stable bridge name is replaced only after a complete copy exists.
    for attempt in 0..2 {
      std::fs::copy(target, &temp)
        .with_context(|| format!("failed to copy {} -> {}", target.display(), temp.display()))?;
      if !same_file_version(&temp, target) {
        let _ = std::fs::remove_file(&temp);
        if attempt == 0 {
          continue;
        }
        bail!("{} changed while it was being copied", target.display());
      }

      if let Err(error) = crate::fsutil::replace_path(&temp, &link) {
        let _ = std::fs::remove_file(&temp);
        // A concurrent instance may have published the same version first.
        if same_file_version(&link, target) {
          return Ok(link);
        }
        return Err(error).with_context(|| format!("failed to publish {}", link.display()));
      }
      if same_file_version(&link, target) {
        return Ok(link);
      }
      if attempt == 0 {
        continue;
      }
      bail!(
        "{} changed while its bridge copy was published",
        target.display()
      );
    }
  }
  Ok(link)
}

#[cfg(windows)]
fn same_file_version(left: &Path, right: &Path) -> bool {
  std::fs::metadata(left)
    .ok()
    .zip(std::fs::metadata(right).ok())
    .is_some_and(|(left, right)| {
      left.len() == right.len()
        && left
          .modified()
          .ok()
          .zip(right.modified().ok())
          .is_some_and(|(left, right)| left == right)
    })
}

use framework_tui::paths::expand_home;
