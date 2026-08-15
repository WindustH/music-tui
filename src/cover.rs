//! Cover art discovery: embedded pictures first, then sibling image files
//! (canonical names like `cover.jpg`, finally any image in the folder)
//! extracted into the cache directory when embedded.

use std::path::{Path, PathBuf};

use lofty::{prelude::*, read_from_path};
use sha2::{Digest, Sha256};

const COVER_NAMES: &[&str] = &["cover", "folder", "front", "albumart", "album"];
const COVER_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif"];

/// Find a cover image for `file`: embedded picture, then images in the
/// same folder.
pub fn find_cover(file: &Path, covers_cache_dir: &Path) -> Result<PathBuf, String> {
  if let Ok(path) = extract_embedded_cover(file, covers_cache_dir) {
    return Ok(path);
  }
  find_sibling_cover(file).ok_or_else(|| "no cover art found".to_string())
}

fn find_sibling_cover(file: &Path) -> Option<PathBuf> {
  let parent = file.parent()?;
  let stem = file.file_stem()?.to_string_lossy().to_ascii_lowercase();

  // Canonical names first, in a stable priority order.
  let mut candidates: Vec<PathBuf> = Vec::new();
  for name in COVER_NAMES {
    for ext in COVER_EXTENSIONS {
      candidates.push(parent.join(format!("{name}.{ext}")));
      candidates.push(parent.join(format!("{name}.{}", ext.to_uppercase())));
    }
  }
  for ext in COVER_EXTENSIONS {
    candidates.push(parent.join(format!("{stem}.{ext}")));
    candidates.push(parent.join(format!("{stem}.{}", ext.to_uppercase())));
  }
  if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
    return Some(path);
  }

  // Fallback: any image in the folder (sorted for a stable pick).
  let entries = std::fs::read_dir(parent).ok()?;
  let mut images: Vec<PathBuf> = entries
    .filter_map(|entry| entry.ok())
    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
    .map(|entry| entry.path())
    .filter(|path| {
      path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| COVER_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
    })
    .collect();
  images.sort();
  images.into_iter().next()
}

fn extract_embedded_cover(file: &Path, covers_cache_dir: &Path) -> Result<PathBuf, String> {
  let tagged = read_from_path(file).map_err(|error| format!("failed to read tags: {error}"))?;
  let tag = tagged
    .primary_tag()
    .or_else(|| tagged.first_tag())
    .ok_or_else(|| "no embedded tags".to_string())?;
  let picture = tag
    .pictures()
    .first()
    .ok_or_else(|| "no embedded cover art".to_string())?;
  let data = picture.data();
  if data.is_empty() {
    return Err("embedded cover art is empty".to_string());
  }

  let ext = sniff_image_extension(data).unwrap_or("bin");
  let mut hasher = Sha256::new();
  hasher.update(data);
  let digest = hex::encode(&hasher.finalize()[..16]);
  let path = covers_cache_dir.join(format!("{digest}.{ext}"));
  if !path.exists() {
    std::fs::create_dir_all(covers_cache_dir)
      .map_err(|error| format!("failed to create covers cache: {error}"))?;
    std::fs::write(&path, data).map_err(|error| format!("failed to write cover cache: {error}"))?;
  }
  Ok(path)
}

fn sniff_image_extension(data: &[u8]) -> Option<&'static str> {
  if data.starts_with(&[0xff, 0xd8, 0xff]) {
    Some("jpg")
  } else if data.starts_with(&[0x89, b'P', b'N', b'G']) {
    Some("png")
  } else if data.starts_with(b"GIF8") {
    Some("gif")
  } else if data.starts_with(b"BM") {
    Some("bmp")
  } else if data.len() > 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
    Some("webp")
  } else {
    None
  }
}
