//! Field matching for the library filter: space-insensitive multi-term
//! AND matching across track fields, ranked title > artist > album >
//! filename > genre > lyrics.

use super::LibraryTrack;

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

/// A matched track plus the field that produced the best
/// (highest-priority) term-0 match; used for result ordering.
#[derive(Debug, Clone)]
pub struct TrackMatch {
  pub track: LibraryTrack,
  /// Field holding the best match.
  pub field: TrackField,
}

/// Filter tracks by `query` over every field. The query is split on
/// whitespace; every term must match somewhere (AND) with spaces inside
/// the field text ignored, and the reported field/range is the term-0
/// match with the best priority (in original-text byte coordinates).
pub fn filter_tracks(tracks: &[LibraryTrack], query: &str) -> Vec<TrackMatch> {
  let terms: Vec<String> = query.split_whitespace().map(str::to_string).collect();
  let mut out = Vec::new();
  for track in tracks {
    let fields: Vec<(TrackField, crate::strip::StrippedText)> = [
      TrackField::Title,
      TrackField::Artist,
      TrackField::Album,
      TrackField::Genre,
      TrackField::Filename,
      TrackField::Lyrics,
    ]
    .iter()
    .map(|field| (*field, crate::strip::StrippedText::new(field.text(track))))
    .collect();
    let mut best: Option<(TrackField, (usize, usize))> = None;
    let mut all_terms_match = true;
    for (index, term) in terms.iter().enumerate() {
      let mut term_match: Option<(TrackField, (usize, usize))> = None;
      for (field, text) in &fields {
        if let Some(range) = text.find_all(term).first().copied() {
          let candidate = (*field, range);
          term_match = Some(match term_match {
            None => candidate,
            Some(current) if field.rank() < current.0.rank() => candidate,
            Some(current) => current,
          });
        }
      }
      match term_match {
        Some(found) => {
          if index == 0 {
            best = Some(found);
          }
        }
        None => {
          all_terms_match = false;
          break;
        }
      }
    }
    if all_terms_match {
      let (field, _range) = best.unwrap_or((TrackField::Title, (0, 0)));
      out.push(TrackMatch {
        track: track.clone(),
        field,
      });
    }
  }
  out.sort_by(|a, b| {
    a.field
      .rank()
      .cmp(&b.field.rank())
      .then_with(|| a.track.artist.cmp(&b.track.artist))
      .then_with(|| a.track.album.cmp(&b.track.album))
      .then_with(|| a.track.title.cmp(&b.track.title))
  });
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  fn track(title: &str, artist: &str, lyrics: &str) -> LibraryTrack {
    LibraryTrack {
      title: title.to_string(),
      artist: artist.to_string(),
      lyrics: lyrics.to_string(),
      ..LibraryTrack::default()
    }
  }

  #[test]
  fn filter_orders_by_artist_album_title() {
    let mut later = track("same title", "artist", "");
    later.album = "Album Z".to_string();
    let mut earlier = track("same title", "artist", "");
    earlier.album = "Album A".to_string();
    let hits = filter_tracks(&[later, earlier], "title");
    assert_eq!(hits[0].track.album, "Album A");
    assert_eq!(hits[1].track.album, "Album Z");
  }

  #[test]
  fn filter_matches_all_terms_and_picks_priority_field() {
    let tracks = vec![
      track("夜的第七章", "周杰伦", "夜曲不停写"),
      track("以父之名", "周杰伦", ""),
    ];
    let hits = filter_tracks(&tracks, "夜曲");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].field, TrackField::Lyrics);
  }

  #[test]
  fn filter_ranks_title_over_album() {
    let tracks = vec![
      track("album-hit", "a", ""), // title match
      track("song", "b", "album-hit in lyrics"),
    ];
    let hits = filter_tracks(&tracks, "album-hit");
    assert_eq!(hits[0].field, TrackField::Title);
    assert_eq!(hits[1].field, TrackField::Lyrics);
  }

  #[test]
  fn filter_requires_every_term() {
    let tracks = vec![track("夜的第七章", "周杰伦", "")];
    assert_eq!(filter_tracks(&tracks, "夜 不存在").len(), 0);
    assert_eq!(filter_tracks(&tracks, "夜 第七").len(), 1);
  }
}
