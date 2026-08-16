//! Library database: a SQLite index of the configured music directories.
//!
//! The scan is incremental: files are re-read only when their mtime
//! changes. Lyrics text is indexed for full-field filtering, but is never
//! rendered in the list (columns show short fields only).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::config::LibraryConfig;

/// One indexed track.
#[derive(Debug, Clone, Default)]
pub struct LibraryTrack {
  /// Row id (unused by the UI, kept for db round-trips).
  #[allow(dead_code)]
  pub id: i64,
  pub path: PathBuf,
  pub title: String,
  pub artist: String,
  pub album: String,
  pub genre: String,
  /// File name without extension (`夜的第七章`, not `....wav`).
  pub filename: String,
  pub duration_secs: f64,
  pub lyrics: String,
  /// File mtime in seconds (scan bookkeeping).
  #[allow(dead_code)]
  pub mtime: u64,
}

/// Open (creating if needed) the library database.
/// Bump when the derivation logic changes so cached rows rescan.
const LIBRARY_DB_VERSION: i64 = 1;

pub fn open_db(db_path: &Path) -> Result<Connection> {
  if let Some(parent) = db_path.parent() {
    std::fs::create_dir_all(parent)
      .with_context(|| format!("failed to create {}", parent.display()))?;
  }
  let connection = Connection::open(db_path)
    .with_context(|| format!("failed to open {}", db_path.display()))?;
  connection.execute_batch(
    r#"
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;
    CREATE TABLE IF NOT EXISTS tracks (
      id INTEGER PRIMARY KEY,
      root_id INTEGER NOT NULL,
      rel_path TEXT NOT NULL,
      title TEXT NOT NULL DEFAULT '',
      artist TEXT NOT NULL DEFAULT '',
      album TEXT NOT NULL DEFAULT '',
      genre TEXT NOT NULL DEFAULT '',
      filename TEXT NOT NULL DEFAULT '',
      duration_secs REAL NOT NULL DEFAULT 0.0,
      lyrics TEXT NOT NULL DEFAULT '',
      mtime INTEGER NOT NULL DEFAULT 0,
      UNIQUE(root_id, rel_path)
    );
    CREATE INDEX IF NOT EXISTS tracks_root ON tracks(root_id);
    CREATE TABLE IF NOT EXISTS roots (
      id INTEGER PRIMARY KEY,
      path TEXT NOT NULL UNIQUE
    );
    "#,
  )?;
  let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
  if version != LIBRARY_DB_VERSION {
    // Older rows were derived with different fallback logic; rescan them.
    connection.execute("DELETE FROM tracks", [])?;
    connection.pragma_update(None, "user_version", LIBRARY_DB_VERSION)?;
  }
  Ok(connection)
}

/// Incremental scan of all configured roots. `progress` is called with
/// (scanned, changed) counters every ~200 files so the UI can show
/// progress without flooding the event loop.
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
        read_lyrics_text(&file)
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

