# Keymap

Key bindings live in `keymap.toml` in the config directory, one section per
context:

- `[queue]`, `[library]`, `[cover]`, `[lyrics]`, `[metadata]`,
  `[visualizer]` — pane bindings
- `[global]` — active in every pane
- `[input]` — the `:` command prompt and `/` filter prompt
- `[help]` — scrolling in the `f1` key help

Each entry binds one key or a key sequence to an action:

```toml
[queue]
keymap = [
  { on = "j", run = "queue_down", desc = "Move selection down" },
  { on = ["g", "c"], run = "queue_goto_playing", desc = "Jump to the playing song" },
]
```

`desc` is what the key help and the which-key hints show.

## Key names

Characters (`q`, `G`, `?`; a backslash is `"\\"` in TOML), `space`,
`enter`, `esc`, `tab`, `backtab`, `backspace`, `delete`, `insert`,
`left`/`right`/`up`/`down`, `home`, `end`, `pgup`/`pgdn` (or
`pageup`/`pagedown`), `f1`–`f12`, and modifiers as `ctrl-x` / `alt-x`.
Vim-style names work too: `<Enter>`, `<Space>`, `<Esc>`, `<S-Tab>`,
`<C-c>`, `<A-x>`.

## How keys are resolved

- The active tab's main pane is asked first, then the tab's other panes in
  layout order: a key the main pane does not bind still works if another
  visible pane binds it.
- Within a pane, `[global]` wins over the pane's own section — unless the
  pane binds that exact key itself (the library's `a` appends, while `a`
  switches tabs everywhere else).
- A key that starts a longer sequence waits for the next key (which-key
  hints appear); a key that breaks the sequence starts over as a new key.
- Prompts only consult `[input]`, so typing never triggers playback keys.

At startup, any action missing from a section is added back with its
default keys. To move an action to another key, rebind it instead of
deleting its entry.

## Actions

**Global**: `quit`, `help`, `command`, `tab_next`, `tab_previous`, `back`,
`play_pause`, `next`, `previous`, `stop`, `seek_back`, `seek_forward`,
`seek_back_long`, `seek_forward_long`, `volume_up`, `volume_down`,
`volume_mute`, `toggle_repeat`, `toggle_random`, `cycle_single`,
`toggle_consume`, `toggle_follow_current` (keep the queue selection on the
playing song; unbound by default).

**Queue**: `queue_down`, `queue_up`, `queue_page_down`, `queue_page_up`,
`queue_top`, `queue_end`, `queue_goto_playing`, `queue_play`,
`queue_delete`, `queue_clear`, `queue_shuffle`, `queue_dedup`,
`queue_detail`, `queue_filter`, `edit_metadata`.

**Library**: `library_down`, `library_up`, `library_page_down`,
`library_page_up`, `library_top`, `library_end`, `library_play`,
`library_append`, `library_detail`, `library_rescan`, `library_filter`.

**Lyrics**: `lyrics_up`, `lyrics_down`, `lyrics_page_up`,
`lyrics_page_down`, `lyrics_jump`, `lyrics_follow`.

**Metadata**: `scroll_up`, `scroll_down`, `page_up`, `page_down`,
`edit_metadata`.

**Visualizer**: `visualizer_reset` (clear the bars; unbound by default).

Actions are not tied to a section: any pane may bind any of them.

**Input**: `cancel`, `submit`, `backspace`, `delete`, `move_left`,
`move_right`, `move_start`, `move_end`, `kill_before_cursor`,
`kill_after_cursor`, `completion_next`, `completion_previous`,
`history_previous`, `history_next`, `help`.

**Help dialog**: `scroll_up`, `scroll_down`, `page_up`, `page_down`; any
other key closes the dialog.
