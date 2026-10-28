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
    && let Ok(body) = std::fs::read_to_string(&path)
  {
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
  if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag())
    && let Some(body) = tag.get_string(&ItemKey::Lyrics)
    && !body.trim().is_empty()
  {
    return parse(body);
  }

  Err("no lyrics found".to_string())
}

fn sibling_lrc_path(file: &Path) -> Option<PathBuf> {
  let mut candidate = file.to_path_buf();
  candidate.set_extension("lrc");
  candidate.is_file().then_some(candidate)
}

fn sanitize_filename(name: &str) -> String {
  // Backslashes are directory separators on Windows (a stray one would
  // point the cache path outside the lyrics dir); NUL is invalid anywhere.
  name.replace(['/', '\\', '\0'], "_")
}

/// Parse LRC content; falls back to plain lines when no timestamps exist.
pub use parse::parse;

mod parse;
