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

`img-tui` stores extracted covers and compressed rendered output as BLOBs in
`cache.sqlite3` in the cache directory (normally `~/.cache/music-tui/`).
The old `covers/` directory is deleted on first use of the new backend; its
contents are not migrated or eagerly rebuilt. Original sibling images are
never deleted. File-only consumers use temporary copies removed on normal
cache-session shutdown.

The in-memory render cache uses moka's LRU policy with byte weights, controlled
by `render.raw_memory_cache_max_bytes` (64 MiB). Persistent payloads have a
`render.disk_cache_max_bytes` budget (512 MiB). Both accept `0` for unlimited.
Disk eviction is independent of memory eviction. Live payload bytes exclude
SQLite page/WAL overhead and temporary active images; a single oversized
result may exceed a budget. Cache keys include source identity, size, mtime,
render dimensions, mode and settings.

The music library (`library.db`), playback state and configuration are separate
from this disposable cache. To manually remove the SQLite cache, first close
music-tui, then delete `cache.sqlite3` and its `-wal`/`-shm` companions.
