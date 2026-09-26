//! Local music library database: a small SQLite store under the state
//! directory, synced from the configured `[library]` roots by a scanner
//! thread. Scanning lives in [`scan`], field matching in [`filter`].

mod filter;
mod scan;

pub use filter::{TrackField, TrackMatch, all_rows, filter_tracks};

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;
use tokio::sync::mpsc;

use crate::{
  config::LibraryConfig,
  event::{AsyncEvent, LibraryEvent},
};

/// Start the scanner thread: one scan right away, then one per rescan
/// request (`u`). Returns the rescan sender, or `None` when no library
/// paths are configured. The thread exits once the app drops the sender.
pub fn spawn_scanner(
  library: &LibraryConfig,
  state_dir: &Path,
  events: mpsc::UnboundedSender<AsyncEvent>,
) -> Option<std::sync::mpsc::Sender<()>> {
  if library.paths.is_empty() {
    return None;
  }
  let library = library.clone();
  let db_path = state_dir.join("library.db");
  let (scan_tx, scan_rx) = std::sync::mpsc::channel::<()>();
  let spawned = std::thread::Builder::new()
    .name("music-tui-library".to_string())
    .spawn(move || {
      let send = |event: LibraryEvent| {
        let _ = events.send(AsyncEvent::Library(event));
      };
      loop {
        let mut progress = |scanned: usize, changed: usize| {
          send(LibraryEvent::Scanning { scanned, changed });
        };
        match scan_and_load(&db_path, &library, &mut progress) {
          Ok(tracks) => send(LibraryEvent::Loaded(tracks)),
          Err(error) => send(LibraryEvent::Failed {
            error: format!("{error:#}"),
            tracks: open_db(&db_path)
              .and_then(|connection| all_tracks(&connection))
              .ok(),
          }),
        }
        if scan_rx.recv().is_err() {
          break;
        }
        // Requests that piled up during the scan are satisfied by one more.
        while scan_rx.try_recv().is_ok() {}
      }
    });
  match spawned {
    Ok(_) => Some(scan_tx),
    Err(error) => {
      tracing::warn!(%error, "failed to start the library scanner");
      None
    }
  }
}

fn scan_and_load(
  db_path: &Path,
  library: &LibraryConfig,
  progress: &mut dyn FnMut(usize, usize),
) -> Result<Vec<LibraryTrack>> {
  let mut connection = open_db(db_path)?;
  sync_roots(&connection, library)?;
  scan::scan_roots(&mut connection, library, progress)?;
  all_tracks(&connection)
}

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

/// Bump when the derivation logic changes so cached rows rescan.
const LIBRARY_DB_VERSION: i64 = 1;

/// Open (creating if needed) the library database.
fn open_db(db_path: &Path) -> Result<Connection> {
  if let Some(parent) = db_path.parent() {
    std::fs::create_dir_all(parent)
      .with_context(|| format!("failed to create {}", parent.display()))?;
  }
  let connection =
    Connection::open(db_path).with_context(|| format!("failed to open {}", db_path.display()))?;
  connection.execute_batch(
    r#"
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;
    PRAGMA busy_timeout = 5000;
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

fn all_tracks(connection: &Connection) -> Result<Vec<LibraryTrack>> {
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

/// Canonical, expanded root paths from the config, in a stable form.
/// `sync_roots` inserts exactly these into the `roots` table, so the same
/// list is used to detect roots the user removed from the config.
fn configured_root_paths(config: &LibraryConfig) -> Vec<String> {
  let mut paths = Vec::new();
  for path in &config.paths {
    let expanded = crate::config::expand_home(path);
    let Ok(canonical) = expanded.canonicalize() else {
      continue;
    };
    paths.push(canonical.to_string_lossy().to_string());
  }
  paths
}

fn sync_roots(connection: &Connection, config: &LibraryConfig) -> Result<()> {
  for text in configured_root_paths(config) {
    connection.execute(
      "INSERT OR IGNORE INTO roots (path) VALUES (?1)",
      [text.as_str()],
    )?;
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn scan(db: &Path, config: &LibraryConfig) -> Vec<LibraryTrack> {
    scan_and_load(db, config, &mut |_, _| {}).expect("scan succeeds")
  }

  #[test]
  fn rescans_drop_vanished_and_nomedia_tracks() {
    let root = std::env::temp_dir().join(format!("music-tui-libdb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let music = root.join("music");
    std::fs::create_dir_all(music.join("x")).unwrap();
    std::fs::create_dir_all(music.join("y")).unwrap();
    std::fs::write(music.join("x/01. Artist - Song A.mp3"), b"").unwrap();
    std::fs::write(music.join("y/b.mp3"), b"").unwrap();
    let db = root.join("library.db");
    let config = LibraryConfig {
      paths: vec![music.to_string_lossy().into_owned()],
      ..LibraryConfig::default()
    };

    let tracks = scan(&db, &config);
    assert_eq!(tracks.len(), 2);
    let untagged = tracks.iter().find(|track| track.title == "Song A").unwrap();
    assert_eq!(
      untagged.artist, "Artist",
      "artist derived from the file name"
    );

    std::fs::write(music.join("y/.nomedia"), b"").unwrap();
    assert_eq!(scan(&db, &config).len(), 1, "nomedia folder dropped");

    std::fs::remove_file(music.join("x/01. Artist - Song A.mp3")).unwrap();
    assert!(scan(&db, &config).is_empty(), "vanished file dropped");

    let unconfigured = LibraryConfig {
      paths: Vec::new(),
      ..LibraryConfig::default()
    };
    std::fs::write(music.join("x/c.mp3"), b"").unwrap();
    assert_eq!(scan(&db, &config).len(), 1);
    assert!(scan(&db, &unconfigured).is_empty(), "removed root dropped");

    let _ = std::fs::remove_dir_all(&root);
  }
}
