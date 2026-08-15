# Troubleshooting

## Cannot connect / "connection lost"

- Check that MPD is running (`systemctl --user status mpd`) and that
  `mpd.host`/`mpd.port` in `~/.config/music-tui/config.toml` match
  `bind_to_address`/`port` in `mpd.conf`.
- A host starting with `/` is treated as a UNIX socket path.
- music-tui reconnects automatically with backoff; the footer shows the
  connection state.

## Covers or lyrics missing

- Both require the music directory: verify `mpd.music_dir` or that
  `~/.config/mpd/mpd.conf` has a readable `music_directory`.
- Covers accept embedded pictures and sibling files (`cover.*`, `folder.*`,
  `front.*`, `<basename>.*`). Minimum pane size applies.
- Lyrics lookup order is documented in [Lyrics](lyrics.md).

## Visualizer stays flat

- MPD must have the fifo output enabled and playing (see
  [Visualizer](visualizer.md)); `fifo_path`, `sample_rate`, and `channels`
  must match the `format` line.
- Some output chains (e.g. certain PipeWire setups with exclusive access) do
  not feed secondary outputs; check that the fifo output is not disabled
  (`mpc outputs`).

## Cover renders as symbols/ASCII on a capable terminal

- Check `MUSIC_TUI_RENDER_MODES` and `render.auto_detect`; inside Zellij set
  `render.zellij_sixel = true` or a `render.passthrough`.

## Metadata edit does not stick

- The file must be writable and the format must support the tag
  (e.g. `.wav` has no standard tag layer for some fields).
- Check `~/.cache/music-tui/music-tui.log` for the write error.

## Logs and cache

- Logs: `~/.cache/music-tui/music-tui.log` (`RUST_LOG` to raise verbosity).
- Cover cache: `~/.cache/music-tui/covers/` (safe to delete).
