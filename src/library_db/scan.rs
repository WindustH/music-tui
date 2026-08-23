//! Library scanning: walk the configured roots, read tags with lofty,
//! and upsert tracks into SQLite (mtime-based incremental sync).

use std::{
  path::{Path, PathBuf},
  time::UNIX_EPOCH,
};

use anyhow::Result;
use rusqlite::Connection;

use super::LibraryTrack;
use crate::config::LibraryConfig;

pub fn scan_roots(
  connection: &mut Connection,
  config: &LibraryConfig,
  progress: &mut dyn FnMut(usize, usize),
) -> Result<()> {
  let roots: Vec<(i64, String)> = {
    let mut statement = connection.prepare("SELECT id, path FROM roots")?;
    
    statement
      .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
      .collect::<std::result::Result<Vec<_>, _>>()?
  };

  let mut scanned = 0usize;
  let mut changed = 0usize;
  for (root_id, root_path) in &roots {
    let Ok(root) = PathBuf::from(&root_path).canonicalize() else {
      continue;
    };
    for file in walk(&root, config.recursive) {
      scanned += 1;
      if scanned.is_multiple_of(200) {
        progress(scanned, changed);
      }
      let rel = match file.strip_prefix(&root) {
        Ok(rel) => rel.to_string_lossy().to_string(),
        Err(_) => continue,
      };
      let Ok(metadata) = std::fs::metadata(&file) else {
        continue;
      };
      let mtime = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
      let known: Option<(i64, u64)> = {
        let mut statement = connection
          .prepare("SELECT id, mtime FROM tracks WHERE root_id = ?1 AND rel_path = ?2")?;
        statement
          .query_row((root_id, rel.as_str()), |row| {
            Ok((row.get(0)?, row.get::<_, i64>(1)? as u64))
          })
          .ok()
      };
      if known.is_some_and(|(_, known_mtime)| known_mtime == mtime) {
        continue;
      }
      let track = read_track(&file).unwrap_or_else(|| LibraryTrack {
        id: 0,
        path: file.clone(),
        title: String::new(),
        artist: String::new(),
        album: String::new(),
        genre: String::new(),
        filename: file
          .file_stem()
          .map(|stem| stem.to_string_lossy().to_string())
          .unwrap_or_default(),
        duration_secs: 0.0,
        lyrics: String::new(),
        mtime,
      });
      let lyrics = if track.lyrics.is_empty() {
        read_sidecar_lyrics(&file)
      } else {
        track.lyrics
      };
      // Untagged files still follow the usual "NN. artist - title"
      // filename convention; derive artist/title from the stem.
      let (derived_artist, derived_title) = derive_from_filename(&track.filename);
      let artist = if track.artist.is_empty() {
        derived_artist
      } else {
        track.artist
      };
      let title = if track.title.is_empty() {
        derived_title
      } else {
        track.title
      };
      if let Some((id, _)) = known {
        connection.execute(
          "UPDATE tracks SET title=?1, artist=?2, album=?3, genre=?4, filename=?5,
             duration_secs=?6, lyrics=?7, mtime=?8 WHERE id=?9",
          rusqlite::params![
            title, artist, track.album, track.genre, track.filename,
            track.duration_secs, lyrics, mtime as i64, id
          ],
        )?;
      } else {
        connection.execute(
          "INSERT INTO tracks (root_id, rel_path, title, artist, album, genre, filename,
             duration_secs, lyrics, mtime) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
          rusqlite::params![
            root_id, rel, title, artist, track.album, track.genre, track.filename,
            track.duration_secs, lyrics, mtime as i64
          ],
        )?;
      }
      changed += 1;
    }
  }
  // Drop tracks of roots no longer configured and vanished files.
  drop_missing(connection, &roots)?;
  progress(scanned, changed);
  Ok(())
}

