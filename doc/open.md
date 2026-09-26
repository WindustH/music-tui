# Open Subcommand

`music-tui open` queues a file, folder, or playlist and then starts the
interface — handy as a file-manager "open with" action:

```sh
music-tui open <PATH> [-m MODE] [-r] [--no-play]
```

## Modes (`-m`/`--mode`)

For a single audio file:

- `interrupt` (default) — save the current queue as a stored playlist,
  replace the queue with the file, and play it once. When it ends, the
  previous queue, song, position, and pause state are restored and the
  stored playlist is deleted. If the restore fails, the playlist
  (`music-tui-preview-<unix time>`) is kept so you can load it by hand.
  The restore is done by the running music-tui, so keep it open until the
  preview ends.
- `append` — add the file to the end of the queue.
- `next` — insert the file right after the playing song (at the end when
  nothing plays).
- `folder` — replace the queue with the file's folder and play the file.

For a folder, `append` adds its audio files to the queue; every other mode
replaces the queue with them and plays from the first.

`-r`/`--recursive` includes subfolders (for folders and `folder` mode).
`--no-play` never starts playback; with `interrupt` it simply appends the
file. Without it, `append` and `next` start playback only when MPD is
stopped. With `behavior.queue_dedup` on, songs already in the queue are not
added again.

## Playlists and path lists

`open` also takes playlists (`.m3u`, `.m3u8`, `.pls`) and plain-text lists
of paths (`.txt`, one per line, `#` comments ignored). Entries resolve
relative to the list's folder and may use `~`; entries that are missing or
not audio files are skipped (the notice counts them).

- `append` adds the entries, `next` inserts them after the playing song;
- `folder` and `interrupt` replace the queue and play from the first entry.

Use `:save` inside music-tui to write such an m3u file
(see [Commands](commands.md)).

## How files reach MPD

- Files inside the music directory (`mpd.music_dir`, or `music_directory`
  from mpd.conf) are queued by their MPD path.
- Over a Unix socket connection, any other local file is queued as
  `file://…` — no music directory needed.
- Over TCP, other files are linked into `mpd.link_dir` (default
  `<music_dir>/.music-tui-links`; copied on Windows) and MPD's database is
  updated for them, which requires a music directory.

## Examples

```sh
music-tui open ~/Music/albums/some-album           # replace the queue and play
music-tui open -m append -r ~/Music/vocaloid       # append a folder tree
music-tui open -m next ~/Music/song.flac           # play next
music-tui open -m folder --no-play ~/Music/a/b.flac  # queue b.flac's folder, don't play
```
