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
      let title = if track.title.is_empty() {
        track.filename.clone()
      } else {
        track.title
      };
      if let Some((id, _)) = known {
        connection.execute(
          "UPDATE tracks SET title=?1, artist=?2, album=?3, genre=?4, filename=?5,
             duration_secs=?6, lyrics=?7, mtime=?8 WHERE id=?9",
          rusqlite::params![
            title, track.artist, track.album, track.genre, track.filename,
            track.duration_secs, lyrics, mtime as i64, id
          ],
        )?;
      } else {
        connection.execute(
          "INSERT INTO tracks (root_id, rel_path, title, artist, album, genre, filename,
             duration_secs, lyrics, mtime) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
          rusqlite::params![
            root_id, rel, title, track.artist, track.album, track.genre, track.filename,
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

/// A matched track: the field that produced the best (highest-priority)
/// match and where in that field the keyword sits.
#[derive(Debug, Clone)]
pub struct TrackMatch {
  pub track: LibraryTrack,
  /// Field holding the best match.
  pub field: TrackField,
  /// Byte range of the first match in that field.
  pub range: (usize, usize),
}

/// Filter tracks by `query` over every field. The query is split on
/// whitespace; every term must match somewhere (AND), and the reported
/// field/range is the term-0 match with the best priority.
pub fn filter_tracks(tracks: Vec<LibraryTrack>, query: &str) -> Vec<TrackMatch> {
  let terms: Vec<String> = query
    .split_whitespace()
    .map(str::to_lowercase)
    .collect();
  let mut out = Vec::new();
  for track in tracks {
    let lower: Vec<(TrackField, String)> = [
      TrackField::Title,
      TrackField::Artist,
      TrackField::Album,
      TrackField::Genre,
      TrackField::Filename,
      TrackField::Lyrics,
    ]
    .iter()
    .map(|field| (*field, field.text(&track).to_lowercase()))
    .collect();
    let mut best: Option<(TrackField, (usize, usize))> = None;
    let mut all_terms_match = true;
    for term in &terms {
      let mut term_match: Option<(TrackField, (usize, usize))> = None;
      for (field, text) in &lower {
        if let Some(at) = text.find(term.as_str()) {
          let end = at + term.len();
          let candidate = (*field, (at, end));
          term_match = Some(match term_match {
            None => candidate,
            Some(current) if field.rank() < current.0.rank() => candidate,
            Some(current) => current,
          });
        }
      }
      match term_match {
        Some(found) => {
          if std::ptr::eq(term, &terms[0]) {
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
      let (field, range) = best.unwrap_or((TrackField::Title, (0, 0)));
      out.push(TrackMatch { track, field, range });
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

/// Byte range of the first `needle` occurrence, for highlighting.
#[allow(dead_code)]
pub fn highlight_range(haystack: &str, needle: &str) -> Option<(usize, usize)> {
  if needle.is_empty() {
    return None;
  }
  haystack
    .to_lowercase()
    .find(needle.to_lowercase().as_str())
    .map(|at| (at, at + needle.len()))
}

#[cfg(test)]
mod tests {
  use super::*;

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
    let hits = filter_tracks(tracks, "夜曲");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].field, TrackField::Lyrics);
    assert_eq!(hits[0].range, (0, "夜曲".len()));
  }

  #[test]
  fn filter_ranks_title_over_album() {
    let tracks = vec![
      track("album-hit", "a", ""),   // title match
      track("song", "b", "album-hit in lyrics"),
    ];
    let hits = filter_tracks(tracks, "album-hit");
    assert_eq!(hits[0].field, TrackField::Title);
    assert_eq!(hits[1].field, TrackField::Lyrics);
  }

  #[test]
  fn filter_requires_every_term() {
    let tracks = vec![track("夜的第七章", "周杰伦", "")];
    assert_eq!(filter_tracks(tracks.clone(), "夜 不存在").len(), 0);
    assert_eq!(filter_tracks(tracks, "夜 第七").len(), 1);
  }

  #[test]
  fn highlight_range_is_case_insensitive() {
    assert_eq!(highlight_range("Hello World", "world"), Some((6, 11)));
    assert_eq!(highlight_range("你好", "好"), Some((3, 6)));
  }
}

#[allow(dead_code)]
fn _unused(system_time: SystemTime) -> SystemTime {
  system_time
}
