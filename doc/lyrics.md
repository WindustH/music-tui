# Lyrics

## Lookup order

For each song, lyrics come from the first of:

1. `<file name>.lrc` next to the audio file
2. `<file name>.lrc` in each `lyrics.extra_dirs` folder
3. `<artist> - <title>.lrc` in each extra folder (from the song's tags)
4. the lyrics tag embedded in the file (ID3 `USLT`, Vorbis/FLAC `LYRICS`, …)

```toml
[lyrics]
extra_dirs = ["~/Music/lyrics"]
follow = true   # start in auto-follow mode
```

`.lrc` files must be UTF-8 (a byte-order mark is fine).

## Synced lyrics

Line and word timestamps are supported:

```text
[00:12.34]first line
[00:15.00]<00:15.00>word <00:15.40>timed <00:15.90>karaoke
[01:02.00][02:10.00]a line repeated at two times
```

- The active line is highlighted; lines with word timestamps light up word
  by word, other lines fill in evenly until the next line starts.
- Lines sharing one timestamp (an original and its translation) light up
  together.
- `[offset:+500]` shifts every line (positive = earlier, in milliseconds);
  `[mm:ss:xx]` stamps are accepted as well as `[mm:ss.xx]`.
- Auto-follow keeps the active line centered. `j`/`k` or the wheel switch
  to manual scrolling; `F` toggles follow, `enter` seeks to the selected
  line and resumes following, and clicking a line seeks there.
- Files without timestamps show as plain, scrollable text.

Very large files are cut at 512 KiB / 2000 lines.
