# Controls

These are the default keys; all of them can be changed in
[keymap.toml](keymap.md). `global` keys work in every pane, except where a
pane binds the same key itself (the library's `a`). Keys the focused (main)
pane does not use fall through to the other panes of the tab. `f1` shows
the keys of the current tab.

## Global

| Key | Action |
| --- | --- |
| `:` | command prompt |
| `q`, `ctrl-c` | quit — or close the detail view first when one is open |
| `h`/`l`, `left`/`right`, `a`/`f`, `backtab`/`tab` | previous / next tab (wraps) |
| `[` / `]` | previous / next song |
| `\` | play / pause |
| `x` | stop |
| `-` / `=` | seek 5 s back / forward |
| `_` / `+` | seek 30 s back / forward |
| `{` / `}` | volume down / up (5%) |
| `m` | mute / restore the volume |
| `,` `r` | toggle repeat |
| `,` `t` | toggle random |
| `,` `y` | cycle single mode (off → on → oneshot) |
| `,` `c` | toggle consume |

## Every pane

| Key | Action |
| --- | --- |
| `f1` | key help for the current tab (scrollable) |
| `esc` | close the detail view, else clear the pane's filter, else go to the first tab |

## Queue

| Key | Action |
| --- | --- |
| `j`/`k`, `down`/`up` | move the selection |
| `pgdn`/`pgup` | page down / up |
| `g` `g`, `home` / `G`, `end` | first / last row |
| `g` `c` | jump to the playing song |
| `enter` | play the selected song |
| `d` | remove the selected song |
| `D` | clear the queue |
| `?` | shuffle the queue |
| `,` `d` | toggle hiding duplicate entries (on by default) |
| `i` | detail view of the selected song |
| `e` | edit the selected song's tags in `$EDITOR` |
| `/` | filter (`enter` keeps it, `esc` clears it) |

## Library

| Key | Action |
| --- | --- |
| `j`/`k`, `down`/`up` | move the selection |
| `pgdn`/`pgup` | page down / up |
| `g` `g`, `home` / `G`, `end` | first / last row |
| `enter` | play the selected track now (inserted after the playing song) |
| `a` | append the selected track to the queue |
| `i` | detail view of the selected track |
| `u` | rescan the library folders |
| `/` | filter every field, lyrics included (`enter` keeps, `esc` clears) |

In the default library tab, `e` (from the metadata pane) edits the selected
track's tags.

## Lyrics

| Key | Action |
| --- | --- |
| `j`/`k`, `down`/`up` | move the line cursor (leaves auto-follow) |
| `pgdn`/`pgup` | move by ten lines |
| `F` | toggle auto-follow |
| `enter` | seek to the selected line and resume following |

## Metadata

| Key | Action |
| --- | --- |
| `j`/`k`, `down`/`up` | scroll |
| `pgdn`/`pgup` | scroll by ten lines |
| `e` | edit the shown song's tags in `$EDITOR` |

## Help dialog

`j`/`k`, `down`/`up`, `pgdn`/`pgup` scroll; any other key or a click closes it.

## Mouse

- **Tabs**: click a tab to switch.
- **Queue / library**: the wheel scrolls the viewport (the selection follows
  to stay visible); click selects, clicking the selected row plays it;
  middle-click plays directly. Click or drag the scrollbar to jump.
- **Lyrics**: the wheel scrolls; clicking a synced line of the playing song
  seeks there.
- **Progress band**: click to seek, drag to scrub, wheel seeks ±5 s.
- **Help dialog**: the wheel scrolls, any click closes it.
