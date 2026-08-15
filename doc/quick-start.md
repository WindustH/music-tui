# Quick Start

`music-tui` is a client for an already running [MPD](https://www.musicpd.org/)
daemon. Make sure MPD is running and reachable (default `127.0.0.1:6600`),
then start the client:

```sh
music-tui
```

The music directory is auto-detected from `~/.config/mpd/mpd.conf`
(`music_directory`); it can also be set explicitly in `config.toml`
(`mpd.music_dir`). The directory is needed for covers, lyrics, and metadata
editing — queue-only usage works without it.

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
