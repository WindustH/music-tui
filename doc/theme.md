# Theme

Colors live in `theme.toml` in the config directory, one section per part of
the interface. A value is a color name (`cyan`, `bright black`,
`light_magenta`, `dark gray`, `default`), a 256-color index (`208`), or a
`#rrggbb` hex string. `default` (or `reset`) keeps the terminal's own
color. Keys you remove fall back to their defaults.

The defaults:

```toml
[base]
foreground = "default"
background = "default"
border = "bright black"       # pane borders, table headers
muted = "bright black"        # dimmed text, durations, hints
accent = "cyan"               # main pane title, prompt, detail border
accent_alt = "magenta"        # currently unused
render_background = false     # paint `background` behind the whole UI

[tab_bar]
active = "cyan"
inactive = "bright black"

[queue]
playing = "green"             # ▶ marker
paused = "yellow"             # ⏸ marker
selection = "cyan"            # selected row
highlight = "yellow"          # filter matches

[library]
playing = "green"             # ▶ marker
paused = "yellow"             # ⏸ marker
highlight = "yellow"          # filter matches
selection_foreground = "black"   # selected-row bar
selection_background = "cyan"
field_primary = "default"     # title / album / file name
field_secondary = "magenta"   # artist / genre / lyrics

[footer]
playing = "green"             # ▶ icon
paused = "yellow"             # ⏸ icon
stopped = "bright black"      # ■ icon and the offline notice
message = "magenta"           # transient messages

[progress]
bar = "cyan"                  # played part
background = "bright black"   # remaining part

[lyrics]
active = "cyan"               # active line / sung part
cursor = "cyan"               # ❯ manual cursor

[metadata]
label = "cyan"                # field names

[visualizer]
low = "green"
mid = "yellow"
high = "red"

[which_key]                   # hints for pending key sequences
background = "reset"          # reset = the popup background
foreground = "white"
key = "light_cyan"
description = "light_magenta"
separator = " -> "            # text between key and description
separator_color = "dark_gray"
columns = 3                   # most columns of hints (fewer when narrow)
```

`white` is the terminal's normal white and `bright white` its bright one;
the `bright …` names and ratatui's `light_…` names are interchangeable.

A `theme.toml` that no longer parses (for example an old flat-format file)
is backed up as `theme.toml.bak.<timestamp>` and replaced with the defaults.
