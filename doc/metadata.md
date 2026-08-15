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

### Multiple tag blocks

Some files carry more than one tag block — most commonly WAV files with a
legacy RIFF INFO block (single-byte encodings like GBK) next to an ID3v2
block. MPD merges every block it can read and replaces undecodable bytes
with `?`, so such files show up with `???` labels even though a clean
value exists. music-tui:

- lists every tag block in the metadata pane (extra blocks appear with a
  `riff Title:`-style prefix) so duplicated or corrupted values are
  visible,
- writes edits to **every** tag block in the file, so whichever block MPD
  prefers carries the corrected values,
- asks MPD to update the file's database entry right after a successful
  write, so the queue picks up the fixed tags without a manual `:update`.
