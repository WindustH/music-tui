//! LRC parsing: synced line/word timestamps and plain-text fallback.

use super::{Lyrics, SyncedLine, Word};

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
      plain
        .into_iter()
        .filter(|line| !line.trim().is_empty())
        .collect(),
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

fn parse_lrc_timestamp(stamp: &str) -> Option<f64> {
  let (minutes, seconds) = stamp.split_once(':')?;
  let minutes: f64 = minutes.trim().parse().ok()?;
  let seconds: f64 = seconds.trim().parse().ok()?;
  Some(minutes * 60.0 + seconds)
}
