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

/// Case-folded text with whitespace removed: the form both sides of a
/// space-insensitive match are compared in.
pub(crate) fn fold(text: &str) -> String {
  text
    .chars()
    .filter(|ch| !ch.is_whitespace())
    .flat_map(char::to_lowercase)
    .collect()
}

/// A filter term prepared for [`matches_needle`]. Prepare terms once per
/// filter pass, not once per field.
pub(crate) fn needle(term: &str) -> String {
  fold(term)
}

/// Whether `text` contains the prepared `needle` with spaces ignored and
/// case folded — same semantics as [`StrippedText::find_all`] finding a
/// match, without building byte-origin tables (filtering only needs a
/// yes/no; ranges are computed for the few rows actually drawn).
pub(crate) fn matches_needle(text: &str, needle: &str) -> bool {
  contains_needle(&fold(text), needle)
}

/// Whether already-[`fold`]ed text contains the prepared `needle`.
pub(crate) fn contains_needle(folded: &str, needle: &str) -> bool {
  !needle.is_empty() && folded.contains(needle)
}
