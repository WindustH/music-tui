//! Field matching for the library filter: space-insensitive multi-term
//! AND matching across track fields, ranked title > artist > album >
//! filename > genre > lyrics.

use super::LibraryTrack;
use crate::strip::{contains_needle, fold, needle};

/// Track field for filter/match purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackField {
  Title,
  Artist,
  Album,
  Genre,
  Filename,
  Lyrics,
}

impl TrackField {
  pub fn parse(value: &str) -> Option<Self> {
    match value.trim() {
      "title" => Some(Self::Title),
      "artist" => Some(Self::Artist),
      "album" => Some(Self::Album),
      "genre" => Some(Self::Genre),
      "filename" => Some(Self::Filename),
      "lyrics" => Some(Self::Lyrics),
      _ => None,
    }
  }

  pub fn text(self, track: &LibraryTrack) -> &str {
    match self {
      Self::Title => &track.title,
      Self::Artist => &track.artist,
      Self::Album => &track.album,
      Self::Genre => &track.genre,
      Self::Filename => &track.filename,
      Self::Lyrics => &track.lyrics,
    }
  }

  /// Priority rank for ordering matches: lower sorts first.
  fn rank(self) -> u8 {
    match self {
      Self::Title => 0,
      Self::Artist => 1,
      Self::Album => 2,
      Self::Filename => 3,
      Self::Genre => 4,
      Self::Lyrics => 5,
    }
  }
}

/// Fields in match-priority order (title > artist > album > filename >
/// genre > lyrics): the first field containing a term is its best match.
const RANKED_FIELDS: [TrackField; 6] = [
  TrackField::Title,
  TrackField::Artist,
  TrackField::Album,
  TrackField::Filename,
  TrackField::Genre,
  TrackField::Lyrics,
];

/// A visible library row: an index into the full track list plus the
/// field holding the best (highest-priority) term-0 match, used for
/// result ordering.
#[derive(Debug, Clone, Copy)]
pub struct TrackMatch {
  pub index: usize,
  pub field: TrackField,
}

/// Every track, unfiltered, in library order.
pub fn all_rows(tracks: &[LibraryTrack]) -> Vec<TrackMatch> {
  (0..tracks.len())
    .map(|index| TrackMatch {
      index,
      field: TrackField::Title,
    })
    .collect()
}

/// Filter the `candidates` (indices into `tracks`) by `query` over every
/// field. The query is split on whitespace; every term must match
/// somewhere (AND) with spaces inside the field text ignored. Results are
/// ordered by the best field of term 0, then artist / album / title.
///
/// Field texts are folded lazily, in priority order, and only until a term
/// matches — the (large) lyrics blob is only touched for tracks no other
/// field matches.
pub fn filter_tracks(
  tracks: &[LibraryTrack],
  candidates: impl IntoIterator<Item = usize>,
  query: &str,
) -> Vec<TrackMatch> {
  let needles: Vec<String> = query.split_whitespace().map(needle).collect();
  let mut out = Vec::new();
  for index in candidates {
    let Some(track) = tracks.get(index) else {
      continue;
    };
    if let Some(field) = best_match(track, &needles) {
      out.push(TrackMatch { index, field });
    }
  }
  out.sort_by(|a, b| {
    let (left, right) = (&tracks[a.index], &tracks[b.index]);
    a.field
      .rank()
      .cmp(&b.field.rank())
      .then_with(|| left.artist.cmp(&right.artist))
      .then_with(|| left.album.cmp(&right.album))
      .then_with(|| left.title.cmp(&right.title))
      .then_with(|| a.index.cmp(&b.index))
  });
  out
}

/// The best field for term 0 when every term matches some field.
fn best_match(track: &LibraryTrack, needles: &[String]) -> Option<TrackField> {
  let mut folded: [Option<String>; RANKED_FIELDS.len()] = Default::default();
  let mut best = None;
  for needle in needles {
    let field = RANKED_FIELDS.iter().enumerate().find_map(|(slot, field)| {
      let text = folded[slot].get_or_insert_with(|| fold(field.text(track)));
      contains_needle(text, needle).then_some(*field)
    })?;
    best.get_or_insert(field);
  }
  Some(best.unwrap_or(TrackField::Title))
}
