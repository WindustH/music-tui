# Keymap

Keymaps are stored in `~/.config/music-tui/keymap.toml`, split by context:

- `[queue]`, `[cover]`, `[lyrics]`, `[metadata]`, `[visualizer]` — pane
  bindings; the active tab's `main` pane decides which section applies
- `[input]` — command prompt / filter input
- `[help]` — key-help dialog scrolling
- `[global]` — always active with priority in every non-input view

Entries use compact TOML:

```toml
[queue]
keymap = [
  { on = "j", run = "queue_down", desc = "Move selection down" },
  { on = "/", run = "queue_filter", desc = "Filter the queue (esc clears)" },
]

[global]
keymap = [
  { on = ["g", "g"], run = "queue_top", desc = "Move selection to top" },
]
```

`on` accepts a single key or a sequence. Supported key names include
characters (`q`, `J`, `\\`), `enter`, `space`, `esc`, `tab`, `backtab`,
`left`/`right`/`up`/`down`, `home`, `end`, `pgup`, `pgdn`, `f1`–`f12`,
`ctrl-x`, `alt-x`, and Yazi-style `<Enter>`, `<C-c>`.

Conflicts resolve as follows: `global` wins over pane sections; multi-key
sequences shadow shorter prefixes of themselves; the `input` context only
consults `[input]`.

## Actions

Queue: `queue_down`, `queue_up`, `queue_page_down`, `queue_page_up`,
`queue_top`, `queue_end`, `queue_goto_playing`, `queue_play`, `queue_delete`,
`queue_clear`, `queue_shuffle`, `queue_detail`, `queue_filter`.

Lyrics: `lyrics_up`, `lyrics_down`, `lyrics_page_up`, `lyrics_page_down`,
`lyrics_jump`, `lyrics_follow`.

Metadata: `scroll_up`, `scroll_down`, `page_up`, `page_down`,
`edit_metadata`, `back`.

Global: `quit`, `help`, `command`, `tab_next`, `tab_previous`,
`play_pause`, `next`, `previous`, `stop`, `seek_back`, `seek_forward`,
`seek_back_long`, `seek_forward_long`, `volume_up`, `volume_down`,
`volume_mute`, `toggle_repeat`, `toggle_random`, `cycle_single`,
`toggle_consume`.

Input: `cancel`, `submit`, `backspace`, `delete`, `move_left`,
`move_right`, `move_start`, `move_end`, `kill_before_cursor`,
`kill_after_cursor`, `completion_next`, `completion_previous`,
`history_previous`, `history_next`, `edit_in_editor`.

Help dialog: `scroll_up`, `scroll_down`, `page_up`, `page_down` — any key
not bound here closes the dialog.
