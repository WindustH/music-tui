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
  `bars` (default 256). The pane then picks a strip count that minimizes
  `(leftover + width/8) / strips` (every band gets an equal-width strip;
  the proportional slack keeps a zero-leftover split from shrinking the
  band count) and centers the leftover columns as margins.
- Band layout and styled-line construction run on a worker thread
  (`spawn_band_renderer`); the UI thread only blits the finished lines.
- Bands are log-spaced with a minimum step of one FFT bin: where a pure
  log grid would be narrower than the FFT resolution (the low end at
  small windows), bands merge onto distinct bins instead of sampling the
  same bin twice and rendering as duplicated identical bars. A larger
  `window` resolves more low-frequency bands.
- The fifo is read non-blocking; nothing is written to it. If the fifo is
  missing or MPD is not playing, the pane simply stays flat.
- Higher `window` gives finer frequency resolution, lower latency the
  opposite; 2048 at 44.1 kHz is a good default.
