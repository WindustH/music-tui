//! Lyrics loading and LRC parsing.

use std::{
  path::{Path, PathBuf},
  time::Duration,
};

use lofty::{prelude::*, read_from_path};

#[derive(Debug, Clone)]
pub enum Lyrics {
  Synced(Vec<SyncedLine>),
  Plain(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct SyncedLine {
  pub time_secs: f64,
  pub text: String,
}

impl Lyrics {
  pub fn line_count(&self) -> usize {
    match self {
      Lyrics::Synced(lines) => lines.len(),
      Lyrics::Plain(lines) => lines.len(),
    }
  }

  /// Index of the active line for synced lyrics at `elapsed`.
  pub fn active_index(&self, elapsed: Duration) -> Option<usize> {
    let Lyrics::Synced(lines) = self else {
      return None;
    };
    let secs = elapsed.as_secs_f64();
    let mut active = None;
    for (index, line) in lines.iter().enumerate() {
      if line.time_secs <= secs + 0.25 {
        active = Some(index);
      } else {
        break;
      }
    }
    active
  }

  pub fn line(&self, index: usize) -> Option<&str> {
    match self {
      Lyrics::Synced(lines) => lines.get(index).map(|line| line.text.as_str()),
      Lyrics::Plain(lines) => lines.get(index).map(String::as_str),
    }
  }
}

/// Find lyrics for `file`: sibling `<name>.lrc`, then `<name>.lrc` in the
/// extra dirs, then `<artist> - <title>.lrc` in the extra dirs, then embedded
/// tag lyrics.
pub fn load(
  file: &Path,
  extra_dirs: &[PathBuf],
  artist: Option<&str>,
  title: Option<&str>,
) -> Result<Lyrics, String> {
  if let Some(path) = sibling_lrc_path(file) {
    if let Ok(body) = std::fs::read_to_string(&path) {
      return parse(&body);
    }
  }

  let song_stem = file
    .file_stem()
    .and_then(|stem| stem.to_str())
    .map(str::to_string);

  for dir in extra_dirs {
    if let Some(stem) = &song_stem {
      let candidate = dir.join(sanitize_filename(&format!("{stem}.lrc")));
      if let Ok(body) = std::fs::read_to_string(&candidate) {
        return parse(&body);
      }
    }

    if let (Some(artist), Some(title)) = (artist, title) {
      let candidate = dir.join(sanitize_filename(&format!("{artist} - {title}.lrc")));
      if let Ok(body) = std::fs::read_to_string(&candidate) {
        return parse(&body);
      }
    }
  }

  let tagged = read_from_path(file).map_err(|error| format!("failed to read tags: {error}"))?;
  if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
    for key in [ItemKey::Lyrics] {
      if let Some(body) = tag.get_string(&key) {
        if !body.trim().is_empty() {
          return parse(body);
        }
      }
    }
  }

  Err("no lyrics found".to_string())
}

fn sibling_lrc_path(file: &Path) -> Option<PathBuf> {
  let mut candidate = file.to_path_buf();
  candidate.set_extension("lrc");
  candidate.is_file().then_some(candidate)
}

fn sanitize_filename(name: &str) -> String {
  name.replace(['/', '\0'], "_")
}

/// Parse LRC content; falls back to plain lines when no timestamps exist.
pub fn parse(body: &str) -> Result<Lyrics, String> {
  let mut synced = Vec::new();
  let mut plain = Vec::new();
  for line in body.lines() {
    let line = line.trim_end_matches('\r');
    if let Some((text, times)) = parse_lrc_line(line) {
      if times.is_empty() {
        plain.push(line.to_string());
        continue;
      }
      for time in times {
        synced.push(SyncedLine {
          time_secs: time,
          text: text.clone(),
        });
      }
    } else {
      plain.push(line.to_string());
    }
  }

  if synced.is_empty() {
    if plain.iter().all(|line| line.trim().is_empty()) {
      return Err("lyrics file is empty".to_string());
    }
    return Ok(Lyrics::Plain(
      plain.into_iter().filter(|line| !line.trim().is_empty()).collect(),
    ));
  }

  synced.sort_by(|left, right| {
    left
      .time_secs
      .partial_cmp(&right.time_secs)
      .unwrap_or(std::cmp::Ordering::Equal)
  });
  Ok(Lyrics::Synced(synced))
}

fn parse_lrc_line(line: &str) -> Option<(String, Vec<f64>)> {
  let mut rest = line;
  let mut times = Vec::new();
  while let Some(after) = rest.strip_prefix('[') {
    let Some((stamp, tail)) = after.split_once(']') else {
      break;
    };
    if let Some(time) = parse_lrc_timestamp(stamp) {
      times.push(time);
      rest = tail;
    } else {
      // Metadata tags like [ar:...] or [ti:...]: skip the bracket.
      rest = tail;
    }
  }
  Some((rest.to_string(), times))
}

fn parse_lrc_timestamp(stamp: &str) -> Option<f64> {
  let (minutes, seconds) = stamp.split_once(':')?;
  let minutes: f64 = minutes.trim().parse().ok()?;
  let seconds: f64 = seconds.trim().parse().ok()?;
  Some(minutes * 60.0 + seconds)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_synced_and_plain() {
    let synced = parse("[00:01.5]hello\n[00:05.00]world\n").unwrap();
    let Lyrics::Synced(lines) = synced else {
      panic!("expected synced lyrics");
    };
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].time_secs, 1.5);

    let plain = parse("just\nsome lines\n").unwrap();
    assert!(matches!(plain, Lyrics::Plain(lines) if lines.len() == 2));
  }

  #[test]
  fn active_index_follows_time() {
    let lyrics = parse("[00:00]a\n[00:10]b\n[00:20]c\n").unwrap();
    assert_eq!(lyrics.active_index(Duration::from_secs(12)), Some(1));
    assert_eq!(lyrics.active_index(Duration::from_secs(59)), Some(2));
  }

  #[test]
  fn finds_same_name_lrc_in_extra_dir() {
    let root = std::env::temp_dir().join(format!("music-tui-test-{}", std::process::id()));
    let lyrics_dir = root.join("lyrics");
    std::fs::create_dir_all(&lyrics_dir).unwrap();

    let song = root.join("song.flac");
    std::fs::write(&song, b"not audio").unwrap();
    std::fs::write(lyrics_dir.join("song.lrc"), "[00:01]extra dir\n").unwrap();

    let found = load(&song, &[lyrics_dir.clone()], None, None).unwrap();
    assert!(matches!(&found, Lyrics::Synced(lines) if lines[0].text == "extra dir"));

    std::fs::remove_dir_all(&root).ok();
  }

  #[test]
  fn artist_title_lrc_takes_backseat_to_same_name() {
    let root = std::env::temp_dir().join(format!("music-tui-test2-{}", std::process::id()));
    let lyrics_dir = root.join("lyrics");
    std::fs::create_dir_all(&lyrics_dir).unwrap();

    let song = root.join("song.flac");
    std::fs::write(&song, b"not audio").unwrap();
    std::fs::write(lyrics_dir.join("song.lrc"), "same name\n").unwrap();
    std::fs::write(lyrics_dir.join("artist - title.lrc"), "artist title\n").unwrap();

    let found = load(&song, &[lyrics_dir.clone()], Some("artist"), Some("title")).unwrap();
    assert!(matches!(&found, Lyrics::Plain(lines) if lines[0] == "same name"));

    std::fs::remove_dir_all(&root).ok();
  }
}
