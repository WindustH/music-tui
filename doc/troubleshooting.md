# Troubleshooting

## Cannot connect / "connection lost"

- Check that MPD is running (`systemctl --user status mpd`) and that
  `mpd.host`/`mpd.port` in `~/.config/music-tui/config.toml` match
  `bind_to_address`/`port` in `mpd.conf`.
- A host starting with `/` is treated as a UNIX socket path.
- On first run without an MPD config, music-tui generates a local socket
  config (`~/.mpd/mpd.conf` on macOS). A missing socket file means the MPD
  daemon has not started yet; start/restart its system service.
- music-tui reconnects automatically with backoff; the footer shows the
  connection state.

## Covers or lyrics missing

- Songs queued as local `file://` URIs resolve without a music directory.
  Relative MPD song URIs require `mpd.music_dir` or a readable
  `music_directory` in one of MPD's normal config locations.
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
- If the fifo file was deleted while mpd kept running (e.g. a `/tmp`
  cleaner), mpd keeps writing to the unlinked inode and no reader can
  reconnect. music-tui recreates a missing fifo itself, but a wedged mpd
  writer needs a restart: `systemctl --user restart mpd` (state file
  restores the queue).

## Cover renders as symbols/ASCII on a capable terminal

- Check `MUSIC_TUI_RENDER_MODES` and `render.auto_detect`. Zellij 0.45+ KGP is
  auto-detected; set `render.zellij_sixel = true` only to additionally allow
  Sixel.

## Metadata edit does not stick

- The file must be writable and the format must support the tag
  (e.g. `.wav` has no standard tag layer for some fields).
- Check `~/.cache/music-tui/music-tui.log` for the write error.

## Logs, cache and state

- Logs: `~/.cache/music-tui/music-tui.log` (`RUST_LOG` to raise verbosity).
- Cover cache: `~/.cache/music-tui/covers/` (safe to delete).
- State (library database, session state): `~/.local/state/music-tui/`
  (`library.db`, `state.toml`; migrated from the cache dir automatically).

## Running several instances at once

Multiple `music-tui` processes against the same MPD and config are safe:

- `state.toml` saves go through per-PID temp files and atomic renames —
  the last instance to exit wins, no corruption possible.
- The cover cache publishes files with rename (no half-written images).
- `library.db` uses WAL with a 5 s busy timeout, so concurrent rescans
  queue up instead of failing with `SQLITE_BUSY`.
- Queue auto-dedup deletes duplicates by stable song id, so two
  instances cleaning the same queue never remove the wrong song; the
  loser of a race just logs "no such song".
- The log file is opened in append mode by every instance; lines may
  interleave but stay readable.

Two caveats:

- **Visualizer fifo is single-reader.** The first instance locks the
  fifo (`flock`); later instances show the waiting hint instead of a
  garbled spectrum, and take over when the first one exits. Point extra
  instances at another fifo (mpd can feed several fifo outputs) via
  `[visualizer] fifo_path` if you need visuals everywhere.
- Both instances watch the same queue via `idle`, so actions from one
  show up in the other within a tick — but each keeps its own tab,
  selection and filters.
