//! Playlist file support: parse `m3u`/`m3u8`/`pls` and plain-text path
//! lists for the `open` subcommand, and write m3u exports for `:save`.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistKind {
  M3u,
  Pls,
  Text,
}

/// Detect a playlist by extension. `m3u`/`m3u8` and `pls` are the common
/// playlist formats; `.txt` is treated as a plain list of song paths.
pub fn playlist_kind(path: &Path) -> Option<PlaylistKind> {
  match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
    "m3u" | "m3u8" => Some(PlaylistKind::M3u),
    "pls" => Some(PlaylistKind::Pls),
    "txt" => Some(PlaylistKind::Text),
    _ => None,
  }
}

/// Parse a playlist file into entries. Relative lines resolve against the
/// playlist's own directory; `#` comments and blank lines are skipped.
pub fn parse_playlist(path: &Path) -> Result<Vec<PathBuf>, String> {
  let body =
    std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
  let base = path.parent().unwrap_or_else(|| Path::new("."));
  let entries = match playlist_kind(path) {
    Some(PlaylistKind::Pls) => body
      .lines()
      .filter_map(pls_entry)
      .map(|line| resolve_entry(base, line))
      .collect(),
    _ => body
      .lines()
      .map(str::trim)
      .filter(|line| !line.is_empty() && !line.starts_with('#'))
      .map(|line| resolve_entry(base, line))
      .collect(),
  };
  Ok(entries)
}

/// `FileN=value` entries of a PLS file, in file order.
fn pls_entry(line: &str) -> Option<&str> {
  let (key, value) = line.split_once('=')?;
  let key = key.trim();
  if key.len() >= 5 && key[..4].eq_ignore_ascii_case("file") && key[4..].chars().all(|c| c.is_ascii_digit())
  {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
  } else {
    None
  }
}

fn resolve_entry(base: &Path, line: &str) -> PathBuf {
  let expanded = crate::config::expand_home(line);
  if expanded.is_absolute() {
    expanded
  } else {
    base.join(expanded)
  }
}

/// Default `:save` directory: the XDG state home (`~/.local/state`).
pub fn default_save_dir() -> PathBuf {
  let state = std::env::var_os("XDG_STATE_HOME")
    .map(PathBuf::from)
    .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
    .unwrap_or_else(|| PathBuf::from(".local/state"));
  state.join("music-tui").join("playlists")
}

/// Decide where `:save` writes.
///
/// - no argument: `<save_dir>/music-tui-<unix-seconds>.m3u`
/// - bare file name: `<save_dir>/<name>` (`.m3u` appended when no extension)
/// - absolute path (after `~` expansion): used verbatim
/// - anything else (a relative path with directories): rejected
pub fn resolve_save_path(arg: Option<&str>, save_dir: &Path) -> Result<PathBuf, String> {
  let Some(arg) = arg else {
    let stamp = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .map(|since| since.as_secs())
      .unwrap_or(0);
    return Ok(save_dir.join(format!("music-tui-{stamp}.m3u")));
  };
  let expanded = crate::config::expand_home(arg.trim());
  if expanded.is_absolute() {
    return Ok(expanded);
  }
  if expanded.components().count() > 1 {
    return Err(format!(
      "relative paths are not allowed; use an absolute path or a bare file name (saved under {})",
      save_dir.display()
    ));
  }
  let name = expanded;
  let with_ext = if name.extension().is_none() {
    name.with_extension("m3u")
  } else {
    name
  };
  Ok(save_dir.join(with_ext))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn write_tmp(name: &str, body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("music-tui-playlist-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    path
  }

  /// Drop the shared scratch dir once the last fixture is gone; harmless
  /// (no-op) while other fixtures still exist.
  fn cleanup_tmp() {
    let dir = std::env::temp_dir().join(format!("music-tui-playlist-{}", std::process::id()));
    let _ = std::fs::remove_dir(&dir);
  }

  #[test]
  fn parses_m3u_with_comments_and_relative_entries() {
    let absolute = std::env::temp_dir().join("music-tui-absolute.mp3");
    let path = write_tmp(
      "list.m3u",
      &format!(
        "#EXTM3U\n#EXTINF:123,Artist - Title\n../songs/a.flac\n{}\n\n\n",
        absolute.display(),
      ),
    );
    let entries = parse_playlist(&path).unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries[0].ends_with("../songs/a.flac"));
    assert_eq!(entries[1], absolute);
    let _ = std::fs::remove_file(&path);
    cleanup_tmp();
  }

  #[test]
  fn parses_pls_file_entries_only() {
    let absolute = std::env::temp_dir().join("music-tui-absolute.flac");
    let path = write_tmp(
      "list.pls",
      &format!(
        "[playlist]\nFile1=one.ogg\nTitle1=x\nFile2={}\nLength1=10\n",
        absolute.display(),
      ),
    );
    let entries = parse_playlist(&path).unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries[0].ends_with("one.ogg"));
    assert_eq!(entries[1], absolute);
    let _ = std::fs::remove_file(&path);
    cleanup_tmp();
  }

  #[test]
  fn parses_plain_text_path_list() {
    let path = write_tmp("paths.txt", "# comment\nsong1.mp3\n~/music/song2.flac\n");
    let entries = parse_playlist(&path).unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries[1].to_string_lossy().contains("music/song2.flac"));
    let _ = std::fs::remove_file(&path);
    cleanup_tmp();
  }

  #[test]
  fn save_path_defaults_to_generated_name_in_save_dir() {
    let dir = Path::new("/state/playlists");
    let resolved = resolve_save_path(None, dir).unwrap();
    assert!(resolved.starts_with(dir));
    assert!(resolved
      .file_name()
      .unwrap()
      .to_string_lossy()
      .starts_with("music-tui-"));
    assert_eq!(resolved.extension().unwrap(), "m3u");
  }

  #[test]
  fn save_path_completes_bare_filenames() {
    let dir = Path::new("/state/playlists");
    assert_eq!(
      resolve_save_path(Some("favorites"), dir).unwrap(),
      dir.join("favorites.m3u")
    );
    assert_eq!(
      resolve_save_path(Some("favorites.m3u8"), dir).unwrap(),
      dir.join("favorites.m3u8")
    );
  }

  #[test]
  fn save_path_rejects_relative_paths() {
    let dir = std::env::temp_dir().join("music-tui-playlists");
    assert!(resolve_save_path(Some("sub/queue.m3u"), &dir).is_err());
    assert!(resolve_save_path(Some("../queue.m3u"), &dir).is_err());
    let absolute = std::env::temp_dir().join("music-tui-queue.m3u");
    assert_eq!(
      resolve_save_path(Some(absolute.to_str().unwrap()), &dir).unwrap(),
      absolute,
    );
    cleanup_tmp();
  }
}
