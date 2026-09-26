//! Text windowing and match highlighting for filtered rows: long field
//! values scroll horizontally (by display columns) so filter matches stay
//! visible, and every matched range is highlighted.

use ratatui::{style::Style, text::Span};
use unicode_width::UnicodeWidthChar;

/// Display width of one char in terminal columns (CJK chars are 2).
pub(super) fn char_display_width(ch: char) -> usize {
  ch.width().unwrap_or(0)
}

/// Total display width of `text` in terminal columns.
pub(super) fn text_display_width(text: &str) -> usize {
  text.chars().map(char_display_width).sum()
}

/// Number of chars starting strictly before `byte` (its char index).
fn chars_starting_before(text: &str, byte: usize) -> usize {
  text.char_indices().take_while(|(b, _)| *b < byte).count()
}

/// Pick a horizontal window for long field values so the match sits
/// visibly inside it, measured in terminal columns: CJK chars occupy
/// two columns, so a char-counted budget can overflow the cell. Returns
/// the char offset to start drawing at.
pub(super) fn match_window(text: &str, range: (usize, usize), budget: usize) -> usize {
  let total = text_display_width(text);
  if budget == 0 || total <= budget {
    return 0;
  }
  let chars: Vec<char> = text.chars().collect();
  let anchor = chars_starting_before(text, range.0);
  let match_end = chars_starting_before(text, range.1);
  // Lead the match in by up to a third of the budget, walking back by
  // display columns (never splitting a wide char).
  let lead_budget = budget / 3;
  let mut start = anchor;
  let mut lead = 0;
  while start > 0 {
    let width = char_display_width(chars[start - 1]);
    if lead + width > lead_budget {
      break;
    }
    lead += width;
    start -= 1;
  }
  // If the match still does not fit, slide the window forward until it does.
  while start < match_end
    && chars[start..match_end]
      .iter()
      .map(|ch| char_display_width(*ch))
      .sum::<usize>()
      > budget
  {
    start += 1;
  }
  start.min(chars.len() - 1)
}

/// The visible slice of `text` starting at char `start`, cut at the
/// first char that would overflow `budget` display columns.
pub(super) fn window_text(text: &str, start: usize, budget: usize) -> String {
  let mut used = 0;
  let mut window = String::new();
  for ch in text.chars().skip(start) {
    let width = char_display_width(ch);
    if used + width > budget {
      break;
    }
    used += width;
    window.push(ch);
  }
  window
}

/// Window for a filtered cell: anchored on the leftmost term match so
/// whichever match sorts first stays visible (anchoring on
/// `ranges.first()` can scroll an earlier match out of view). Returns
/// the visible text and its char offset into `text`.
pub(super) fn filter_window(
  text: &str,
  ranges: &[(usize, usize)],
  budget: usize,
) -> (String, usize) {
  let start = match ranges.iter().copied().min() {
    Some(range) => match_window(text, range, budget),
    None => 0,
  };
  (window_text(text, start, budget), start)
}

/// Largest char boundary <= `index` in `text`.
pub(super) fn char_boundary_index(text: &str, index: usize) -> usize {
  if index >= text.len() {
    text.len()
  } else {
    let mut index = index;
    while index > 0 && !text.is_char_boundary(index) {
      index -= 1;
    }
    index
  }
}

/// Multi-range variant: `ranges` hold byte offsets into the full `text`
/// (e.g. every filter term match); all matches inside the window are
/// highlighted.
pub(super) fn highlighted_ranges_spans(
  window: &str,
  text: &str,
  window_start: usize,
  ranges: Vec<(usize, usize)>,
  base: Style,
  highlight: Style,
) -> Vec<Span<'static>> {
  // Byte offset of the visible window inside `text`.
  let window_bytes: usize = text.chars().take(window_start).map(char::len_utf8).sum();
  let mut shifted: Vec<(usize, usize)> = ranges
    .into_iter()
    .map(|(start, end)| {
      (
        start.saturating_sub(window_bytes),
        end.saturating_sub(window_bytes).min(window.len()),
      )
    })
    .filter(|(start, end)| start < end && *end <= window.len())
    .collect();
  shifted.sort();
  // Merge overlapping ranges, then emit plain/highlight segments.
  let mut merged: Vec<(usize, usize)> = Vec::new();
  for (start, end) in shifted {
    match merged.last_mut() {
      Some((_, last_end)) if start <= *last_end => *last_end = (*last_end).max(end),
      _ => merged.push((start, end)),
    }
  }
  let mut spans = Vec::new();
  let mut cursor = 0;
  for (start, end) in merged {
    let start = char_boundary_index(window, start);
    let end = char_boundary_index(window, end);
    if start < cursor || start >= end {
      continue;
    }
    if cursor < start {
      spans.push(Span::styled(window[cursor..start].to_string(), base));
    }
    spans.push(Span::styled(window[start..end].to_string(), highlight));
    cursor = end;
  }
  if cursor < window.len() {
    spans.push(Span::styled(window[cursor..].to_string(), base));
  }
  if spans.is_empty() {
    spans.push(Span::styled(window.to_string(), base));
  }
  spans
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn match_window_measures_display_columns() {
    // 28 CJK chars = 56 columns; a 30-column budget must scroll.
    let text = format!("{}夜曲{}", "曲".repeat(20), "词".repeat(6));
    let start_byte = text.find("夜曲").unwrap();
    let range = (start_byte, start_byte + "夜曲".len());
    let start = match_window(&text, range, 30);
    let window = window_text(&text, start, 30);
    assert!(window.contains("夜曲"), "window {window:?} hides the match");
    assert!(
      window.chars().map(char_display_width).sum::<usize>() <= 30,
      "window overflows the column budget"
    );
  }

  #[test]
  fn filter_window_anchors_on_leftmost_match() {
    // "love" matches late in the text, "beatles" at the very start:
    // the leftmost match must stay visible.
    let text = format!("Beatles Band Song{} Love Song", " pad".repeat(10));
    let love = text.find("Love").unwrap();
    let beatles = text.find("Beatles").unwrap();
    let ranges = [(love, love + 4), (beatles, beatles + 7)];
    let (window, _) = filter_window(&text, &ranges, 20);
    assert!(
      window.contains("Beatles"),
      "window {window:?} hides the leftmost match"
    );
  }

  #[test]
  fn window_text_never_splits_wide_chars() {
    // 7 columns fit only 3 double-width chars.
    assert_eq!(window_text("曲曲曲", 0, 7).chars().count(), 3);
    assert_eq!(window_text("曲曲曲", 0, 5).chars().count(), 2);
    assert_eq!(window_text("夜曲", 1, 10), "曲");
  }
}
