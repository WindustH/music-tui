# Open Subcommand

`music-tui open` is designed for file-manager integration:

```sh
music-tui open <PATH> [OPTIONS]
```

The path must be inside MPD's music directory (auto-detected or configured) —
except playlist files, which may live anywhere. Folders take every audio
file inside (sorted); `-r`/`--recursive` recurses.

## Playlist and path-list files

`open` also accepts playlists (`.m3u`/`.m3u8`/`.pls`) and plain-text lists
of song paths (`.txt`, one path per line, `#` comments ignored). Entries
resolve relative to the list's own folder (or `~`), must stay inside the
music directory, and non-audio lines are skipped (the notice reports the
count).

- `append` adds all entries to the queue;
- `next` inserts them right after the current song;
- `folder`/`interrupt` (the default for lists) replaces the queue and
  starts playback at the first entry; `--no-play` just queues.

Use `:save` inside music-tui to produce such m3u files (see
[Commands](commands.md)).

## Modes (`-m`/`--mode`)

- `append` — add to the end of the queue, keeping what is playing.
- `next` — insert the file right after the currently playing song (or at the
  end when nothing plays).
- `interrupt` (default) — remember the current queue and playback state
  (saved as a stored playlist), replace the queue with the file, enable
  single-shot mode, and play. When the song ends, the previous queue,
  position, seek offset, and pause state are restored.
- `folder` — replace the queue with the song's folder (recursive optional)
  and start playback at the chosen file.

`--no-play` queues without starting playback (append and folder modes).

## Examples

```sh
music-tui open ~/Music/albums/some-album           # interrupt an album in
music-tui open -m append -r ~/Music/vocaloid       # recursive append
music-tui open -m next ~/Music/song.flac           # play next
music-tui open -m folder --no-play ~/Music/a/b.flac
```
