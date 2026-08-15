//! Queue label helpers.
//!
//! MPD merges every tag source it can decode; legacy single-byte tags
//! (e.g. GBK RIFF INFO inside WAV files) fail its UTF-8 validation and come
//! through as runs of literal `?` — before the properly encoded duplicate
//! (ID3v2) values. These helpers skip corrupted values, and queue a lofty
//! re-read for songs whose *only* values are corrupted.

use super::*;
use lofty::prelude::*;
use mpd_client::tag::Tag;
use std::path::Path;

/// True for values MPD produced by replacing non-UTF-8 bytes with `?`:
/// question marks (and punctuation) but no alphanumeric character of any
/// script. Genuine titles like `Is This It?` still pass.
fn looks_corrupted(value: &str) -> bool {
  value.contains('?') && !value.chars().any(char::is_alphanumeric)
}

/// True for values likely produced by replacing non-UTF-8 bytes with `?`:
/// a run of consecutive `?` (multi-byte encodings map one character to
/// several `?`) or a fully corrupted value. Genuine titles like
/// `Is This It?` still pass.
fn looks_suspect(value: &str) -> bool {
  value.contains("??") || looks_corrupted(value)
}

/// Pick the best value: prefer one without any `?`, then one that is not
/// fully corrupted, then the first non-empty one.
fn clean_value(values: &[String]) -> Option<&str> {
  let non_empty: Vec<&str> = values
    .iter()
    .map(|value| value.trim())
    .filter(|value| !value.is_empty())
    .collect();
  non_empty
    .iter()
    .copied()
    .find(|value| !looks_suspect(value))
    .or_else(|| {
      non_empty
        .iter()
        .copied()
        .find(|value| !looks_corrupted(value))
    })
    .or_else(|| non_empty.first().copied())
}

/// The best available title for `song` (skips `?`-corrupted duplicates).
pub(crate) fn song_title(song: &Song) -> Option<&str> {
  clean_value(song.tags.get(&Tag::Title)?)
}

/// The best available artist for `song` (skips `?`-corrupted duplicates).
pub(crate) fn song_artist(song: &Song) -> Option<&str> {
  clean_value(song.tags.get(&Tag::Artist)?)
}

/// `title — artist` display label used by the queue, footer and detail view.
fn queue_label(song: &Song) -> String {
  let title = song_title(song).map(str::to_string).unwrap_or_else(|| song.url.clone());
  match song_artist(song) {
    Some(artist) if !artist.is_empty() => format!("{title} — {artist}"),
    _ => title,
  }
}

/// True when a lofty re-read could improve on the tags MPD reported: no
/// title or artist value is free of `?` (all suspect or empty).
fn needs_fallback(titles: Option<&Vec<String>>, artists: Option<&Vec<String>>) -> bool {
  let bad = |values: Option<&Vec<String>>| match values {
    Some(values) => values
      .iter()
      .all(|value| value.trim().is_empty() || looks_suspect(value)),
    None => false, // missing tag: nothing better to read
  };
  bad(titles) || bad(artists)
}

impl App {
  /// After a queue snapshot: re-read tags for songs whose MPD values are
  /// all corrupted (guarded by an in-flight set so each song loads once).
  pub(crate) fn scan_queue_labels(&mut self) {
    if self.music_dir.is_none() {
      return;
    }
    for song in &self.queue {
      let url = song.song.url.as_str();
      if !needs_fallback(song.song.tags.get(&Tag::Title), song.song.tags.get(&Tag::Artist))
        || self.tag_fallbacks_done.contains(url)
        || self.tag_fallbacks_pending.contains(url)
      {
        continue;
      }
      let Some(path) = self.song_path(url) else {
        continue;
      };
      self.tag_fallbacks_pending.insert(url.to_string());
      let url = url.to_string();
      let tx = self.events.clone();
      tokio::task::spawn_blocking(move || {
        let (title, artist) = read_labels(&path);
        let _ = tx.send(AsyncEvent::Mpd(MpdEvent::TagFallback { url, title, artist }));
      });
    }
  }

