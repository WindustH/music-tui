# Theme

Colors live in `~/.config/music-tui/theme.toml`, grouped into one section
per interface part (the same way `keymap.toml` groups bindings per view).
Values are color names (`red`, `light_cyan`, `bright black`, `default`) or
`#rrggbb` hex strings. Any key you remove falls back to its default.

```toml
[base]                     # shared colors
foreground = "default"
background = "default"
border = "bright black"    # pane borders, table headers
muted = "bright black"     # dimmed text, durations, hints
accent = "cyan"            # main pane title, prompts, dialogs
accent_alt = "magenta"     # secondary accent

[tab_bar]                  # tab bar
active = "cyan"
inactive = "bright black"

[queue]                    # queue pane
playing = "green"          # ▶ marker
paused = "yellow"          # ⏸ marker
selection = "cyan"         # selected row
highlight = "yellow"       # filter keyword matches

[library]                  # library pane
playing = "green"          # ▶ marker
paused = "yellow"          # ⏸ marker
highlight = "yellow"       # filter keyword matches
selection_foreground = "black"   # selected-row bar
selection_background = "cyan"
field_primary = "default"        # title / album / filename text
field_secondary = "magenta"      # artist / genre / lyrics text

[footer]                   # status line
playing = "green"          # ▶ icon
paused = "yellow"          # ⏸ icon
stopped = "bright black"   # ■ icon, title while stopped
message = "magenta"        # transient messages

[progress]                 # bottom seek band
bar = "cyan"               # filled part
background = "bright black" # unfilled part

[lyrics]                   # lyrics pane
active = "cyan"            # active line / sung karaoke prefix
cursor = "cyan"            # ❯ manual navigation marker

[metadata]                 # metadata pane
label = "cyan"             # field label column

[visualizer]               # spectrum bands by frequency range
low = "green"
mid = "yellow"
high = "red"

[which_key]                # pending-sequence hint bar
background = "black"
foreground = "white"
key = "light_cyan"
description = "light_magenta"
separator = " -> "
separator_color = "dark_gray"
columns = 3
```

`which_key.columns` limits the hint bar to N columns (narrow terminals
shrink it automatically).

An incompatible `theme.toml` (for example a pre-section flat file) is
backed up next to the original (`.bak.<timestamp>`) and replaced with a
fresh commented default the next time the app starts.