fn drop_missing(connection: &Connection, roots: &[(i64, String)]) -> Result<()> {
  for (root_id, root_path) in roots {
    let root = PathBuf::from(root_path);
    let vanished: Vec<i64> = {
      let mut statement =
        connection.prepare("SELECT id, rel_path FROM tracks WHERE root_id = ?1")?;
      
      statement
        .query_map([root_id], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?
        .filter_map(|row| row.ok())
        .filter(|(_, rel)| {
        let path = root.join(rel);
        !path.exists()
          || crate::library::is_excluded_by_nomedia(&root, Path::new(rel))
      })
        .map(|(id, _)| id)
        .collect()
    };
    for id in vanished {
      connection.execute("DELETE FROM tracks WHERE id = ?1", [id])?;
    }
  }
  let configured: Vec<String> = roots.iter().map(|(_, path)| path.clone()).collect();
  let stale: Vec<i64> = {
    let mut statement = connection.prepare("SELECT id, path FROM roots")?;
    
    statement
      .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?
      .filter_map(|row| row.ok())
      .filter(|(_, path)| !configured.contains(path))
      .map(|(id, _)| id)
      .collect()
  };
  for id in stale {
    connection.execute("DELETE FROM tracks WHERE root_id = ?1", [id])?;
    connection.execute("DELETE FROM roots WHERE id = ?1", [id])?;
  }
  Ok(())
}

fn walk(root: &Path, recursive: bool) -> Vec<PathBuf> {
  // Scanning tolerates unreadable directories (treated as empty); open's
  // collect_audio_files still surfaces the error to the user.
  crate::library::collect_audio_files(root, recursive).unwrap_or_default()
}

/// Split a filename stem like `2. ARForest - Your Way` into
/// `(artist, title)` for untagged files. Leading track numbers
/// (`2. `, `03 - `, `7_`) are dropped; the first ` - ` separates artist
/// and title. Stems without a separator yield an empty artist.
fn derive_from_filename(stem: &str) -> (String, String) {
  let mut rest = stem.trim();
  // Strip a leading track number: 1-3 digits followed by a separator run.
  let digits = rest.chars().take_while(char::is_ascii_digit).count();
  if (1..=3).contains(&digits) {
    let after_digits = &rest[digits..];
    let separators = after_digits
      .chars()
      .take_while(|ch| matches!(ch, ' ' | '.' | '-' | '_'))
      .count();
    if separators > 0 {
      rest = after_digits[separators..].trim_start();
    }
  }
  match rest.split_once(" - ") {
    Some((artist, title)) => {
      let artist = artist.trim();
      let title = title.trim();
      if artist.is_empty() || title.is_empty() {
        (String::new(), rest.to_string())
      } else {
        (artist.to_string(), title.to_string())
      }
    }
    None => (String::new(), rest.to_string()),
  }
}

/// Read tags with lofty; None means the file could not be read at all.
fn read_track(path: &Path) -> Option<LibraryTrack> {
  use lofty::prelude::*;
  let tagged = lofty::probe::Probe::open(path).ok()?.read().ok()?;
  let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
  let properties = tagged.properties();
  let track = LibraryTrack {
    id: 0,
    path: path.to_path_buf(),
    title: tag.title().unwrap_or_default().trim().to_string(),
    artist: tag
      .artist()
      .map(|artist| artist.to_string())
      .unwrap_or_default(),
    album: tag.album().unwrap_or_default().trim().to_string(),
    genre: tag.genre().unwrap_or_default().trim().to_string(),
    filename: path
      .file_stem()
      .map(|stem| stem.to_string_lossy().to_string())
      .unwrap_or_default(),
    duration_secs: properties.duration().as_secs_f64(),
    lyrics: tag
      .get_string(&lofty::tag::ItemKey::Lyrics)
      .unwrap_or_default()
      .to_lowercase(),
    mtime: 0,
  };
  Some(track)
}

/// Sidecar lyrics as a lowercase blob for filtering; embedded lyrics are
/// already read by `read_track` (one lofty probe per file).
fn read_sidecar_lyrics(path: &Path) -> String {
  std::fs::read_to_string(path.with_extension("lrc"))
    .unwrap_or_default()
    .to_lowercase()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn derive_from_filename_splits_artist_title() {
    assert_eq!(
      derive_from_filename("2. ARForest - Your Way(credits)"),
      ("ARForest".to_string(), "Your Way(credits)".to_string())
    );
    assert_eq!(
      derive_from_filename("03 - Taylor Swift - Mine"),
      ("Taylor Swift".to_string(), "Mine".to_string())
    );
    // No separator: keep the stem as the title, artist stays empty.
    assert_eq!(
      derive_from_filename("夏末递归定义"),
      (String::new(), "夏末递归定义".to_string())
    );
    // Track number is part of the title when there is no separator.
    assert_eq!(
      derive_from_filename("7. Intro"),
      (String::new(), "Intro".to_string())
    );
  }
}
