//! Local music library database: a small SQLite store under the cache
//! directory, synced from the configured `[library]` roots. Scanning
//! lives in [`scan`], field matching in [`filter`].

mod filter;
mod scan;

pub use filter::{TrackField, TrackMatch, filter_tracks};
pub use scan::scan_roots;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::config::LibraryConfig;

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
