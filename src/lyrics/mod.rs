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
  /// Bilingual LRC files repeat one timestamp for the original line and
  /// its translation; the first line of such a group is the anchor (for
  /// scroll centering and the manual cursor).
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
    let index = active?;
    let time = lines[index].time_secs;
    let mut start = index;
    while start > 0 && lines[start - 1].time_secs == time {
      start -= 1;
    }
    Some(start)
  }

  /// Range `[start, end)` of lines sharing the active line's timestamp —
  /// a bilingual original/translation pair lights up together.
  pub fn active_group(&self, elapsed: Duration) -> Option<(usize, usize)> {
    let start = self.active_index(elapsed)?;
    self.item_group(start)
  }

  /// Range `[start, end)` of the navigation item containing `index`.
  /// Synced lines with the same timestamp form one item; plain lyrics keep
  /// one physical line per item.
  pub fn item_group(&self, index: usize) -> Option<(usize, usize)> {
    match self {
      Lyrics::Synced(lines) => {
        let time = lines.get(index)?.time_secs;
        let mut start = index;
        while start > 0 && lines[start - 1].time_secs == time {
          start -= 1;
        }
        let mut end = index + 1;
        while end < lines.len() && lines[end].time_secs == time {
          end += 1;
        }
        Some((start, end))
      }
      Lyrics::Plain(lines) => (index < lines.len()).then_some((index, index + 1)),
    }
  }

  /// Move from `index` by logical lyric items and return the destination
  /// item's first line. Repeated timestamps therefore consume one step.
  pub fn move_item_index(&self, index: usize, delta: i32) -> Option<usize> {
    let (mut start, mut end) = self.item_group(index)?;
    if delta < 0 {
      for _ in 0..delta.unsigned_abs() {
        if start == 0 {
          break;
        }
        start = self.item_group(start - 1)?.0;
      }
    } else {
      for _ in 0..delta as usize {
        if end >= self.line_count() {
          break;
        }
        (start, end) = self.item_group(end)?;
      }
    }
    Some(start)
  }

  /// Sung char count for the line at `index` at `elapsed`.
  /// Word-timed lines follow their word timestamps; line-timed lines
  /// interpolate evenly over the line's characters.
  pub fn karaoke_at(&self, index: usize, elapsed: Duration) -> usize {
    let Lyrics::Synced(lines) = self else {
      return 0;
    };
    let Some(line) = lines.get(index) else {
      return 0;
    };
    let secs = elapsed.as_secs_f64();
    let total = line.text.chars().count();
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
      (fraction * total as f64).round() as usize
    };
    sung.min(total)
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
pub use parse::parse;

mod parse;

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
    assert_eq!(lyrics.karaoke_at(0, Duration::from_secs(15)), 2);
    assert_eq!(lyrics.karaoke_at(0, Duration::from_secs(10)), 0);
    assert_eq!(lyrics.karaoke_at(1, Duration::from_secs(21)), 1);
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

    assert_eq!(lyrics.karaoke_at(0, Duration::from_secs_f64(10.5)), 4);
    assert_eq!(lyrics.karaoke_at(0, Duration::from_secs_f64(11.5)), 6);
  }

  #[test]
  fn bilingual_pairs_light_up_together() {
    // Enhanced LRC with same-timestamp original/translation pairs, as
    // produced by dual-language lyric exporters.
    let body = concat!(
      "[ti:chAngE]\n[ar:MIWA]\n\n",
      "[00:00.67]<00:00.67>ChAngE <00:01.34><00:01.34>な\n",
      "[00:00.67]改变 不再屈意顺从感情用事\n",
      "[00:03.47]<00:03.47>今\n",
      "[00:03.47]与闪烁其词的自己毅然道一声永别\n",
    );
    let lyrics = parse(body).unwrap();
    let Lyrics::Synced(lines) = &lyrics else {
      panic!("expected synced lyrics");
    };
    assert_eq!(lines.len(), 4);
    // Metadata tags are skipped; both pairs parse as same-time groups.
    assert_eq!(lyrics.active_group(Duration::from_secs_f64(1.0)), Some((0, 2)));
    assert_eq!(lyrics.active_group(Duration::from_secs_f64(4.0)), Some((2, 4)));
    // Group anchor is the first line of the pair.
    assert_eq!(lyrics.active_index(Duration::from_secs_f64(1.0)), Some(0));

    // Word-timed original: double timestamps (`<01.34><01.34>`) yield an
    // empty word, harmless; sung chars follow word starts.
    assert_eq!(lyrics.karaoke_at(0, Duration::from_secs_f64(1.0)), 7); // "ChAngE "
    assert_eq!(lyrics.karaoke_at(0, Duration::from_secs_f64(1.5)), 8); // + な

    // Translation: interpolates over the pair's full span, not +0.05s.
    let span = lines[1].end_secs - lines[1].time_secs;
    assert!((span - 2.8).abs() < 1e-6, "span was {span}");
    // 13 chars; 60% through the pair -> 7.8 -> 8 sung.
    let past_half = lines[1].time_secs + span * 0.6;
    assert_eq!(lyrics.karaoke_at(1, Duration::from_secs_f64(past_half)), 8);
  }

  #[test]
  fn keyboard_navigation_treats_equal_timestamps_as_one_item() {
    let lyrics = parse(
      "[00:01]original one\n[00:01]translation one\n[00:02]original two\n[00:02]translation two\n[00:03]last\n",
    )
    .unwrap();

    assert_eq!(lyrics.item_group(1), Some((0, 2)));
    assert_eq!(lyrics.move_item_index(0, 1), Some(2));
    assert_eq!(lyrics.move_item_index(1, 1), Some(2));
    assert_eq!(lyrics.move_item_index(3, -1), Some(0));
    assert_eq!(lyrics.move_item_index(0, 10), Some(4));
    assert_eq!(lyrics.move_item_index(4, -10), Some(0));
  }

  #[test]
  fn plain_lyrics_navigation_remains_line_based() {
    let lyrics = parse("one\ntwo\nthree\n").unwrap();
    assert_eq!(lyrics.move_item_index(0, 1), Some(1));
    assert_eq!(lyrics.move_item_index(2, -1), Some(1));
  }

  #[test]
  fn finds_same_name_lrc_in_extra_dir() {
    let root = std::env::temp_dir().join(format!("music-tui-test-{}", std::process::id()));
    let lyrics_dir = root.join("lyrics");
    std::fs::create_dir_all(&lyrics_dir).unwrap();

    let song = root.join("song.flac");
    std::fs::write(&song, b"not audio").unwrap();
    std::fs::write(lyrics_dir.join("song.lrc"), "[00:01]extra dir\n").unwrap();

    let found = load(&song, &[lyrics_dir], None, None).unwrap();
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

    let found = load(&song, &[lyrics_dir], Some("artist"), Some("title")).unwrap();
    assert!(matches!(&found, Lyrics::Plain(lines) if lines[0] == "same name"));

    std::fs::remove_dir_all(&root).ok();
  }
}
