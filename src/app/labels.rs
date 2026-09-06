//! Queue label helpers: title/artist straight from the tags MPD reports.
//!
//! No corruption heuristics here: values are shown as reported (control
//! characters stripped for terminal safety). When a file's tags are wrong
//! (e.g. mixed-encoding duplicates), fix them in the metadata editor — the
//! write hits every tag block and music-tui then asks MPD to update its
//! database entry for the file.

use super::*;
use crate::sanitize::sanitize_text;
use mpd_client::tag::Tag;

/// The song's title (first reported value, trimmed and sanitized).
pub(crate) fn song_title(song: &Song) -> Option<String> {
  tag_value(song, Tag::Title)
}

/// The song's artist (first reported value, trimmed and sanitized).
pub(crate) fn song_artist(song: &Song) -> Option<String> {
  tag_value(song, Tag::Artist)
}

/// The song's album (first reported value, trimmed and sanitized).
pub(crate) fn song_album(song: &Song) -> Option<String> {
  tag_value(song, Tag::Album)
}

fn tag_value(song: &Song, tag: Tag) -> Option<String> {
  song
    .tags
    .get(&tag)
    .and_then(|values| values.first())
    .map(|value| sanitize_text(value.trim()))
    .filter(|value| !value.is_empty())
}
