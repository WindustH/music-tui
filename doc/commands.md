# Commands

Press `:` to open the command prompt. Tab completion completes command names;
`up`/`down` walk history; `ctrl-g` edits the line in `$EDITOR`.

| Command | Effect |
| --- | --- |
| `:quit`, `:q` | exit music-tui |
| `:help` | open the key-binding help |
| `:play` | toggle play/pause |
| `:pause` | pause |
| `:toggle` | toggle play/pause |
| `:update` | rescan the music database |
| `:stop` | stop playback |
| `:update` | rescan the music database |
| `:next`, `:prev` | next / previous song |
| `:volume <n>` | set volume 0–100 |
| `:volume +n` / `:volume -n` | nudge volume |
| `:volume` | show current volume |
| `:vol` | alias of `volume` |
| `:repeat`, `:random`, `:single`, `:consume` | toggle the mode |
| `:clear` | clear the queue |
| `:dedup` | toggle hiding duplicate queue entries |
| `:tab` | list tabs |
| `:tab <n|name>` | switch tab by 1-based number or name |
| `:save` | export the queue to `<state>/playlists/music-tui-<time>.m3u` |
| `:save <name>` | export to `<state>/playlists/<name>.m3u` (`.m3u` added if missing) |
| `:save /abs/path.m3u` | export to an absolute path (relative paths are rejected) |
| `:add <path>` | append a file or folder (relative to `music_dir` if not absolute) |
| `:add <dir> -r` | append a folder recursively |

Examples:

```text
:volume 40
:volume +10
:tab playing
:add albums/game-ost --recursive
```
