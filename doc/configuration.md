# Configuration

Configuration lives in `~/.config/music-tui/`:

- `config.toml` — general settings (below)
- `keymap.toml` — see [Keymap](keymap.md)
- `theme.toml` — see [Theme](theme.md)

Files are created with commented defaults on first run. An incompatible or
invalid file is backed up (`config.toml.bak.<timestamp>`) and rewritten.

## `config.toml`

```toml
[mpd]
host = "127.0.0.1"   # a path starting with / selects a UNIX socket
port = 6600
password = ""        # optional MPD password
music_dir = ""       # optional; auto-detected from ~/.config/mpd/mpd.conf

[behavior]
tick_ms = 1000         # status refresh while idle
playing_tick_ms = 200  # status refresh while playing

[render]
chafa_bin = "chafa"    # Chafa binary for symbol/ASCII rendering
auto_detect = true     # probe terminal graphics support
chafa_args = []        # extra args, e.g. ["--colors", "256"]
chafa_threads = 0      # 0 = Chafa default
passthrough = ""       # optional escape passthrough (Zellij etc.)
zellij_sixel = false   # advertise Sixel inside Zellij

[visualizer]
fifo_path = "/tmp/mpd.fifo"  # MPD fifo output path
sample_rate = 44100          # must match the fifo format
channels = 2
bars = 256                   # bar cap; analysis follows the pane width
fps = 30
window = 2048                # FFT window in samples (256..=8192)

[lyrics]
extra_dirs = []   # extra lookup dirs for `<artist> - <title>.lrc`
follow = true     # auto-scroll synced lyrics

[playlist]
save_dir = ""     # `:save` destination dir; empty = ~/.local/state/music-tui/playlists

[layout]
detail = "H(2:1, cover, metadata)"  # secondary detail view (`i`)

[[layout.tabs]]
name = "playlist"
layout = "H(2:1, queue, V(2:1, cover, metadata))"
main = "queue"
```

## Layout DSL

Each tab's `layout` is a tree of splits and panes:

- `H(ratio, left, right)` — horizontal split (side by side)
- `V(ratio, top, bottom)` — vertical split (stacked)
- leaf panes: `queue`, `cover`, `lyrics`, `metadata`, `visualizer`

`ratio` is `a:b` (e.g. `2:1` — left pane gets two thirds). Splits nest
freely:

```text
H(1:2, cover, V(2:1, lyrics, metadata))
```

`main` names the pane that receives key input while the tab is active; it
must appear in the tree (defaults to the first leaf).

### Pane data sources

The `cover`, `lyrics` and `metadata` panes take an optional `:source`
suffix selecting which song they display:

- `playing` (default) — the currently playing song
- `hovered` — the song selected (hovered) in the queue

```text
H(2:1, queue, V(2:1, cover:hovered, lyrics:hovered))
```

A `:hovered` lyrics pane has no playback state: it renders as a plain
scrollable list without sync highlighting, follow mode, or click-to-seek
(those report "song is not playing"). Data for the hovered song loads
lazily, only when some pane uses the source.

## Detail view layout

`[layout].detail` configures the secondary detail view opened with `i`
from the queue. It is a layout tree over exactly one `cover` and one
`metadata` pane — side by side by default:

```toml
[layout]
detail = "H(2:1, cover, metadata)"   # default: cover left, metadata right
# detail = "V(2:1, cover, metadata)" # stacked instead
```
