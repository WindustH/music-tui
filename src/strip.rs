//! Space-insensitive matching helpers.
//!
//! Filter terms are split on whitespace and every term must match (AND).
//! Field text is matched with spaces ignored: "Love Story" matches
//! "lovestory". Matches map back to byte ranges of the ORIGINAL text so
//! highlights land on the characters the user sees.

/// A field text prepared for space-insensitive matching.
pub(crate) struct StrippedText {
  /// Lowercased text with whitespace removed.
  lowered: String,
  /// For each char in `lowered`: its byte range in the original text.
  /// A single original char can expand to several lowered chars (e.g.
  /// 'İ'), which all share the same origin range.
  origins: Vec<(usize, usize)>,
}

impl StrippedText {
  pub fn new(text: &str) -> Self {
    let mut lowered = String::new();
    let mut origins = Vec::new();
    for (start, ch) in text.char_indices() {
      if ch.is_whitespace() {
        continue;
      }
      let end = start + ch.len_utf8();
      for low in ch.to_lowercase() {
        lowered.push(low);
        origins.push((start, end));
      }
    }
    Self { lowered, origins }
  }

  pub fn matches(&self, term: &str) -> bool {
    self.first_range(term).is_some()
  }

  fn first_range(&self, term: &str) -> Option<(usize, usize)> {
    self.find_all(term).into_iter().next()
  }

  /// All occurrence ranges of `term`, in byte coordinates of the
  /// ORIGINAL text. The term itself is stripped and lowercased too.
  pub fn find_all(&self, term: &str) -> Vec<(usize, usize)> {
    let needle = StrippedText::new(term);
    if needle.lowered.is_empty() || self.lowered.is_empty() {
      return Vec::new();
    }
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(relative) = self.lowered[from..].find(needle.lowered.as_str()) {
      let start = from + relative;
      let end = start + needle.lowered.len();
      let first_char = self.lowered[..start].chars().count();
      let last_char = self.lowered[..end].chars().count() - 1;
      out.push((self.origins[first_char].0, self.origins[last_char].1));
      from = end;
    }
    out
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn ignores_spaces_and_case() {
    let text = StrippedText::new("Love  Story");
    assert!(text.matches("lovestory"));
    assert!(text.matches("Love Story"));
    assert!(text.matches("love"));
    assert!(!text.matches("story love"));
  }

  #[test]
  fn ranges_land_on_original_bytes() {
    let text = StrippedText::new("Love Story love");
    let ranges = text.find_all("love");
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0], (0, 4));
    // "Love Story love": bytes L0 o1 v2 e3 ' '4 S5 t6 o7 r8 y9 ' '10 l11 o12 v13 e14.
    assert_eq!(ranges[1], (11, 15));
  }

  #[test]
  fn unicode_ranges_are_char_boundaries() {
    let text = StrippedText::new("夜的 第七章 夜曲");
    let ranges = text.find_all("夜曲");
    // "夜的 第七章 夜曲": 夜(0)的(3)sp(6)第(7)七(10)章(13)sp(16)夜(17)曲(20)
    // → the match spans bytes 17..23.
    assert_eq!(ranges, vec![(17, 23)]);
    let full = "夜的 第七章 夜曲";
    assert!(full.is_char_boundary(ranges[0].0));
    assert!(full.is_char_boundary(ranges[0].1));
  }

  #[test]
  fn match_spanning_removed_spaces() {
    // "a b" matches the whole text with the space skipped.
    let text = StrippedText::new("x a by");
    let ranges = text.find_all("a b");
    // Original text: 'x'(0) ' '(1) 'a'(2) ' '(3) 'b'(4) 'y'(5)
    assert_eq!(ranges, vec![(2, 5)]);
  }
}
