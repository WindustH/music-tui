# Lyrics

## Lookup order

For the currently playing song, lyrics are searched in this order:

1. `<song-basename>.lrc` next to the audio file
2. `<song-basename>.lrc` in each configured `lyrics.extra_dirs`
3. `<artist> - <title>.lrc` in each extra dir
4. embedded `LYRICS` tags in the file (ID3/Vorbis/FLAC)

## Synced lyrics

Line-level and word-level timestamps are supported:

```text
[00:12.34] first line
<00:12.34> word <00:12.60> timed <00:13.00> karaoke
```

- The active line is highlighted; with word timestamps the sung prefix is
  colored progressively (karaoke).
- Bilingual exports that repeat one timestamp for the original line and
  its translation light up as a group: the original follows its word
  timings, the translation interpolates over the pair's span.
- Auto-follow keeps the active line centered while playing.
- Scrolling (`j`/`k`, wheel) leaves follow mode; `F` toggles it back,
  `enter` seeks to the highlighted line and resumes following.
- Clicking a synced line seeks there.
- Lines without timestamps fall back to plain (scroll-only) lyrics.

## Extra dirs

```toml
[lyrics]
extra_dirs = ["~/Music/lyrics"]
follow = true
```
