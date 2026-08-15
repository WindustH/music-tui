# Visualizer

The spectrum visualizer reads raw PCM from an MPD `fifo` output and renders
log-spaced frequency bars (colored low/mid/high, full pane height).

## MPD setup

Add a fifo output to `mpd.conf` if you do not have one:

```conf
audio_output {
  type    "fifo"
  name    "Visualizer feed"
  path    "/tmp/mpd.fifo"
  format  "44100:16:2"
}
```

Then match it in `~/.config/music-tui/config.toml`:

```toml
[visualizer]
fifo_path = "/tmp/mpd.fifo"
sample_rate = 44100   # must match the fifo format
channels = 2
bars = 256            # band cap; analysis follows pane width
fps = 30
window = 2048         # FFT window, 256..=8192 (rounded to a power of two)
```

## Notes

- The band count follows the pane width: one band per column, capped at
  `bars` (default 256). Wider panes give every band an equal-width strip,
  with the remainder spread as evenly spaced gaps so the full width is
  used.
- The fifo is read non-blocking; nothing is written to it. If the fifo is
  missing or MPD is not playing, the pane simply stays flat.
- Higher `window` gives finer frequency resolution, lower latency the
  opposite; 2048 at 44.1 kHz is a good default.
