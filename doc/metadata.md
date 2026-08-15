# Metadata

## View

The metadata pane (and the detail view) show two groups of entries:

- `file` — path, duration, bitrate, sample rate, bit depth
- `tag` — title, artist, album, album artist, genre, year, track, disk,
  composer, comment

## Editing

Press `e` in a metadata pane (or in the detail view) on the target song.
This opens a TOML draft in `$EDITOR`:

```toml
# music-tui metadata draft — save to apply, quit without saving to discard.
# File: /path/to/song.flac
# Leave a value empty to remove the tag.

[metadata]
title = "Old Title"
artist = "Old Artist"
album = "Old Album"
track = "3/12"
```

On save, music-tui diffs the draft against the original tags and writes only
the changed keys back to the file (title/artist/album/album-artist/genre/
year/track/disk/composer/comment). The queue is not disturbed; metadata
refreshes automatically after the write.

Editing targets the song shown in the pane — the current song in a metadata
pane, the detailed song in the detail view.
