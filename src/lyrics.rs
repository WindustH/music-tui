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
  /// End of the line: the start of the next line, or an estimate for the
  /// last line. Used for per-char karaoke interpolation.
  pub end_secs: f64,
  pub text: String,
  /// Word-level timings from enhanced LRC (`<mm:ss.xx>` tags). When present
  /// karaoke highlighting follows them instead of even interpolation.
  pub words: Option<Vec<Word>>,
}

#[derive(Debug, Clone)]
pub struct Word {
  pub start_secs: f64,
  pub end_secs: f64,
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

  /// Karaoke progress at `elapsed`: `(line index, sung char count)`.
  /// Word-timed lines follow their word timestamps; line-timed lines
  /// interpolate evenly over the line's characters.
  pub fn karaoke(&self, elapsed: Duration) -> Option<(usize, usize)> {
    let Lyrics::Synced(lines) = self else {
      return None;
    };
    let secs = elapsed.as_secs_f64();
    let index = self.active_index(elapsed)?;
    let line = &lines[index];
    let sung = if secs < line.time_secs {
      0
    } else if let Some(words) = &line.words {
      let mut count = 0;
      for word in words {
        if word.start_secs <= secs {
          count += word.text.chars().count();
        } else {
          break;
        }
      }
      count
    } else {
      let span = (line.end_secs - line.time_secs).max(0.001);
      let fraction = ((secs - line.time_secs) / span).clamp(0.0, 1.0);
      (fraction * line.text.chars().count() as f64).round() as usize
    };
    Some((index, sung.min(line.text.chars().count())))
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
  if let Some(path) = sibling_lrc_path(file)
    && let Ok(body) = std::fs::read_to_string(&path) {
      return parse(&body);
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
      if let Some(body) = tag.get_string(&key)
        && !body.trim().is_empty() {
          return parse(body);
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
  let mut timed: Vec<SyncedLine> = Vec::new();
  let mut plain: Vec<String> = Vec::new();

  for line in body.lines() {
    let line = line.trim_end_matches('\r');
    match parse_lrc_line(line) {
      ParsedLine::Timed { times, text, words } => {
        if times.is_empty() {
          plain.push(line.to_string());
        } else {
          // Word timings only apply to single-timestamp lines.
          let words = if times.len() == 1 { words } else { None };
          for time in times {
            timed.push(SyncedLine {
              time_secs: time,
              end_secs: 0.0,
              text: text.clone(),
              words: words.clone(),
            });
          }
        }
      }
      ParsedLine::Untimed => plain.push(line.to_string()),
    }
  }

  if timed.is_empty() {
    if plain.iter().all(|line| line.trim().is_empty()) {
      return Err("lyrics file is empty".to_string());
    }
    return Ok(Lyrics::Plain(
      plain.into_iter().filter(|line| !line.trim().is_empty()).collect(),
    ));
  }

  timed.sort_by(|left, right| {
    left
      .time_secs
      .partial_cmp(&right.time_secs)
      .unwrap_or(std::cmp::Ordering::Equal)
  });

  // Derive line end times (next line's start) and word end times (next
  // word's start, else the line's end) for karaoke interpolation.
  let last_index = timed.len() - 1;
  for index in 0..=last_index {
    let start = timed[index].time_secs;
    let fallback_end = start + 5.0;
    let end = timed
      .get(index + 1)
      .map(|next| next.time_secs.max(start + 0.05))
      .unwrap_or(fallback_end);
    timed[index].end_secs = end;
    if let Some(words) = &mut timed[index].words {
      for word in 0..words.len() {
        let word_end = words
          .get(word + 1)
          .map(|next| next.start_secs.max(words[word].start_secs))
          .unwrap_or(end);
        words[word].end_secs = word_end;
      }
    }
  }
  Ok(Lyrics::Synced(timed))
}

enum ParsedLine {
  Untimed,
  Timed {
    times: Vec<f64>,
    text: String,
    words: Option<Vec<Word>>,
  },
}

fn parse_lrc_line(line: &str) -> ParsedLine {
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
  if times.is_empty() {
    return ParsedLine::Untimed;
  }

  // Enhanced LRC: <mm:ss.xx> word tags precede the text they time.
  let mut words: Vec<Word> = Vec::new();
  let mut segment = String::new();
  let mut plain = String::new();
  let mut chars = rest.chars().peekable();
  while let Some(ch) = chars.next() {
    if ch != '<' {
      segment.push(ch);
      plain.push(ch);
      continue;
    }
    let mut stamp = String::new();
    let mut closed = false;
    for tag_ch in chars.by_ref() {
      if tag_ch == '>' {
        closed = true;
        break;
      }
      stamp.push(tag_ch);
    }
    match (closed, parse_lrc_timestamp(&stamp)) {
      (true, Some(start)) => {
        // Text collected since the previous tag belongs to it; text after
        // this tag belongs to the new word.
        if let Some(last) = words.last_mut() {
          last.text.push_str(&segment);
        }
        segment.clear();
        words.push(Word {
          start_secs: start,
          end_secs: 0.0,
          text: String::new(),
        });
      }
      _ => {
        // Not a timestamp tag: keep it literally.
        let literal = format!("<{stamp}>");
        segment.push_str(&literal);
        plain.push_str(&literal);
      }
    }
  }
  if let Some(last) = words.last_mut() {
    last.text.push_str(&segment);
  }
  if !words.is_empty() {
    return ParsedLine::Timed {
      times,
      text: plain,
      words: Some(words),
    };
  }

  ParsedLine::Timed {
    times,
    text: rest.to_string(),
    words: None,
  }
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
  fn karaoke_interpolates_line_timed_text() {
    let lyrics = parse("[00:10]abcd\n[00:20]next\n").unwrap();
    // Halfway through the 10s line: two of four chars sung.
    assert_eq!(lyrics.karaoke(Duration::from_secs(15)), Some((0, 2)));
    assert_eq!(lyrics.karaoke(Duration::from_secs(10)), Some((0, 0)));
    assert_eq!(lyrics.karaoke(Duration::from_secs(21)), Some((1, 1)));
  }

  #[test]
  fn karaoke_follows_word_tags() {
    let body = "[00:10.00]<00:10.00>you <00:11.00>me\n[00:20.00]next\n";
    let lyrics = parse(body).unwrap();
    let Lyrics::Synced(lines) = &lyrics else {
      panic!("expected synced lyrics");
    };
    let words = lines[0].words.as_ref().expect("word timings");
    assert_eq!(words.len(), 2);
    assert_eq!(words[0].text, "you ");
    assert_eq!(words[0].end_secs, 11.0);
    assert_eq!(words[1].text, "me");

    assert_eq!(lyrics.karaoke(Duration::from_secs_f64(10.5)), Some((0, 4)));
    assert_eq!(lyrics.karaoke(Duration::from_secs_f64(11.5)), Some((0, 6)));
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
