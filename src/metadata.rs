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
  push_file_entry(&mut entries, "duration", format_duration(properties.duration()));
  if let Some(bitrate) = properties.overall_bitrate() {
    push_file_entry(&mut entries, "bitrate", format!("{bitrate} kbps"));
  }
  if let Some(rate) = properties.sample_rate() {
    push_file_entry(&mut entries, "sample rate", format!("{rate} Hz"));
  }
  if let Some(bits) = properties.bit_depth() {
    push_file_entry(&mut entries, "bit depth", format!("{bits} bits"));
  }

  let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
  match tag {
    Some(tag) => {
      for name in EDITABLE_TAGS {
        let value = read_tag_value(tag, name).unwrap_or_default();
        push_tag_entry(&mut entries, name, value);
      }
    }
    None => push_tag_entry(&mut entries, "Tags", "no tags".to_string()),
  }

  Ok(entries)
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
  out
}

/// Diff an edited draft against the original entries.
pub fn metadata_changes(entries: &[MetadataEntry], edited: &str) -> Result<Vec<MetadataChange>, String> {
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

  let mut changes = Vec::new();
  for (tag, new_value) in metadata {
    if !EDITABLE_TAGS.contains(&tag.as_str()) {
      return Err(format!("unsupported tag: {tag}"));
    }
    let Some(new_value) = new_value.as_str() else {
      return Err(format!("tag {tag} must be a string"));
    };
    let old_value = original.get(tag.as_str()).cloned();
    if old_value.as_deref().unwrap_or_default() != new_value {
      changes.push(MetadataChange {
        tag: tag.to_string(),
        old_value,
        new_value: new_value.to_string(),
      });
    }
  }
  Ok(changes)
}

/// Apply changes to the file's primary tag.
pub fn write_metadata(path: &Path, changes: &[MetadataChange]) -> Result<usize, String> {
  if changes.is_empty() {
    return Ok(0);
  }
  let mut tagged = read_from_path(path).map_err(|error| format!("failed to read tags: {error}"))?;
  let primary_type = tagged.primary_tag().map(|tag| tag.tag_type());

  let edited = {
    let tag = match tagged.primary_tag_mut() {
      Some(tag) => tag,
      None => {
        let tag_type = primary_type
          .unwrap_or_else(|| tagged.file_type().primary_tag_type());
        tagged.insert_tag(lofty::tag::Tag::new(tag_type));
        tagged.primary_tag_mut().expect("tag just inserted")
      }
    };
    for change in changes {
      write_tag_value(tag, &change.tag, &change.new_value);
    }
    tag.clone()
  };

  edited
    .save_to_path(path, lofty::config::WriteOptions::default())
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
          value.split(['/', ':']).next().unwrap_or("").trim().to_string()
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
