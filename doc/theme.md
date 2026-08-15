# Theme

Colors live in `~/.config/music-tui/theme.toml`. Values are color names
(`red`, `light_cyan`, …) or `#rrggbb` hex strings.

```toml
foreground = "white"
background = "black"
border = "dark_gray"
muted = "gray"
accent = "cyan"            # main pane title, pointers, dialogs
accent_alt = "light_cyan"  # messages

playing = "light_green"    # ▶ marker, playing title
paused = "light_yellow"    # ⏸ marker
stopped = "dark_gray"

progress = "cyan"              # filled part of the seek band
progress_background = "gray"   # unfilled part

lyrics_active = "light_yellow" # active lyric line / sung karaoke prefix
library_highlight = "light_yellow" # filter keyword matches in queue/library rows

visualizer_low = "green"       # bars by magnitude
visualizer_mid = "yellow"
visualizer_high = "red"

which_key_background = "black"       # pending-sequence hint bar
which_key_foreground = "white"
which_key_key = "light_cyan"
which_key_description = "light_magenta"
which_key_separator = " -> "
which_key_separator_color = "dark_gray"
which_key_columns = 3
```

`which_key_columns` limits the hint bar to N columns (narrow terminals shrink
it automatically).
