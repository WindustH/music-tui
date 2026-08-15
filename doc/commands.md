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
| `:stop` | stop playback |
| `:next`, `:prev` | next / previous song |
| `:volume <n>` | set volume 0–100 |
| `:volume +n` / `:volume -n` | nudge volume |
| `:volume` | show current volume |
| `:vol` | alias of `volume` |
| `:repeat`, `:random`, `:single`, `:consume` | toggle the mode |
| `:clear` | clear the queue |
| `:update` | rescan the music database |
| `:tab` | list tabs |
| `:tab <n|name>` | switch tab by 1-based number or name |
| `:add <path>` | append a file or folder (relative to `music_dir` if not absolute) |
| `:add <dir> -r` | append a folder recursively |

Examples:

```text
:volume 40
:volume +10
:tab playing
:add albums/game-ost --recursive
```
