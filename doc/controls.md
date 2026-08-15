# Controls

All keys are remappable — this documents the defaults. The `global` section
wins over pane bindings in every view.

## Global (every view)

| Key | Action |
| --- | --- |
| `:` | command prompt |
| `q` / `ctrl-c` | quit |
| `h`/`l`, `left`/`right`, `a`/`f`, `tab`/`backtab` | previous/next tab (cycles) |
| `[` / `]` | previous / next song |
| `\` | play/pause toggle |
| `x` | stop |
| `-` / `=` | seek 5s back / forward |
| `_` / `+` | seek 30s back / forward |
| `{` / `}` | volume down / up |
| `m` | mute toggle |
| `r` / `t` | repeat / random toggle |
| `y` | single mode (off → on → oneshot) |
| `C` | consume toggle |
| `f1` | key-binding help (scrollable) |

## Queue pane

| Key | Action |
| --- | --- |
| `j`/`k`, `up`/`down` | move selection |
| `pgup`/`pgdn` | page up / down |
| `home` / `G`/`end` | top / end |
| `g` or `c` | jump to the currently playing song |
| `enter` | play selected song |
| `d` | remove selected song |
| `D` | clear the queue |
| `i` | open detail view for the selected song |
| `/` | filter the queue (enter keeps, `esc` clears) |

## Lyrics pane

| Key | Action |
| --- | --- |
| `j`/`k`, `up`/`down` | scroll (leaves auto-follow) |
| `pgup`/`pgdn` | scroll by page |
| `F` | toggle auto-follow playback |
| `enter` | seek to the highlighted line and resume following |

## Metadata pane

| Key | Action |
| --- | --- |
| `j`/`k`, `up`/`down`, `pgup`/`pgdn` | scroll |
| `e` | edit tags in `$EDITOR` |
| `esc` | back (close filter/detail, then first tab) |
| `q` | quit — or close the detail view first when one is open |

## Mouse

- **Queue**: wheel scrolls the viewport (the selection passively follows and
  stays in view); click selects, clicking the selected row plays; middle-click
  selects and plays.
- **Lyrics**: wheel scrolls the viewport like the queue; clicking a synced
  line seeks there.
- **Tabs**: click a tab to switch.
- **Progress band**: click to seek, drag to scrub, wheel seeks ±5s.
- **Help dialog**: wheel scrolls, any click or key closes.
