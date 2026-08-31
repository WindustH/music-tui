//! Tag metadata reading, editing drafts, and writing, following the
//! pdf-tui / gallery-tui editor flow: `e` opens a TOML draft in $EDITOR,
//! changes are diffed and written back with lofty.

use std::{path::Path, time::Duration};

use lofty::{prelude::*, read_from_path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataEntry {
  pub group: String,
  pub name: String,
  pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataChange {
  pub tag: String,
  pub old_value: Option<String>,
  pub new_value: String,
}

pub const EDITABLE_TAGS: &[&str] = &[
  "Title",
  "Artist",
  "Album",
  "AlbumArtist",
  "Genre",
  "Year",
  "Track",
  "Disk",
  "Composer",
  "Comment",
];

pub fn read_metadata(path: &Path) -> Result<Vec<MetadataEntry>, String> {
  let tagged = read_from_path(path).map_err(|error| format!("failed to read tags: {error}"))?;
  let mut entries = Vec::new();

  let properties = tagged.properties();
  push_file_entry(
    &mut entries,
    "duration",
    format_duration(properties.duration()),
  );
  if let Some(bitrate) = properties.overall_bitrate() {
    push_file_entry(&mut entries, "bitrate", format!("{bitrate} kbps"));
  }
  if let Some(rate) = properties.sample_rate() {
    push_file_entry(&mut entries, "sample rate", format!("{rate} Hz"));
  }
  if let Some(bits) = properties.bit_depth() {
    push_file_entry(&mut entries, "bit depth", format!("{bits} bits"));
  }

  // Files can carry several tag blocks (e.g. WAV with a GBK RIFF INFO
  // next to a clean ID3v2). The primary block becomes the editable
  // `[metadata]` draft surface (group "tag"); every extra block is listed
  // with a source prefix so corrupted duplicates are visible and —
  // because writes hit every block — correctable.
  let primary_type = tagged.primary_tag().map(|tag| tag.tag_type());
  let mut blocks: Vec<&lofty::tag::Tag> = tagged.tags().iter().collect();
  blocks.sort_by_key(|tag| Some(tag.tag_type()) != primary_type);
  if blocks.is_empty() {
    push_tag_entry(&mut entries, "Tags", "no tags".to_string());
  }
  for tag in blocks {
    let is_primary = Some(tag.tag_type()) == primary_type;
    for name in EDITABLE_TAGS {
      let Some(value) = read_tag_value(tag, name) else {
        continue;
      };
      if is_primary {
        push_tag_entry(&mut entries, name, value);
      } else {
        let label = tag_block_label(tag.tag_type());
        entries.push(MetadataEntry {
          group: format!("tag/{label}"),
          name: format!("{label} {name}"),
          value,
        });
      }
    }
  }

  Ok(entries)
}

/// Short human label for a non-primary tag block.
fn tag_block_label(tag_type: lofty::tag::TagType) -> String {
  let debug = format!("{tag_type:?}").to_lowercase();
  match debug.as_str() {
    "riffinfo" => "riff".to_string(),
    "vorbiscomments" => "vorbis".to_string(),
    other => other.to_string(),
  }
}

fn push_file_entry(entries: &mut Vec<MetadataEntry>, name: &str, value: String) {
  entries.push(MetadataEntry {
    group: "file".to_string(),
    name: name.to_string(),
    value,
  });
}

fn push_tag_entry(entries: &mut Vec<MetadataEntry>, name: &str, value: String) {
  entries.push(MetadataEntry {
    group: "tag".to_string(),
    name: name.to_string(),
    value,
  });
}

fn read_tag_value(tag: &lofty::tag::Tag, name: &str) -> Option<String> {
  match name {
    "Title" => tag.title().map(|value| value.into_owned()),
    "Artist" => tag.artist().map(|value| value.into_owned()),
    "Album" => tag.album().map(|value| value.into_owned()),
    "Genre" => tag.genre().map(|value| value.into_owned()),
    "Year" => tag.year().map(|value| value.to_string()),
    "Track" => tag.track().map(|value| value.to_string()),
    "Disk" => tag.disk().map(|value| value.to_string()),
    "AlbumArtist" => tag.get_string(&ItemKey::AlbumArtist).map(str::to_string),
    "Composer" => tag.get_string(&ItemKey::Composer).map(str::to_string),
    "Comment" => tag.comment().map(|value| value.into_owned()),
    _ => None,
  }
}

/// Build the TOML draft opened in $EDITOR.
pub fn metadata_draft(path: &Path, entries: &[MetadataEntry]) -> String {
  let mut out = String::new();
  out.push_str("# Edit music tags. Save and exit to apply.\n");
  out.push_str("# Empty strings clear the field.\n");
  out.push_str(&format!("# file = {:?}\n\n", path.display().to_string()));
  out.push_str("[metadata]\n");
  let mut values: std::collections::BTreeMap<&str, String> = entries
    .iter()
    .filter(|entry| entry.group == "tag")
    .map(|entry| (entry.name.as_str(), entry.value.clone()))
    .collect();
  for tag in EDITABLE_TAGS {
    let value = values.remove(*tag).unwrap_or_default();
    out.push_str(&format!("{tag} = {}\n", toml_string(&value)));
  }
  // Extra tag blocks are not directly editable but are listed so their
  // state (e.g. corrupted duplicates) is visible; saving overwrites them
  // with the values above.
  let extras: Vec<&MetadataEntry> = entries
    .iter()
    .filter(|entry| entry.group.starts_with("tag/"))
    .collect();
  if !extras.is_empty() {
    out.push_str("\n# Other tag blocks in this file are overwritten with the values\n");
    out.push_str("# above on save. Their current values:\n");
    for entry in extras {
      out.push_str(&format!("# {} = {}\n", entry.name, entry.value));
    }
  }
  out
}

/// Diff an edited draft against the original entries. A field counts as
/// changed when it differs from the primary block **or from any extra
/// block** — saving then normalizes every block to the draft value, so a
/// corrupted duplicate (e.g. GBK RIFF INFO) is fixed even when the primary
/// value already matches the draft.
pub fn metadata_changes(
  entries: &[MetadataEntry],
  edited: &str,
) -> Result<Vec<MetadataChange>, String> {
  let value = edited
    .parse::<toml::Table>()
    .map_err(|err| format!("metadata draft is not valid TOML: {err}"))?;
  let metadata = value
    .get("metadata")
    .and_then(toml::Value::as_table)
    .ok_or("metadata draft must contain a [metadata] table")?;

  let original: std::collections::BTreeMap<String, String> = entries
    .iter()
    .filter(|entry| entry.group == "tag" && EDITABLE_TAGS.contains(&entry.name.as_str()))
    .map(|entry| (entry.name.clone(), entry.value.clone()))
    .collect();
  // Extra blocks store their field as "<label> <Tag>" (e.g. "riff Title").
  let extras: Vec<(String, String)> = entries
    .iter()
    .filter(|entry| entry.group.starts_with("tag/"))
    .filter_map(|entry| {
      entry
        .name
        .split_once(' ')
        .map(|(_, tag)| (tag.to_string(), entry.value.clone()))
    })
    .collect();

  let mut changes = Vec::new();
  for (tag, new_value) in metadata {
    if !EDITABLE_TAGS.contains(&tag.as_str()) {
      return Err(format!("unsupported tag: {tag}"));
    }
    let Some(new_value) = new_value.as_str() else {
      return Err(format!("tag {tag} must be a string"));
    };
    let old_value = original.get(tag.as_str()).cloned();
    let differs_from_primary = old_value.as_deref().unwrap_or_default() != new_value;
    let differs_from_extra = extras.iter().any(|(extra_tag, extra_value)| {
      extra_tag.as_str() == tag.as_str() && extra_value.as_str() != new_value
    });
    if differs_from_primary || differs_from_extra {
      changes.push(MetadataChange {
        tag: tag.to_string(),
        old_value,
        new_value: new_value.to_string(),
      });
    }
  }
  Ok(changes)
}

/// Apply changes to every tag block in the file, so whichever block a
/// reader (MPD) prefers carries the corrected values. Saving persists all
/// blocks at once.
pub fn write_metadata(path: &Path, changes: &[MetadataChange]) -> Result<usize, String> {
  if changes.is_empty() {
    return Ok(0);
  }
  let mut tagged = read_from_path(path).map_err(|error| format!("failed to read tags: {error}"))?;
  if tagged.tags().is_empty() {
    let tag_type = tagged.file_type().primary_tag_type();
    tagged.insert_tag(lofty::tag::Tag::new(tag_type));
  }
  let block_types: Vec<lofty::tag::TagType> =
    tagged.tags().iter().map(|tag| tag.tag_type()).collect();
  for tag_type in block_types {
    let Some(tag) = tagged.tag_mut(tag_type) else {
      continue;
    };
    for change in changes {
      write_tag_value(tag, &change.tag, &change.new_value);
    }
  }

  let mut file = std::fs::OpenOptions::new()
    .read(true)
    .write(true)
    .open(path)
    .map_err(|error| format!("failed to open file for writing: {error}"))?;
  tagged
    .save_to(&mut file, lofty::config::WriteOptions::default())
    .map_err(|error| format!("failed to write tags: {error}"))?;
  Ok(changes.len())
}

fn write_tag_value(tag: &mut lofty::tag::Tag, name: &str, value: &str) {
  let empty = value.trim().is_empty();
  match name {
    "Title" => {
      if empty {
        tag.remove_title();
      } else {
        tag.set_title(value.to_string());
      }
    }
    "Artist" => {
      if empty {
        tag.remove_artist();
      } else {
        tag.set_artist(value.to_string());
      }
    }
    "Album" => {
      if empty {
        tag.remove_album();
      } else {
        tag.set_album(value.to_string());
      }
    }
    "Genre" => {
      if empty {
        tag.remove_genre();
      } else {
        tag.set_genre(value.to_string());
      }
    }
    "Comment" => {
      if empty {
        tag.remove_comment();
      } else {
        tag.set_comment(value.to_string());
      }
    }
    "Year" | "Track" | "Disk" => {
      let key = match name {
        "Year" => ItemKey::Year,
        "Track" => ItemKey::TrackNumber,
        _ => ItemKey::DiscNumber,
      };
      if empty {
        tag.remove_key(&key);
      } else {
        let normalized = if value.trim().parse::<u32>().is_ok() {
          value.trim().to_string()
        } else {
          value
            .split(['/', ':'])
            .next()
            .unwrap_or("")
            .trim()
            .to_string()
        };
        if normalized.is_empty() {
          tag.remove_key(&key);
        } else {
          tag.insert_text(key, normalized);
        }
      }
    }
    "AlbumArtist" | "Composer" => {
      let key = if name == "AlbumArtist" {
        ItemKey::AlbumArtist
      } else {
        ItemKey::Composer
      };
      if empty {
        tag.remove_key(&key);
      } else {
        tag.insert_text(key, value.to_string());
      }
    }
    _ => {}
  }
}

fn format_duration(duration: Duration) -> String {
  let total = duration.as_secs();
  format!("{}:{:02}", total / 60, total % 60)
}

fn toml_string(value: &str) -> String {
  let mut out = String::with_capacity(value.len() + 2);
  out.push('"');
  for ch in value.chars() {
    match ch {
      '\\' => out.push_str("\\\\"),
      '"' => out.push_str("\\\""),
      '\n' => out.push_str("\\n"),
      '\r' => out.push_str("\\r"),
      '\t' => out.push_str("\\t"),
      ch if ch.is_control() => out.push(' '),
      ch => out.push(ch),
    }
  }
  out.push('"');
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  fn tag_entry(name: &str, value: &str) -> MetadataEntry {
    MetadataEntry {
      group: "tag".to_string(),
      name: name.to_string(),
      value: value.to_string(),
    }
  }

  fn extra_entry(label: &str, name: &str, value: &str) -> MetadataEntry {
    MetadataEntry {
      group: format!("tag/{label}"),
      name: format!("{label} {name}"),
      value: value.to_string(),
    }
  }

  #[test]
  fn draft_lists_extra_blocks() {
    let entries = vec![
      tag_entry("Title", "珊瑚海"),
      extra_entry("riff", "Title", "???"),
    ];
    let draft = metadata_draft(Path::new("/tmp/x.wav"), &entries);
    assert!(draft.contains("Title = \"珊瑚海\""));
    assert!(draft.contains("# riff Title = ???"));
  }

  #[test]
  fn unchanged_draft_still_fixes_extra_blocks() {
    // The primary value already matches the draft, but the corrupted
    // RIFF duplicate differs — saving must count as a change so the
    // write normalizes every block.
    let entries = vec![
      tag_entry("Title", "珊瑚海"),
      extra_entry("riff", "Title", "???"),
      extra_entry("riff", "Artist", "???, Lara???"),
    ];
    let edited = "[metadata]\nTitle = \"珊瑚海\"\nArtist = \"\"\n";
    let changes = metadata_changes(&entries, edited).unwrap();
    assert_eq!(changes.len(), 2);
    assert!(changes.iter().any(|change| change.tag == "Title"));
    assert!(changes.iter().any(|change| change.tag == "Artist"));
  }

  #[test]
  fn no_changes_when_all_blocks_agree() {
    let entries = vec![
      tag_entry("Title", "珊瑚海"),
      extra_entry("riff", "Title", "珊瑚海"),
    ];
    let edited = "[metadata]\nTitle = \"珊瑚海\"\n";
    assert!(metadata_changes(&entries, edited).unwrap().is_empty());
  }
}
