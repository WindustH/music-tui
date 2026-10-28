//! Commented config.toml generation: serialise AppConfig to TOML and
//! attach the user-facing comment table to each key.

use anyhow::Result;

use super::AppConfig;

pub fn config_comment(key: &str) -> Option<&'static str> {
  match key {
    "mpd" => Some("Connection settings for the MPD daemon."),
    "mpd.host" => Some(
      "MPD host. A path starting with / or ~ connects over a unix socket. First-run MPD setup uses ~/.config/mpd/socket so local file:// songs need no music directory.",
    ),
    "mpd.port" => Some("MPD TCP port."),
    "mpd.password" => Some("Optional MPD password."),
    "mpd.music_dir" => Some(
      "Optional local music root. Empty reads music_directory from mpd.conf; file:// songs over a unix socket work without either setting.",
    ),
    "mpd.link_dir" => Some(
      "Where files outside the music directory are linked (copied on Windows) so MPD can play them over TCP. Empty uses <music_dir>/.music-tui-links.",
    ),
    "behavior" => Some("Interactive behavior settings."),
    "behavior.tick_ms" => Some(
      "Status poll interval (ms) while paused or stopped; MPD also pushes every change as it happens.",
    ),
    "behavior.playing_tick_ms" => {
      Some("Status poll interval (ms) while playing: drives the progress band and synced lyrics.")
    }
    "behavior.queue_dedup" => Some(
      "Duplicate handling: adding a song that is already queued is skipped, and the queue view hides extra copies of the same song (the first copy and the playing copy stay visible). The existing queue is never modified.",
    ),
    "render" => Some("Cover art rendering settings."),
    "render.raw_memory_cache_max_bytes" => {
      Some("Cover render memory budget in bytes; 0 means unlimited.")
    }
    "render.disk_cache_max_bytes" => {
      Some("SQLite cache payload budget in bytes; 0 means unlimited.")
    }
    "render.chafa_bin" => {
      Some("Command used to render cover art when no graphics protocol is available.")
    }
    "render.auto_detect" => Some("Detect terminal graphics capability automatically."),
    "render.chafa_args" => Some("Extra arguments passed to Chafa."),
    "render.chafa_threads" => Some("Threads requested per Chafa render job."),
    "render.passthrough" => Some(
      "Graphics escape passthrough: tmux or screen, none to disable; unset detects the multiplexer.",
    ),
    "render.zellij_sixel" => Some("Allow Sixel images inside Zellij."),
    "visualizer" => Some("Spectrum visualizer settings."),
    "visualizer.fifo_path" => Some("MPD fifo output path feeding the visualizer."),
    "visualizer.sample_rate" => Some("Sample rate of the fifo audio_output format."),
    "visualizer.channels" => Some("Channel count of the fifo audio_output format."),
    "visualizer.bars" => Some(
      "Maximum band count; the analysis follows the pane width (one band per column) up to this cap. Wider panes use equal-width, centered bars.",
    ),
    "visualizer.fps" => Some("Spectrum analysis updates per second."),
    "visualizer.window" => {
      Some("FFT window size in samples (256-8192, rounded up to a power of two).")
    }
    "lyrics" => Some("Lyrics loading settings."),
    "lyrics.extra_dirs" => {
      Some("Extra directories searched for `<song>.lrc` and `<artist> - <title>.lrc` files.")
    }
    "lyrics.follow" => Some("Follow playback when synced lyrics are available."),
    "playlist" => Some("Playlist file handling (`:save`, `open` on .m3u/.pls/.txt files)."),
    "playlist.save_dir" => Some(
      "Directory for `:save` exports; empty uses the playlists folder in the state directory (~/.local/state/music-tui/playlists on Linux). Bare `:save` names resolve here.",
    ),
    "library" => Some(
      "Local library pane: music-tui scans these folders into its own database (they may differ from MPD's music directory).",
    ),
    "library.paths" => Some("Folders to index; empty disables the library."),
    "library.recursive" => {
      Some("Scan subfolders too. Folders containing a .nomedia file are skipped.")
    }
    "library.columns" => Some(
      "Library table columns in order: field is title, artist, album, genre, filename or duration; width is a relative weight.",
    ),
    "layout" => Some(
      "Tab layout. Each tab is a layout tree like H(2:1, queue, V(2:1, cover:hovered, metadata:hovered)) with a main pane that receives its keys. cover/lyrics/metadata panes take an optional :playing, :hovered or :library-hovered source suffix.",
    ),
    "layout.detail" => Some(
      "Secondary detail view (i) layout over the cover and metadata panes, e.g. H(2:1, cover, metadata).",
    ),
    "layout.tabs" => Some("Tabs shown in the tab bar, in order."),
    _ => None,
  }
}

pub fn app_config_toml(config: &AppConfig) -> Result<String> {
  let body = toml::to_string_pretty(config)?;
  Ok(framework_tui::commented_toml::add_comments(
    &body,
    &[
      "music-tui main configuration.",
      "Missing fields are rewritten with defaults when the app loads this file.",
    ],
    config_comment,
    str::to_string,
  ))
}
