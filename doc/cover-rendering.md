# Cover Rendering

Cover art is looked up per song (embedded pictures, then sibling files like
`cover.*`, `folder.*`, `front.*`, or a file named like the track) and rendered
aspect-correct and centered via [img-tui](https://github.com/WindustH/img-tui).

## Protocol selection

On startup music-tui probes the terminal (and `$TERM_PROGRAM`, Zellij
environment) and picks the first working mode:

1. **Kitty graphics protocol** — transmitted as escape sequences; placements
   are erased/redrawn on layout changes.
2. **Sixel** — including inside Zellij with `render.zellij_sixel = true`.
3. **iTerm2 inline images** — also used by some WezTerm/mintty setups.
4. **Chafa symbols** — half-block symbol art via the `chafa` binary; colors
   and symbols follow `render.chafa_args`.
5. **ASCII** — Chafa with `--colors=none --symbols=ascii`, for the most
   constrained terminals.

Set `MUSIC_TUI_RENDER_MODES` (comma-separated: `kitty,sixel,iterm,symbols,ascii`)
to override the detection order.

## Cache

Extracted covers are cached under `~/.cache/music-tui/covers/` keyed by a
hash of the picture bytes; renders are kept in a small in-memory LRU keyed by
path + size + mode, so pane resizes re-render only what changed.
