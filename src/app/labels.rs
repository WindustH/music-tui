//! Queue label helpers: title/artist straight from the tags MPD reports.
//!
//! No corruption heuristics here: values are shown as reported. When a
//! file's tags are wrong (e.g. mixed-encoding duplicates), fix them in the
//! metadata editor — the write hits every tag block and music-tui then
//! asks MPD to update its database entry for the file.

use super::*;
use mpd_client::tag::Tag;

/// The song's title (first reported value, trimmed).
pub(crate) fn song_title(song: &Song) -> Option<&str> {
  tag_value(song, Tag::Title)
}

/// The song's artist (first reported value, trimmed).
pub(crate) fn song_artist(song: &Song) -> Option<&str> {
  tag_value(song, Tag::Artist)
}

/// The song's album (first reported value, trimmed).
pub(crate) fn song_album(song: &Song) -> Option<&str> {
  tag_value(song, Tag::Album)
}

fn tag_value(song: &Song, tag: Tag) -> Option<&str> {
  song
    .tags
    .get(&tag)
    .and_then(|values| values.first())
    .map(|value| value.trim())
    .filter(|value| !value.is_empty())
}
