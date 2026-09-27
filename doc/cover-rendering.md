# Cover Rendering

## Where covers come from

For each song, music-tui uses the first of:

1. a picture embedded in the file (extracted into the cover cache);
2. `cover`, `folder`, `front`, `albumart`, or `album` with a `png`, `jpg`,
   `jpeg`, `webp`, `bmp`, or `gif` extension next to the file;
3. an image named like the audio file (`song.jpg` for `song.flac`);
4. the alphabetically first image in the same folder.

Covers are drawn aspect-correct and centered via
[img-tui](https://github.com/WindustH/img-tui).

## Render modes

At startup music-tui probes the terminal and uses the first mode that
works:

1. **Kitty graphics protocol** — inside Zellij 0.45+ only when Zellij's
   protocol query confirms support (plain Kitty placements, since Zellij
   has no Unicode-placeholder support).
2. **Sixel** — inside Zellij only with `render.zellij_sixel = true`.
3. **iTerm2 inline images** — iTerm2, WezTerm, some mintty setups.
4. **Symbols** — colored character art drawn by the `chafa` binary
   (options from `render.chafa_args`).
5. **ASCII** — `chafa` with plain ASCII, for the most limited terminals.

With `render.auto_detect = false`, only the character-art modes are used.

The `MUSIC_TUI_RENDER_MODES` environment variable overrides the order: a
comma-separated list of `kitty`, `sixel`, `iterm`, `symbols`, `ascii` (or
`auto`). Older versions read `GALLERY_TUI_RENDER_MODES` instead; that variable
no longer affects music-tui. Inside tmux or GNU screen, image escapes are
wrapped for the multiplexer automatically; `render.passthrough` (`tmux`,
`screen`, or `none`) overrides the detection.

## Cache

Extracted embedded covers are stored in the `covers/` folder of the cache
directory (`~/.cache/music-tui/covers/`), named by a hash of the picture;
it is safe to delete. Rendered covers are kept in memory for the last few
path/size combinations, so switching tabs or songs back and forth does not
re-render.
