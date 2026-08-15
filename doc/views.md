# Views

## Tabs and panes

The interface is a row of tabs, each rendering a configurable pane layout.
The default layout configuration:

```toml
[[layout.tabs]]
name = "playlist"
layout = "H(2:1, queue, V(2:1, cover, metadata))"
main = "queue"

[[layout.tabs]]
name = "playing"
layout = "H(1:2, cover, lyrics)"
main = "cover"

[[layout.tabs]]
name = "metadata"
layout = "metadata"
main = "metadata"

[[layout.tabs]]
name = "lyrics"
layout = "lyrics"
main = "lyrics"

[[layout.tabs]]
name = "visualizer"
layout = "visualizer"
main = "visualizer"
```

See [Configuration](configuration.md) for the full DSL (`H`/`V` splits, pane
names, `main`).

Each tab declares one **main pane**. The main pane's title is highlighted and
its key bindings are the ones active while the tab is shown — pressing keys
always targets the main pane of the current tab. Keys bound in the `global`
keymap section (playback control, tab switching, `:` command, quit) take
priority over pane bindings everywhere.

## Panes

- `queue` — the MPD current playlist. The playing song is marked with `▶`/`⏸`,
  filtered mode narrows the list as you type (`/`).
- `cover` — cover art for the currently playing song, aspect-correct and
  centered (see [Cover Rendering](cover-rendering.md)).
- `lyrics` — synced or plain lyrics for the current song with auto-follow and
  karaoke highlighting (see [Lyrics](lyrics.md)).
- `metadata` — tag and file properties of the current song, `e` edits.
- `visualizer` — spectrum bars fed from the MPD fifo output.

### Hovered data sources

`cover`, `lyrics` and `metadata` panes accept a `:hovered` suffix (e.g.
`layout = "H(2:1, queue, cover:hovered)"`) to display the queue's hovered
row instead of the playing song. Hovered lyrics have no playback state —
they render as a plain scrollable list (j/k scroll, no sync highlight, no
seek). See [Configuration](configuration.md#pane-data-sources).

## Detail view

Pressing `i` on a queue entry opens a secondary detail view (like opening an
image in a gallery browser): a large cover beside the metadata of that
entry (layout configurable via `[layout].detail`, cover left by default).
The sidebar panes always keep showing the *currently playing* song. `e`
edits the detailed song's tags, `esc`/`i`/`q` returns to the queue — with a
secondary view open, `q` leaves that level instead of quitting the app.

## Progress band

The bottom of the screen is a full-width seek bar showing elapsed/total time.
Click anywhere on it to seek; click-drag to scrub; the mouse wheel on it seeks
±5 seconds.
