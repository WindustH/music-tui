# Metadata

## View

The metadata pane and the detail view list:

- audio properties — duration, bitrate, sample rate, bit depth
- tags — Title, Artist, Album, AlbumArtist, Genre, Year, Track, Disk,
  Composer, Comment

## Editing

`e` opens the tags as a TOML draft in your editor (`$VISUAL`, then
`$EDITOR`, then `vi`, or `notepad` on Windows):

```toml
# Edit music tags. Save and exit to apply.
# Empty strings clear the field.
# file = "/path/to/song.flac"

[metadata]
Title = "Old Title"
Artist = "Old Artist"
Album = "Old Album"
AlbumArtist = ""
Genre = ""
Year = "2020"
Track = "3"
Disk = ""
Composer = ""
Comment = ""
```

Save and quit to apply; quit without saving (or leave the draft unchanged)
to cancel. music-tui compares the draft with the file and writes only the
changed tags. An empty value removes the tag; `Year`, `Track` and `Disk`
keep only the number before a `/` (`3/12` → `3`). Afterwards the views
refresh; for songs in MPD's music directory, MPD is also asked to re-read
the file so the queue shows the new tags without a manual `:update`.

`e` edits the song you are looking at: the detail view's song, the
selected queue row or library track, or otherwise the song shown by the
focused pane.

### Multiple tag blocks

Some files carry more than one tag block — typically WAV files with a
legacy RIFF INFO block (in an encoding such as GBK) next to an ID3v2 block.
MPD merges the blocks and shows undecodable bytes as `?`, so such songs
appear as `???` even though a clean value exists. music-tui:

- lists every block in the metadata pane (extra blocks are prefixed, e.g.
  `riff Title:`), and shows their values as comments in the draft;
- writes edits to **every** block, so whichever one MPD prefers is fixed —
  saving the draft unchanged still counts as an edit when another block
  disagrees with it.
