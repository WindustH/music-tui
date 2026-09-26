# Visualizer

The visualizer shows a spectrum of what MPD is playing: log-spaced
frequency bands, colored low / mid / high, one band per pane column (up to
`bars`). It reads raw audio from an MPD `fifo` output, so it works on Linux
and macOS but not on Windows.

## MPD setup

Add a fifo output to `mpd.conf` (the config music-tui generates on first
run has none):

```conf
audio_output {
  type    "fifo"
  name    "Visualizer feed"
  path    "/tmp/mpd.fifo"
  format  "44100:16:2"
}
```

The samples must be 16-bit. Match the path, rate, and channel count in
`config.toml`:

```toml
[visualizer]
fifo_path = "/tmp/mpd.fifo"
sample_rate = 44100   # first field of `format`
channels = 2          # last field of `format`
bars = 256            # band cap
fps = 30              # updates per second
window = 2048         # FFT size, 256–8192; larger resolves more bass bands
```

Restart MPD after editing its config.

## Notes

- The analysis only runs while the visualizer pane is on screen, and only
  on new audio: paused playback freezes the bars at no cost.
- music-tui reads the fifo without blocking MPD and recreates it if
  something (such as a `/tmp` cleaner) deletes it.
- One reader at a time: a second music-tui instance shows the waiting hint
  until the first exits. Give extra instances their own fifo output if
  you want visuals everywhere.
- `fifo_path` must be a fifo; any other file is ignored.
