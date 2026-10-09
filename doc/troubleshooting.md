# Troubleshooting

## Cannot connect / "mpd offline"

- Check that MPD is running (`systemctl --user status mpd`) and that
  `mpd.host`/`mpd.port` in `config.toml` match `bind_to_address`/`port` in
  `mpd.conf`. A host starting with `/` or `~` is a Unix socket path.
- After the first-run setup, a missing `~/.config/mpd/socket` means MPD has
  not been started yet; start or restart its service.
- music-tui reconnects on its own (with growing delays up to 30 s). Keys
  pressed while it is offline are dropped with a "mpd is not connected"
  notice rather than replayed later.

## MPD stopped starting after a cache cleanup

Before 0.1.11, the MPD config music-tui generates kept MPD's database,
state, sticker, and log files in the cache directory (`~/.cache/mpd`, or
`~/Library/Caches/mpd` on macOS). MPD refuses to start once a cleaner
removes that directory, and the state (queue) and sticker (ratings) files
are lost with it. Move them to `~/.local/state/mpd`, which newer versions
use:

```sh
mkdir -p ~/.local/state/mpd
mv ~/Library/Caches/mpd/* ~/.local/state/mpd/   # or ~/.cache/mpd/*
```

then point `db_file`, `log_file`, `state_file`, and `sticker_file` in
`mpd.conf` at the new directory and restart MPD. If the directory is
already gone, MPD rebuilds its database on the next update, but the saved
queue and ratings are lost.

## Volume keys have no effect

music-tui sets MPD's volume, which needs a mixer on an enabled output.
Without one, the volume keys show "MPD can't change the volume: No mixer"
(or the mixer's error), and the footer reads `vol:0%`. Set
`mixer_type "software"` on the output; MPD then scales its own output and
works with any device. On macOS, see also the next section.

In a config without any `audio_output`, MPD picks an output itself. Adding
an output block (a mixer fix, or the visualizer's fifo) turns that off, so
list every output you want.

## macOS audio

MPD's CoreAudio output has a few quirks; the config music-tui generates
on macOS works around the first and last:

```conf
audio_output {
  type        "osx"
  name        "CoreAudio"
  mixer_type  "software"
  format      "44100:16:2"
}
```

- **Volume keys change the wrong device.** MPD's CoreAudio mixer sets the
  system volume of the device that was the default when MPD started, so
  after a switch to headphones, AirPods, or a display the keys keep
  changing the old device, and a device without a volume control has no
  mixer at all. `mixer_type "software"` avoids both.
- **No sound after reconnecting Bluetooth headphones.** MPD keeps the
  device it found at startup. Once that device goes away, opening the
  output fails with `OSStatus error 560947818` (shown in music-tui as an
  "MPD:" notice) until MPD restarts: `brew services restart mpd`.
- **Bluetooth headphones switch to call quality.** The output sets the
  device to each song's sample rate and channels. Headphones only take a
  low-rate mono file, such as a voice recording, in call (HFP) mode, so
  they drop to 16 kHz mono. A fixed `format` makes MPD resample instead.

Restart MPD after editing its config (`brew services restart mpd`).

## Covers or lyrics missing

- Songs queued as `file://` over a Unix socket resolve without a music
  directory; relative MPD paths need `mpd.music_dir` or a readable
  `music_directory` in mpd.conf. The panes say "no local file for this
  song" when a path cannot be resolved (for example for streams).
- See [Cover Rendering](cover-rendering.md) and [Lyrics](lyrics.md) for the
  lookup order. `.lrc` files must be UTF-8.

## Cover shows as character art on a capable terminal

- Check `MUSIC_TUI_RENDER_MODES` and `render.auto_detect`. Inside Zellij,
  Kitty images need Zellij 0.45+; `render.zellij_sixel = true` additionally
  allows Sixel.

## Visualizer stays empty

- MPD needs a fifo output with 16-bit samples that is enabled and playing
  (see [Visualizer](visualizer.md)); `fifo_path`, `sample_rate`, and
  `channels` must match it. `mpc outputs` lists the enabled outputs.
- Some output setups (for example exclusive-access PipeWire chains) do not
  feed secondary outputs.
- If the fifo was deleted while MPD kept running, MPD keeps writing to the
  deleted file. music-tui recreates the fifo, but MPD only reconnects after
  a restart: `systemctl --user restart mpd`.

## Tag edit does not stick

- The file must be writable and its format must support the tag.
- The log (below) has the write error.

## Logs, cache and state

- Log: `music-tui.log` in the cache directory (`~/.cache/music-tui/`);
  set `RUST_LOG=debug` for more detail.
- Cover cache: `covers/` in the cache directory (safe to delete).
- State: `library.db`, `state.toml`, and `:save` playlists in the state
  directory (`~/.local/state/music-tui/`).
- Errors inside background workers are logged there instead of being
  printed over the interface.

## Running several instances

Several music-tui processes can share one MPD and one config:

- state, cover-cache and config writes go through temp files and atomic
  renames, so nothing is left half-written;
- `library.db` is shared: scans commit in small batches and wait for each
  other instead of failing;
- every instance follows the same queue through MPD's change
  notifications, but keeps its own tab, selection, and filters.

The visualizer fifo has a single reader: the first instance locks it and
later ones show the waiting hint until it exits. Configure another fifo
output (`[visualizer] fifo_path`) for a second instance.
