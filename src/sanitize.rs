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
