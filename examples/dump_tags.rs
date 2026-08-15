//! Debug tool: dump every tag block lofty sees for a file.
//!
//! Usage:
//!   cargo run --example dump_tags -- <file>
//!   cargo run --example dump_tags -- <file> Title=... Artist=...
//!
//! With Key=Value pairs, the values are written into every tag block
//! (mirroring the app's `e` editor write path), then the result is dumped.

use lofty::prelude::*;

fn main() {
  let mut args = std::env::args().skip(1);
  let path = args.next().expect("usage: dump_tags <file> [Key=Value ...]");
  let fixes: Vec<(String, String)> = args
    .map(|arg| {
      let (key, value) = arg
        .split_once('=')
        .unwrap_or_else(|| panic!("expected Key=Value, got {arg}"));
      (key.to_string(), value.to_string())
    })
    .collect();

  let mut tagged = lofty::read_from_path(&path).expect("failed to read");

  if !fixes.is_empty() {
    let block_types: Vec<lofty::tag::TagType> =
      tagged.tags().iter().map(|tag| tag.tag_type()).collect();
    for tag_type in block_types {
      let Some(tag) = tagged.tag_mut(tag_type) else {
        continue;
      };
      for (key, value) in &fixes {
        match key.as_str() {
          "Title" => tag.set_title(value.clone()),
          "Artist" => tag.set_artist(value.clone()),
          "Album" => tag.set_album(value.clone()),
          "Year" => {
            tag.insert_text(ItemKey::Year, value.clone());
          }
          other => panic!("unsupported key: {other}"),
        }
      }
    }
    let mut file = std::fs::OpenOptions::new()
      .read(true)
      .write(true)
      .open(&path)
      .expect("failed to open for writing");
    tagged
      .save_to(&mut file, lofty::config::WriteOptions::default())
      .expect("failed to save");
    tagged = lofty::read_from_path(&path).expect("failed to re-read");
  }

  println!("file type: {:?}", tagged.file_type());
  println!("tags: {} block(s)", tagged.tags().len());
  for (index, tag) in tagged.tags().iter().enumerate() {
    println!("\n== tag[{index}] type={:?} ==", tag.tag_type());
    for item in tag.items() {
      println!("  {:?} = {:?}", item.key(), item.value());
    }
  }
  println!("\nprimary: {:?}", tagged.primary_tag().map(|tag| tag.tag_type()));
}
