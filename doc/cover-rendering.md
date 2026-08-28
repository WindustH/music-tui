# Cover Rendering

Cover art is looked up per song (embedded pictures first, then sibling
files like `cover.*`, `folder.*`, `front.*`, a file named like the track,
and finally any image in the same folder) and rendered
aspect-correct and centered via [img-tui](https://github.com/WindustH/img-tui).

## Protocol selection

On startup music-tui probes the terminal and picks the first working mode:

1. **Kitty graphics protocol** — transmitted as escape sequences; placements
   are erased/redrawn on layout changes. Under Zellij 0.45+, KGP is selected
   only when Zellij's protocol query confirms that it and the host terminal
   support it. Regular Kitty placements are used because Zellij does not
   currently support Kitty Unicode placeholders.
2. **Sixel** — including inside Zellij with `render.zellij_sixel = true`.
3. **iTerm2 inline images** — also used by some WezTerm/mintty setups.
4. **Chafa symbols** — half-block symbol art via the `chafa` binary; colors
   and symbols follow `render.chafa_args`.
5. **ASCII** — Chafa with `--colors=none --symbols=ascii`, for the most
   constrained terminals.

Set `MUSIC_TUI_RENDER_MODES` (comma-separated: `kitty,sixel,iterm,symbols,ascii`)
to override the detection order.

`render.zellij_sixel` controls only Sixel. Kitty remains automatically enabled
when Zellij confirms KGP support; older or KGP-disabled Zellij sessions fall
back to the remaining modes.

## Cache

Extracted covers are cached under `~/.cache/music-tui/covers/` keyed by a
hash of the picture bytes; renders are kept in a small in-memory LRU keyed by
path + size + mode, so pane resizes re-render only what changed.
