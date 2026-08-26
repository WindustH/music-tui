//! Commented config.toml generation: serialise AppConfig to TOML and
//! attach the user-facing comment table to each key.

use anyhow::Result;
use std::fmt::Write as FmtWrite;
use std::collections::BTreeSet;

use super::AppConfig;

pub fn config_comment(key: &str) -> Option<&'static str> {
  match key {
    "mpd" => Some("Connection settings for the MPD daemon."),
    "mpd.host" => Some("MPD host. A path starting with / or ~ connects over a unix socket. First-run MPD setup uses ~/.config/mpd/socket so local file:// songs need no music directory."),
    "mpd.port" => Some("MPD TCP port."),
    "mpd.password" => Some("Optional MPD password."),
    "mpd.music_dir" => Some(
      "Optional local music root. Empty reads music_directory from mpd.conf; file:// songs over a unix socket work without either setting.",
    ),
    "behavior" => Some("Interactive behavior settings."),
    "behavior.tick_ms" => Some("Status refresh interval while idle."),
    "behavior.playing_tick_ms" => Some("Status refresh interval while playing."),
    "behavior.queue_dedup" => Some("Duplicate handling: adding a song that is already queued is skipped (playback jumps to the existing entry), and the live queue is pruned to one copy per song (the playing copy wins)."),
    "render" => Some("Cover art rendering settings."),
    "render.chafa_bin" => Some("Command used to render cover art when no graphics protocol is available."),
    "render.auto_detect" => Some("Detect terminal graphics capability automatically."),
    "render.chafa_args" => Some("Extra arguments passed to Chafa."),
    "render.chafa_threads" => Some("Threads requested per Chafa render job."),
    "render.passthrough" => Some("Optional Chafa passthrough mode, such as tmux."),
    "render.zellij_sixel" => Some("Zellij SIXEL handling mode."),
    "visualizer" => Some("Spectrum visualizer settings."),
    "visualizer.fifo_path" => Some("MPD fifo output path feeding the visualizer."),
    "visualizer.sample_rate" => Some("Sample rate of the fifo audio_output format."),
    "visualizer.channels" => Some("Channel count of the fifo audio_output format."),
    "visualizer.bars" => Some("Maximum band count; the analysis follows the pane width (one band per column) up to this cap. Wider panes render equal-width bars with evenly spread gaps."),
    "visualizer.fps" => Some("Spectrum analysis updates per second."),
    "visualizer.window" => Some("FFT window size in samples."),
    "lyrics" => Some("Lyrics loading settings."),
    "lyrics.extra_dirs" => Some("Extra directories searched for `<song>.lrc` and `<artist> - <title>.lrc` files."),
    "lyrics.follow" => Some("Follow playback when synced lyrics are available."),
    "playlist" => Some("Playlist file handling (`:save`, `open` on .m3u/.pls/.txt files)."),
    "playlist.save_dir" => Some("Directory for `:save` exports; empty uses ~/.local/state/music-tui/playlists. Bare `:save` names resolve here."),
    "layout" => Some("Tab layout. Each tab is a layout tree like H(2:1, queue, V(2:1, cover:hovered, metadata:hovered)) with a main pane that receives its keys. cover/lyrics/metadata panes take an optional :playing/:hovered source suffix."),
    "layout.detail" => Some("Secondary detail view (i) layout over the cover and metadata panes, e.g. H(2:1, cover, metadata)."),
    "layout.tabs" => Some("Tabs shown in the tab bar, switched with left/right."),
    _ => None,
  }
}

pub fn app_config_toml(config: &AppConfig) -> Result<String> {
  let body = toml::to_string_pretty(config)?;
  Ok(add_app_config_comments(
    &body,
    &[
      "music-tui main configuration.",
      "Missing fields are rewritten with defaults when the app loads this file.",
    ],
    config_comment,
  ))
}

fn add_app_config_comments(
  body: &str,
  header: &[&str],
  comment_for: fn(&str) -> Option<&'static str>,
) -> String {
  let mut out = String::new();
  let mut seen_comments = BTreeSet::new();
  for line in header {
    push_toml_comment(&mut out, line);
  }
  out.push('\n');

  let mut table = String::new();
  for line in body.lines() {
    let trimmed = line.trim();
    if let Some(header) = toml_table_header(trimmed) {
      table = header.to_string();
      if seen_comments.insert(table.clone())
        && let Some(comment) = comment_for(&table)
      {
        push_toml_comment(&mut out, comment);
      }
    } else if let Some(key) = toml_field_key(trimmed) {
      let comment_key = if table.is_empty() {
        key.to_string()
      } else {
        format!("{table}.{key}")
      };
      if seen_comments.insert(comment_key.clone())
        && let Some(comment) = comment_for(&comment_key)
      {
        push_toml_comment(&mut out, comment);
      }
    }
    out.push_str(line);
    out.push('\n');
  }
  out
}

fn push_toml_comment(out: &mut String, comment: &str) {
  for line in comment.lines() {
    let _ = writeln!(out, "# {line}");
  }
}

fn toml_table_header(line: &str) -> Option<&str> {
  if line.starts_with("[[") {
    return None;
  }
  line.strip_prefix('[')?.strip_suffix(']')
}

fn toml_field_key(line: &str) -> Option<&str> {
  if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
    return None;
  }
  let (key, _) = line.split_once('=')?;
  let key = key.trim();
  (!key.is_empty()).then_some(key)
}
