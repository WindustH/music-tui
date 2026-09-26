# Configuration

Configuration lives in the config directory (`~/.config/music-tui/` on
Linux and macOS, `%APPDATA%\music-tui\` on Windows — see
[Quick Start](quick-start.md#files)):

- `config.toml` — general settings (below)
- `keymap.toml` — see [Keymap](keymap.md)
- `theme.toml` — see [Theme](theme.md)

Each file is created with commented defaults on first run, and missing keys
are filled in with their defaults when music-tui starts. A file that no
longer parses (or, for `config.toml`, has an invalid layout) is moved aside
as `<name>.bak.<timestamp>` and replaced with defaults; the footer reports
it.

## `config.toml`

The defaults, with notes:

```toml
[mpd]
host = "127.0.0.1"   # a path starting with / or ~ connects over a Unix socket
port = 6600
# password = "..."   # optional
# music_dir = ""     # optional; empty = music_directory from mpd.conf
link_dir = ""        # outside files over TCP; empty = <music_dir>/.music-tui-links

[behavior]
tick_ms = 1000          # status poll while paused/stopped (100–10000)
playing_tick_ms = 200   # status poll while playing (100–10000)
queue_dedup = true      # hide duplicate queue entries, skip duplicate adds

[render]
chafa_bin = "chafa"     # character-art fallback renderer
auto_detect = true      # probe the terminal's image protocols
chafa_args = []         # extra Chafa arguments, e.g. ["--colors=256"]
chafa_threads = 0       # 0 = Chafa's default
# passthrough = "tmux"  # force tmux/screen escape wrapping, or "none"
zellij_sixel = false    # also allow Sixel inside Zellij

[visualizer]
fifo_path = "/tmp/mpd.fifo"   # path of the MPD fifo output
sample_rate = 44100           # must match the fifo format
channels = 2                  # must match the fifo format (1–8)
bars = 256                    # band cap; one band per pane column up to this
fps = 30                      # spectrum updates per second
window = 2048                 # FFT size, 256–8192 (rounded up to a power of two)

[lyrics]
extra_dirs = []   # extra folders searched for .lrc files
follow = true     # auto-scroll synced lyrics

[playlist]
save_dir = ""     # :save folder; empty = <state dir>/playlists

[library]
paths = []        # folders to index; empty disables the library
recursive = true

[[library.columns]]
field = "title"
width = 4
# … artist (3), album (3), duration (1)

[layout]
detail = "H(2:1, cover, metadata)"   # the detail view (i)

[[layout.tabs]]
name = "playlist"
layout = "H(2:1, queue, V(2:1, cover:hovered, metadata:hovered))"
main = "queue"
# … library, playing, metadata, lyrics, visualizer (see Views)
```

Notes:

- On the very first run on Linux/macOS, `mpd.host` may be set to the
  generated Unix socket (`~/.config/mpd/socket`) instead of `127.0.0.1`.
- `queue_dedup` never deletes anything from MPD's queue: adding a song that
  is already queued is skipped, and the queue view hides extra copies (the
  first copy and the playing copy stay visible; every copy shows while a
  filter is active). Toggle it with `,` `d` or `:dedup`.
- MPD pushes every change as it happens; the polls above only keep the
  elapsed time moving.

## Layout language

Each tab's `layout` is a tree of splits and panes:

- `H(a:b, left, right)` — side by side, widths shared `a:b`
- `V(a:b, top, bottom)` — stacked, heights shared `a:b`
- panes: `queue`, `library`, `cover`, `lyrics`, `metadata`, `visualizer`

Splits nest freely:

```text
H(1:2, cover, V(2:1, lyrics, metadata))
```

`main` names the pane whose keys are active on the tab (it must be in the
tree; default: the first pane). Keys the main pane does not use fall
through to the tab's other panes.

### Pane data sources

`cover`, `lyrics` and `metadata` panes take an optional `:source` suffix:

- `playing` (default) — the song MPD is playing
- `hovered` (also `queue-hovered`) — the song selected in the queue
- `library-hovered` — the track selected in the library

```text
H(2:1, queue, V(2:1, cover:hovered, lyrics:hovered))
H(2:1, library, V(2:1, cover:library-hovered, metadata:library-hovered))
```

Hovered lyrics have no playback state: they show as a plain scrollable list
(no highlighting, follow mode, or click-to-seek). Hovered data is only
loaded when some tab uses the source.

## Library

The library pane indexes the folders in `[library] paths` into its own
database (`library.db` in the state directory), independent of MPD's music
directory. Scans are incremental (by modification time) and run in the
background at startup; `u` rescans. Folders containing a `.nomedia` file
are skipped with everything below them, and tracks under them are dropped
on the next scan. Untagged files take artist and title from a
`NN. Artist - Title` file name.

Playing a library track queues its file: a relative URI inside the music
directory, `file://` over a Unix socket, or a link in `mpd.link_dir` over
TCP (a copy on Windows) — the same rules as `music-tui open`.

`[[library.columns]]` picks the table columns in order: `field` is `title`,
`artist`, `album`, `genre`, `filename`, or `duration`; `width` is a relative
weight. `/` filters every field (lyrics text included) and scrolls long
cells to the match.

## Detail view layout

`[layout].detail` arranges the detail view (`i`): exactly one `cover` and
one `metadata` pane.

```toml
[layout]
detail = "H(2:1, cover, metadata)"    # default: cover left, tags right
# detail = "V(2:1, cover, metadata)"  # stacked instead
```