  /// Apply a lofty re-read result to the queue (and open detail/hover
  /// views) so every display site picks up the clean values.
  pub(crate) fn apply_tag_fallback(
    &mut self,
    url: &str,
    title: Option<String>,
    artist: Option<String>,
  ) -> bool {
    self.tag_fallbacks_pending.remove(url);
    self.tag_fallbacks_done.insert(url.to_string());
    for song in &mut self.queue {
      if song.song.url != url {
        continue;
      }
      if let Some(title) = title.clone() {
        song.song.tags.insert(Tag::Title, vec![title]);
      }
      if let Some(artist) = artist.clone() {
        song.song.tags.insert(Tag::Artist, vec![artist]);
      }
      let label = queue_label(&song.song);
      if let Some(detail) = self.detail.as_mut()
        && detail.url == url
      {
        detail.title = label.clone();
      }
      if let Some(hover) = self.hover.as_mut()
        && hover.url == url
      {
        hover.title = label;
      }
    }
    true
  }
}

/// Read title/artist straight from the file's tags with lofty, searching
/// every tag block (primary first): files can carry a corrupted legacy
/// block (e.g. GBK RIFF INFO) next to a clean one (ID3v2).
fn read_labels(path: &Path) -> (Option<String>, Option<String>) {
  let Ok(tagged) = lofty::read_from_path(path) else {
    return (None, None);
  };
  let primary_type = tagged.primary_tag().map(|tag| tag.tag_type());
  let mut blocks: Vec<&lofty::tag::Tag> = tagged.tags().iter().collect();
  blocks.sort_by_key(|tag| Some(tag.tag_type()) != primary_type);
  let pick =
    |read: fn(&lofty::tag::Tag) -> Option<std::borrow::Cow<'_, str>>| -> Option<String> {
      blocks
        .iter()
        .flat_map(|tag| read(tag))
        .find(|value| !looks_suspect(value) && !value.trim().is_empty())
        .map(|value| value.into_owned())
    };
  (pick(|tag| tag.title()), pick(|tag| tag.artist()))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn detects_corrupted_values() {
    assert!(looks_corrupted("?????"));
    assert!(looks_corrupted("???, ??"));
    assert!(!looks_corrupted("Is This It?"));
    assert!(!looks_corrupted("爱你?"));
    assert!(!looks_corrupted("夜的第七章"));
  }

  #[test]
  fn clean_value_prefers_valid_duplicate() {
    let values = vec!["?????".to_string(), "夜的第七章".to_string()];
    assert_eq!(clean_value(&values), Some("夜的第七章"));
  }

  #[test]
  fn clean_value_skips_partially_corrupted() {
    // `???, Lara???` mixes corruption with kept ASCII words — it must
    // lose against the fully clean duplicate from the other tag block.
    let values = vec!["???, Lara???".to_string(), "周杰伦 / Lara梁心颐".to_string()];
    assert_eq!(clean_value(&values), Some("周杰伦 / Lara梁心颐"));
  }

  #[test]
  fn clean_value_falls_back_to_first() {
    let values = vec!["?????".to_string()];
    assert_eq!(clean_value(&values), Some("?????"));
  }

  #[test]
  fn all_corrupted_requests_fallback() {
    let titles = vec!["?????".to_string()];
    let artists = vec!["???, Lara???".to_string()];
    assert!(needs_fallback(Some(&titles), Some(&artists)));
    let good = vec!["Is This It?".to_string()];
    assert!(!needs_fallback(Some(&good), None));
    assert!(!needs_fallback(None, None));
    // A clean duplicate anywhere keeps MPD's copy usable.
    let mixed = vec!["???".to_string(), "珊瑚海".to_string()];
    assert!(!needs_fallback(Some(&mixed), None));
  }
}