/// Delete files that vanished from disk.
fn drop_missing(connection: &Connection, roots: &[(i64, String)]) -> Result<()> {
  for (root_id, root_path) in roots {
    let root = PathBuf::from(root_path);
    let vanished: Vec<i64> = {
      let mut statement =
        connection.prepare("SELECT id, rel_path FROM tracks WHERE root_id = ?1")?;
      
      statement
        .query_map([root_id], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?
        .filter_map(|row| row.ok())
        .filter(|(_, rel)| !root.join(rel).exists())
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

/// All tracks ordered by artist, then album, then title.
pub fn all_tracks(connection: &Connection) -> Result<Vec<LibraryTrack>> {
  let mut statement = connection.prepare(
    "SELECT t.id, r.path || '/' || t.rel_path, t.title, t.artist, t.album, t.genre,
            t.filename, t.duration_secs, t.lyrics, t.mtime
       FROM tracks t JOIN roots r ON r.id = t.root_id
      ORDER BY t.artist, t.album, t.title",
  )?;
  let tracks = statement
    .query_map([], |row| {
      Ok(LibraryTrack {
        id: row.get(0)?,
        path: PathBuf::from(row.get::<_, String>(1)?),
        title: row.get(2)?,
        artist: row.get(3)?,
        album: row.get(4)?,
        genre: row.get(5)?,
        filename: row.get(6)?,
        duration_secs: row.get(7)?,
        lyrics: row.get(8)?,
        mtime: row.get::<_, i64>(9)? as u64,
      })
    })?
    .collect::<std::result::Result<Vec<_>, _>>()?;
  Ok(tracks)
}

/// Register configured roots (creating missing rows).
pub fn sync_roots(connection: &Connection, config: &LibraryConfig) -> Result<()> {
  for path in &config.paths {
    let expanded = crate::config::expand_home(path);
    let Ok(canonical) = expanded.canonicalize() else {
      continue;
    };
    let text = canonical.to_string_lossy().to_string();
    connection.execute(
      "INSERT OR IGNORE INTO roots (path) VALUES (?1)",
      [text.as_str()],
    )?;
  }
  Ok(())
}

fn walk(root: &Path, recursive: bool) -> Vec<PathBuf> {
  let mut out = Vec::new();
  let mut stack = vec![root.to_path_buf()];
  while let Some(dir) = stack.pop() {
    let Ok(entries) = std::fs::read_dir(&dir) else {
      continue;
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
      let path = entry.path();
      let Ok(file_type) = entry.file_type() else { continue };
      if file_type.is_dir() {
        if recursive && !entry.file_name().to_string_lossy().starts_with('.') {
          stack.push(path);
        }
      } else if crate::library::is_audio_file(&path) {
        files.push(path);
      }
    }
    files.sort();
    out.extend(files);
  }
  // stack-based walk yields deepest-first per dir; sort the whole set for
  // a stable artist/album-ish order
  out.sort();
  out
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
    lyrics: String::new(),
    mtime: 0,
  };
  Some(track)
}

/// Sidecar/embedded lyrics as one lowercase blob for filtering.
fn read_lyrics_text(path: &Path) -> String {
  use lofty::prelude::*;
  let mut text = String::new();
  let lrc = path.with_extension("lrc");
  if let Ok(body) = std::fs::read_to_string(&lrc) {
    text.push_str(&body);
  }
  if let Ok(tagged) = lofty::probe::Probe::open(path).map(|probe| probe.read())
    && let Ok(tagged) = tagged
      && let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag())
        && let Some(lyrics) = tag.get_string(&lofty::tag::ItemKey::Lyrics) {
          text.push_str(lyrics);
        }
  text.to_lowercase()
}

/// Track field for filter/match purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackField {
  Title,
  Artist,
  Album,
  Genre,
  Filename,
  Lyrics,
}

impl TrackField {
  pub fn parse(value: &str) -> Option<Self> {
    match value.trim() {
      "title" => Some(Self::Title),
      "artist" => Some(Self::Artist),
      "album" => Some(Self::Album),
      "genre" => Some(Self::Genre),
      "filename" => Some(Self::Filename),
      "lyrics" => Some(Self::Lyrics),
      _ => None,
    }
  }

  pub fn text(self, track: &LibraryTrack) -> &str {
    match self {
      Self::Title => &track.title,
      Self::Artist => &track.artist,
      Self::Album => &track.album,
      Self::Genre => &track.genre,
      Self::Filename => &track.filename,
      Self::Lyrics => &track.lyrics,
    }
  }

  /// Priority rank for ordering matches: lower sorts first.
  fn rank(self) -> u8 {
    match self {
      Self::Title => 0,
      Self::Artist => 1,
      Self::Album => 2,
      Self::Filename => 3,
      Self::Genre => 4,
      Self::Lyrics => 5,
    }
  }
}

/// A matched track plus the field that produced the best
/// (highest-priority) term-0 match; used for result ordering.
#[derive(Debug, Clone)]
pub struct TrackMatch {
  pub track: LibraryTrack,
  /// Field holding the best match.
  pub field: TrackField,
}

/// Filter tracks by `query` over every field. The query is split on
/// whitespace; every term must match somewhere (AND) with spaces inside
/// the field text ignored, and the reported field/range is the term-0
/// match with the best priority (in original-text byte coordinates).
pub fn filter_tracks(tracks: &[LibraryTrack], query: &str) -> Vec<TrackMatch> {
  let terms: Vec<String> = query.split_whitespace().map(str::to_string).collect();
  let mut out = Vec::new();
  for track in tracks {
    let fields: Vec<(TrackField, crate::strip::StrippedText)> = [
      TrackField::Title,
      TrackField::Artist,
      TrackField::Album,
      TrackField::Genre,
      TrackField::Filename,
      TrackField::Lyrics,
    ]
    .iter()
    .map(|field| (*field, crate::strip::StrippedText::new(field.text(track))))
    .collect();
    let mut best: Option<(TrackField, (usize, usize))> = None;
    let mut all_terms_match = true;
    for (index, term) in terms.iter().enumerate() {
      let mut term_match: Option<(TrackField, (usize, usize))> = None;
      for (field, text) in &fields {
        if let Some(range) = text.find_all(term).first().copied() {
          let candidate = (*field, range);
          term_match = Some(match term_match {
            None => candidate,
            Some(current) if field.rank() < current.0.rank() => candidate,
            Some(current) => current,
          });
        }
      }
      match term_match {
        Some(found) => {
          if index == 0 {
            best = Some(found);
          }
        }
        None => {
          all_terms_match = false;
          break;
        }
      }
    }
    if all_terms_match {
      let (field, _range) = best.unwrap_or((TrackField::Title, (0, 0)));
      out.push(TrackMatch {
        track: track.clone(),
        field,
      });
    }
  }
  out.sort_by(|a, b| {
    a.field
      .rank()
      .cmp(&b.field.rank())
      .then_with(|| a.track.artist.cmp(&b.track.artist))
      .then_with(|| a.track.title.cmp(&b.track.title))
  });
  out
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

  fn track(title: &str, artist: &str, lyrics: &str) -> LibraryTrack {
    LibraryTrack {
      title: title.to_string(),
      artist: artist.to_string(),
      lyrics: lyrics.to_string(),
      ..LibraryTrack::default()
    }
  }

  #[test]
  fn filter_matches_all_terms_and_picks_priority_field() {
    let tracks = vec![
      track("夜的第七章", "周杰伦", "夜曲不停写"),
      track("以父之名", "周杰伦", ""),
    ];
    let hits = filter_tracks(&tracks, "夜曲");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].field, TrackField::Lyrics);
  }

  #[test]
  fn filter_ranks_title_over_album() {
    let tracks = vec![
      track("album-hit", "a", ""),   // title match
      track("song", "b", "album-hit in lyrics"),
    ];
    let hits = filter_tracks(&tracks, "album-hit");
    assert_eq!(hits[0].field, TrackField::Title);
    assert_eq!(hits[1].field, TrackField::Lyrics);
  }

  #[test]
  fn filter_requires_every_term() {
    let tracks = vec![track("夜的第七章", "周杰伦", "")];
    assert_eq!(filter_tracks(&tracks, "夜 不存在").len(), 0);
    assert_eq!(filter_tracks(&tracks, "夜 第七").len(), 1);
  }

}

#[allow(dead_code)]
fn _unused(system_time: SystemTime) -> SystemTime {
  system_time
}
