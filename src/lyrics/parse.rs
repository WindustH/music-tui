//! LRC parsing: synced line/word timestamps and plain-text fallback.

use tracing::warn;

use super::{Lyrics, SyncedLine, Word};

/// Upper bound for a single LRC body, in bytes. Oversized bodies are
/// truncated at the byte cap (on a char boundary) before parsing so a
/// corrupt/hostile file cannot balloon memory or the render path.
pub const MAX_LRC_BYTES: usize = 512 * 1024;
/// Upper bound for the number of LRC lines parsed from a body.
pub const MAX_LRC_LINES: usize = 2000;

pub fn parse(body: &str) -> Result<Lyrics, String> {
  // Editors on Windows often save LRC files with a UTF-8 byte-order mark;
  // left in place it hides the first line's timestamp.
  let mut body = body.strip_prefix('\u{feff}').unwrap_or(body);
  if body.len() > MAX_LRC_BYTES {
    let mut end = MAX_LRC_BYTES;
    while !body.is_char_boundary(end) {
      end -= 1;
    }
    body = &body[..end];
    warn!(bytes = MAX_LRC_BYTES, "lyrics body truncated (byte cap)");
  }
  let mut timed: Vec<SyncedLine> = Vec::new();
  let mut plain: Vec<String> = Vec::new();
  let mut offset_secs = 0.0;

  let mut lines = body.lines();
  for raw_line in lines.by_ref().take(MAX_LRC_LINES) {
    let line = crate::sanitize::sanitize_text(raw_line.trim_end_matches('\r'));
    match parse_lrc_line(&line) {
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
      ParsedLine::Untimed => match parse_offset_tag(&line) {
        Some(offset) => offset_secs = offset,
        None => plain.push(line.to_string()),
      },
    }
  }

  if lines.next().is_some() {
    warn!(lines = MAX_LRC_LINES, "lyrics body truncated (line cap)");
  }

  if timed.is_empty() {
    if plain.iter().all(|line| line.trim().is_empty()) {
      return Err("lyrics file is empty".to_string());
    }
    return Ok(Lyrics::Plain(
      plain
        .into_iter()
        .filter(|line| !line.trim().is_empty())
        .collect(),
    ));
  }

  if offset_secs != 0.0 {
    apply_offset(&mut timed, offset_secs);
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
    // Skip same-time duplicates (bilingual pairs) so both lines of a
    // pair interpolate over the full span up to the next distinct line.
    let end = timed[index + 1..]
      .iter()
      .find(|next| next.time_secs > start)
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

/// `[offset:±ms]`: a positive offset makes the lyrics appear sooner.
fn parse_offset_tag(line: &str) -> Option<f64> {
  let value = line.trim().strip_prefix("[offset:")?.strip_suffix(']')?;
  let millis: i64 = value.trim().parse().ok()?;
  Some(millis as f64 / 1000.0)
}

fn apply_offset(lines: &mut [SyncedLine], offset_secs: f64) {
  let shift = |time: f64| (time - offset_secs).clamp(0.0, MAX_LRC_SECS);
  for line in lines {
    line.time_secs = shift(line.time_secs);
    for word in line.words.iter_mut().flatten() {
      word.start_secs = shift(word.start_secs);
    }
  }
}

/// Upper bound for a sane LRC timestamp (hours). Anything beyond it is a
/// corrupt/hostile file rather than a real position, so it is rejected
/// instead of producing an infinite or overflowing duration downstream.
const MAX_LRC_SECS: f64 = 24.0 * 60.0 * 60.0;

fn parse_lrc_timestamp(stamp: &str) -> Option<f64> {
  let (minutes, seconds) = stamp.split_once(':')?;
  let minutes: f64 = minutes.trim().parse().ok()?;
  let seconds = seconds.trim();
  // Some exporters write hundredths after a second colon (`mm:ss:xx`).
  let seconds: f64 = match seconds.split_once(':') {
    Some((whole, fraction)) if fraction.bytes().all(|byte| byte.is_ascii_digit()) => {
      format!("{whole}.{fraction}").parse().ok()?
    }
    _ => seconds.parse().ok()?,
  };
  let value = minutes * 60.0 + seconds;
  if !value.is_finite() || !(0.0..=MAX_LRC_SECS).contains(&value) {
    return None;
  }
  Some(value)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn oversized_body_is_truncated_on_a_char_boundary() {
    // The byte cap lands inside a 3-byte char; parsing must not panic and
    // the surviving prefix must be usable.
    let body = format!("{}あ", "x".repeat(MAX_LRC_BYTES - 2));
    let lyrics = parse(&body).expect("truncated body still parses");
    assert_eq!(lyrics.line_count(), 1);
    let line = lyrics.line(0).expect("has one line");
    assert!(line.len() < MAX_LRC_BYTES);
    assert!(line.chars().all(|ch| ch == 'x'));
  }

  #[test]
  fn line_count_is_capped() {
    let body = (0..MAX_LRC_LINES * 2)
      .map(|index| index.to_string())
      .collect::<Vec<_>>()
      .join("\n");
    let lyrics = parse(&body).expect("plain body parses");
    assert_eq!(lyrics.line_count(), MAX_LRC_LINES);
  }

  #[test]
  fn normal_body_is_untouched() {
    let body = "[00:01.00]one\n[00:02.00]two\n";
    let lyrics = parse(body).unwrap();
    assert_eq!(lyrics.line_count(), 2);
  }

  #[test]
  fn byte_order_mark_keeps_the_first_line() {
    let lyrics = parse("\u{feff}[00:00.50]first\n[00:01.00]second\n").unwrap();
    assert_eq!(lyrics.line_count(), 2);
    assert_eq!(lyrics.line(0), Some("first"));
  }

  #[test]
  fn offset_tag_shifts_every_timestamp() {
    let lyrics = parse("[offset:+500]\n[00:01.00]<00:01.00>a <00:02.00>b\n[00:03.00]c\n").unwrap();
    let Lyrics::Synced(lines) = &lyrics else {
      panic!("expected synced lyrics");
    };
    assert_eq!(lines[0].time_secs, 0.5);
    assert_eq!(lines[0].words.as_ref().unwrap()[1].start_secs, 1.5);
    assert_eq!(lines[1].time_secs, 2.5);
    assert_eq!(lines[0].end_secs, 2.5, "end times follow the shifted lines");

    let later = parse("[offset:-1000]\n[00:01.00]a\n").unwrap();
    let Lyrics::Synced(lines) = &later else {
      panic!("expected synced lyrics");
    };
    assert_eq!(lines[0].time_secs, 2.0);
  }

  #[test]
  fn colon_separated_hundredths_parse() {
    let lyrics = parse("[00:12:34]line\n").unwrap();
    let Lyrics::Synced(lines) = &lyrics else {
      panic!("expected synced lyrics");
    };
    assert!((lines[0].time_secs - 12.34).abs() < 1e-9);
    // Metadata tags still do not parse as timestamps.
    assert!(parse_lrc_timestamp("ar:Artist").is_none());
    assert!(parse_lrc_timestamp("00:12:3x").is_none());
  }

  #[test]
  fn empty_body_is_still_rejected() {
    assert!(parse("\n\n").is_err());
  }
}
