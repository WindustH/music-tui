//! Terminal-output sanitation.
//!
//! Untrusted text — MPD tags, URLs, PLS/LRC bodies, server error strings —
//! can carry control characters (ESC, CSI, C1 ranges) that a terminal would
//! interpret as input instead of rendering. Strip every control character
//! so all text that reaches ratatui is display-only.

pub(crate) fn sanitize_text(text: &str) -> String {
  text
    .chars()
    .filter(|ch| !ch.is_control() && *ch != '\u{7f}')
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn keeps_printable_text() {
    assert_eq!(sanitize_text("ABC cde 123"), "ABC cde 123");
    assert_eq!(sanitize_text("夜的第七章"), "夜的第七章");
    assert_eq!(sanitize_text("a_b-c.d"), "a_b-c.d");
  }

  #[test]
  fn strips_escape_sequences() {
    assert_eq!(sanitize_text("hello \u{1b}[31mred"), "hello [31mred");
    assert_eq!(sanitize_text("\u{1b}]0;x\u{7}"), "]0;x");
    assert_eq!(sanitize_text("\u{9b}1 @"), "1 @");
  }

  #[test]
  fn strips_tabs_newlines_and_del() {
    assert_eq!(sanitize_text("a\tb\nc\u{7f}d"), "abcd");
  }
}
