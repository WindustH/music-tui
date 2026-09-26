//! Library scanning: walk the configured roots, read tags with lofty,
//! and upsert tracks into SQLite (mtime-based incremental sync).

use std::{
  collections::{HashMap, HashSet},
  path::Path,
  time::UNIX_EPOCH,
};

use anyhow::Result;
use rusqlite::{Connection, Transaction};
use tracing::{debug, warn};

use super::LibraryTrack;
use crate::config::LibraryConfig;

/// Changed tracks written per transaction. Committing in batches keeps the
/// write lock short (a concurrent instance's scan waits on the busy
/// timeout instead of failing) and makes a first scan of a large library
/// resumable: quitting halfway keeps everything indexed so far.
const COMMIT_BATCH: usize = 200;

/// Progress is reported every this many files.
const PROGRESS_EVERY: usize = 200;

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
  let configured = super::configured_root_paths(config);

  let mut counts = ScanCounts::default();
  for (root_id, root_path) in &roots {
    if configured.contains(root_path) {
      scan_root(
        connection,
        *root_id,
        Path::new(root_path),
        config,
        &mut counts,
        progress,
      )?;
    }
  }

  // Drop roots no longer configured (or no longer resolvable) with their
  // tracks.
  let transaction = connection.transaction()?;
  for (root_id, root_path) in &roots {
    if !configured.contains(root_path) {
      transaction.execute("DELETE FROM tracks WHERE root_id = ?1", [root_id])?;
      transaction.execute("DELETE FROM roots WHERE id = ?1", [root_id])?;
    }
  }
  transaction.commit()?;
  progress(counts.scanned, counts.changed);
  Ok(())
}

#[derive(Default)]
struct ScanCounts {
  scanned: usize,
  changed: usize,
}

/// Sync one root: upsert new or modified files, then delete rows whose
/// file vanished, moved under a `.nomedia` marker, or became unreadable.
fn scan_root(
  connection: &mut Connection,
  root_id: i64,
  root: &Path,
  config: &LibraryConfig,
  counts: &mut ScanCounts,
  progress: &mut dyn FnMut(usize, usize),
) -> Result<()> {
  let files = match crate::library::collect_audio_files(root, config.recursive) {
    Ok(files) => files,
    Err(error) => {
      // The root itself is unreadable: keep its rows rather than wiping
      // them over what may be a transient failure.
      warn!("library root {} skipped: {error:#}", root.display());
      return Ok(());
    }
  };
  let known: HashMap<String, (i64, i64)> = {
    let mut statement =
      connection.prepare("SELECT rel_path, id, mtime FROM tracks WHERE root_id = ?1")?;
    statement
      .query_map([root_id], |row| {
        Ok((row.get::<_, String>(0)?, (row.get(1)?, row.get(2)?)))
      })?
      .collect::<std::result::Result<_, _>>()?
  };

  let mut seen: HashSet<&str> = HashSet::with_capacity(files.len());
  let mut transaction = connection.transaction()?;
  let mut pending = 0usize;
  for file in &files {
    counts.scanned += 1;
    if counts.scanned.is_multiple_of(PROGRESS_EVERY) {
      progress(counts.scanned, counts.changed);
    }
    // The database stores paths as text: a non-UTF-8 name would be saved
    // lossily, fail to resolve, and be dropped and re-added on every scan.
    let Some(rel) = file.strip_prefix(root).ok().and_then(Path::to_str) else {
      debug!("library scan skips non-UTF-8 path {}", file.display());
      continue;
    };
    seen.insert(rel);
    let Ok(metadata) = std::fs::metadata(file) else {
      continue;
    };
    let mtime = metadata
      .modified()
      .ok()
      .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
      .map(|duration| duration.as_secs() as i64)
      .unwrap_or(0);
    let known_id = match known.get(rel) {
      Some((_, known_mtime)) if *known_mtime == mtime => continue,
      Some((id, _)) => Some(*id),
      None => None,
    };
    upsert_track(&transaction, root_id, rel, file, mtime, known_id)?;
    counts.changed += 1;
    pending += 1;
    if pending >= COMMIT_BATCH {
      transaction.commit()?;
      transaction = connection.transaction()?;
      pending = 0;
    }
  }

  for (rel, (id, _)) in &known {
    if !seen.contains(rel.as_str()) {
      transaction.execute("DELETE FROM tracks WHERE id = ?1", [id])?;
    }
  }
  transaction.commit()?;
  Ok(())
}

fn upsert_track(
  transaction: &Transaction<'_>,
  root_id: i64,
  rel: &str,
  file: &Path,
  mtime: i64,
  known_id: Option<i64>,
) -> Result<()> {
  let track = read_track(file).unwrap_or_else(|| LibraryTrack {
    path: file.to_path_buf(),
    filename: file
      .file_stem()
      .map(|stem| crate::sanitize::sanitize_text(&stem.to_string_lossy()))
      .unwrap_or_default(),
    ..LibraryTrack::default()
  });
  let lyrics = if track.lyrics.is_empty() {
    read_sidecar_lyrics(file)
  } else {
    track.lyrics
  };
  // Untagged files still follow the usual "NN. artist - title" filename
  // convention; derive artist/title from the stem.
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
  match known_id {
    Some(id) => {
      transaction
        .prepare_cached(
          "UPDATE tracks SET title=?1, artist=?2, album=?3, genre=?4, filename=?5,
             duration_secs=?6, lyrics=?7, mtime=?8 WHERE id=?9",
        )?
        .execute(rusqlite::params![
          title,
          artist,
          track.album,
          track.genre,
          track.filename,
          track.duration_secs,
          lyrics,
          mtime,
          id
        ])?;
    }
    None => {
      transaction
        .prepare_cached(
          "INSERT INTO tracks (root_id, rel_path, title, artist, album, genre, filename,
             duration_secs, lyrics, mtime) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        )?
        .execute(rusqlite::params![
          root_id,
          rel,
          title,
          artist,
          track.album,
          track.genre,
          track.filename,
          track.duration_secs,
          lyrics,
          mtime
        ])?;
    }
  }
  Ok(())
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
    title: crate::sanitize::sanitize_text(tag.title().unwrap_or_default().trim()),
    artist: crate::sanitize::sanitize_text(tag.artist().unwrap_or_default().trim()),
    album: crate::sanitize::sanitize_text(tag.album().unwrap_or_default().trim()),
    genre: crate::sanitize::sanitize_text(tag.genre().unwrap_or_default().trim()),
    filename: path
      .file_stem()
      .map(|stem| crate::sanitize::sanitize_text(&stem.to_string_lossy()))
      .unwrap_or_default(),
    duration_secs: properties.duration().as_secs_f64(),
    lyrics: crate::sanitize::sanitize_text(
      &tag
        .get_string(&lofty::tag::ItemKey::Lyrics)
        .unwrap_or_default()
        .to_lowercase(),
    ),
    mtime: 0,
  };
  Some(track)
}

/// Sidecar lyrics as a lowercase blob for filtering; embedded lyrics are
/// already read by `read_track` (one lofty probe per file).
fn read_sidecar_lyrics(path: &Path) -> String {
  let lyrics = std::fs::read_to_string(path.with_extension("lrc"))
    .unwrap_or_default()
    .to_lowercase();
  crate::sanitize::sanitize_text(&lyrics)
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
