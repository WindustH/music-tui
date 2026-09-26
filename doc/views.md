# Views

## Tabs and panes

The interface is a row of tabs, each showing a layout of panes. The
defaults:

```toml
[[layout.tabs]]
name = "playlist"
layout = "H(2:1, queue, V(2:1, cover:hovered, metadata:hovered))"
main = "queue"

[[layout.tabs]]
name = "library"
layout = "H(2:1, library, V(2:1, cover:library-hovered, metadata:library-hovered))"
main = "library"

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
name = "visualizer"   # not on Windows
layout = "visualizer"
main = "visualizer"
```

See [Configuration](configuration.md#layout-language) for the layout
language.

Each tab has one **main pane**, drawn with a highlighted title. Keys go to
the main pane first; keys it does not use fall through to the other panes
of the tab, and `global` keys (playback, tab switching, `:`, quit) work
everywhere.

## Panes

- `queue` — MPD's queue. The playing song is marked `▶`/`⏸`; `/` filters
  as you type (title, artist, album, and file path; every word must match,
  spaces inside words are ignored). Duplicate entries are hidden unless
  `behavior.queue_dedup` is off.
- `library` — the music-tui library as a table with weighted columns
  (`[library] columns`). `/` filters every field, lyrics included, and
  highlights matches; `enter` plays, `a` appends, `i` opens the detail
  view, `u` rescans. See [Configuration](configuration.md#library).
- `cover` — cover art, aspect-correct and centered (see
  [Cover Rendering](cover-rendering.md)).
- `lyrics` — synced or plain lyrics with auto-follow and karaoke
  highlighting (see [Lyrics](lyrics.md)).
- `metadata` — tags and audio properties; `e` edits the tags (see
  [Metadata](metadata.md)).
- `visualizer` — spectrum bars from the MPD fifo output (see
  [Visualizer](visualizer.md)).

The queue and library panes have a scrollbar that can be clicked or dragged.

### Hovered data sources

`cover`, `lyrics` and `metadata` panes can show the selected row instead of
the playing song:

- `:hovered` — the song selected in the queue (alias `:queue-hovered`)
- `:library-hovered` — the track selected in the library

Hovered lyrics have no playback state: a plain scrollable list without
highlighting or seeking.

## Detail view

`i` on a queue or library entry opens a full-screen detail view: a large
cover beside the song's tags (arranged by `[layout].detail`). The other
tabs keep showing the playing song. `e` edits the shown song's tags;
`esc`, `i`, or `q` closes the view.

## Progress band

The bottom line is a seek bar with the elapsed and total time. Click to
seek, drag to scrub, or use the wheel to seek ±5 seconds.
