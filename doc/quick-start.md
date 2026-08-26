# Quick Start

`music-tui` is a client for an [MPD](https://www.musicpd.org/) daemon. Make
sure MPD is installed, then start the client:

```sh
music-tui
```

When no MPD startup config exists, music-tui creates a minimal one on first
launch and configures both sides to use `~/.config/mpd/socket`. On macOS the
generated config is `~/.mpd/mpd.conf`; on other Unix systems it is
`$XDG_CONFIG_HOME/mpd/mpd.conf`. It does not set `music_directory` and never
overwrites an existing MPD config or a custom music-tui host. Start MPD with
your service manager after installation; music-tui reconnects automatically.

The music directory is auto-detected from the usual MPD config locations
(`music_directory`); it can also be set explicitly in `config.toml`
(`mpd.music_dir`). Neither setting is required when music-tui connects through
a UNIX socket and songs are queued as local `file://` URIs: `open`, Library
playback, covers, lyrics, and metadata editing can resolve those paths
directly. MPD-relative queue URIs and local-file playback over TCP still need
the music directory.

Default configuration files are created on first run:

- `~/.config/music-tui/config.toml`
- `~/.config/music-tui/keymap.toml`
- `~/.config/music-tui/theme.toml`

State (last tab, queue selection, lyrics follow mode) is restored between runs;
logs are written under `~/.cache/music-tui/`.

Basic workflow:

1. Switch tabs with `h`/`l`, arrow keys, or `tab` (cycles; wraps around).
2. In the queue, move with `j`/`k` and press `enter` to play a song.
3. Press `i` on a queue entry for its detail view (cover + metadata);
   `esc` returns.
4. Press `[`/`]` for previous/next, `\` to toggle pause, `-`/`=` to seek.
5. Click the progress band at the bottom to seek; drag to scrub.
6. Press `:` for the command prompt, `f1` for the key-binding help.

To hook up the visualizer, MPD needs a fifo output — see
[Visualizer](visualizer.md).
