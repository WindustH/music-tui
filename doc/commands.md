# Commands

Press `:` to open the command prompt. `tab`/`backtab` cycle completions
(command names, and tab names after `:tab`); `up`/`down` walk the history.

| Command | Effect |
| --- | --- |
| `:quit`, `:q` | exit music-tui |
| `:help` | open the key help |
| `:play`, `:toggle` | toggle play / pause |
| `:pause` | pause |
| `:stop` | stop playback |
| `:next`, `:prev` | next / previous song |
| `:volume <n>` | set the volume (0–100) |
| `:volume +<n>`, `:volume -<n>` | change the volume by `n` |
| `:volume` | show the current volume |
| `:vol …` | alias of `:volume` |
| `:repeat`, `:random`, `:consume` | toggle the mode |
| `:single` | toggle single mode on / off (the `,` `y` key also cycles through oneshot) |
| `:clear` | clear the queue |
| `:dedup` | toggle hiding duplicate queue entries |
| `:update` | make MPD rescan its whole music database |
| `:tab` | list the tabs |
| `:tab <n>`, `:tab <name>` | switch to a tab by 1-based number or name |
| `:add <path>` | append a file, or a folder's audio files |
| `:add <path> -r` | same, including subfolders (`--recursive` works too) |
| `:save` | export the queue to `playlists/music-tui-<unix time>.m3u` in the state directory |
| `:save <name>` | export to `<name>` in that directory (`.m3u` added when there is no extension) |
| `:save /abs/path.m3u` | export to an absolute path (`~` works; relative paths with folders are rejected) |

`:add` resolves relative paths against the music directory. Local files
outside it work over a Unix socket connection; over TCP they need to be
inside the music directory. The export directory can be changed with
`playlist.save_dir` (see [Configuration](configuration.md)).

Examples:

```text
:volume 40
:volume +10
:tab playing
:add albums/game-ost --recursive
:save favorites
```

Command prompts, completion lists, status messages, and which-key hints use overlay surfaces: showing or dismissing them does not resize the main content viewport. Permanent navigation (the music tab bar and Calibre search header) keeps its existing position.
Kitty Unicode images are occluded through text cells without changing image geometry. Pixel protocols may repaint intersecting images when an overlay changes, then paint the overlay text last. Overlay surfaces consume pointer events instead of activating covered content.
