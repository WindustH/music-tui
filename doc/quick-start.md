# Quick Start

`music-tui` is a client for an [MPD](https://www.musicpd.org/) daemon.
Install MPD, make sure it runs, then start the client:

```sh
music-tui
```

## First run

music-tui writes commented default configs on its first launch. If no MPD
config exists yet (`~/.config/mpd/mpd.conf`, `~/.mpd/mpd.conf`, or
`~/.mpdconf`), it also generates a minimal one that listens on the Unix
socket `~/.config/mpd/socket` and points music-tui there. The generated file
is `~/.mpd/mpd.conf` on macOS and `$XDG_CONFIG_HOME/mpd/mpd.conf`
(`~/.config/mpd/mpd.conf`) elsewhere. It sets no `music_directory` and
keeps MPD's database, state, sticker, and log files in `~/.local/state/mpd`.
On macOS it also adds a CoreAudio output with a software mixer and a fixed
format (see [Troubleshooting](troubleshooting.md#macos-audio)). Existing MPD
configs and a host you already configured are never replaced. Start MPD
with your service manager; music-tui reconnects automatically. (Windows
has no automatic setup — see [Windows](windows.md).)

## Music directory

Songs queued as local `file://` URIs over a Unix socket need no music
directory: `open`, library playback, covers, lyrics, and tag editing resolve
their paths directly. Relative MPD song URIs — and local files over TCP —
need one. music-tui takes `mpd.music_dir` from `config.toml`, or else reads
`music_directory` from the MPD config locations above.

## Files

| | Linux / macOS | Windows |
| --- | --- | --- |
| `config.toml`, `keymap.toml`, `theme.toml` | `$XDG_CONFIG_HOME/music-tui` (`~/.config/music-tui`) | `%APPDATA%\music-tui` |
| `library.db`, `state.toml`, `playlists/` | `$XDG_STATE_HOME/music-tui` (`~/.local/state/music-tui`) | `%APPDATA%\music-tui` |
| `music-tui.log`, `covers/` | `$XDG_CACHE_HOME/music-tui` (`~/.cache/music-tui`) | `%LOCALAPPDATA%\music-tui` |

`state.toml` remembers the active tab, the queue selection, and the lyrics
follow mode between runs.

## Basic workflow

1. Switch tabs with `h`/`l`, the arrow keys, or `tab` (wraps around).
2. In the queue, move with `j`/`k` and press `enter` to play a song.
3. Press `i` for a song's detail view (cover + tags); `esc` returns.
4. `[`/`]` skip to the previous/next song, `\` toggles pause, `-`/`=` seek.
5. Click the progress band at the bottom to seek; drag to scrub.
6. `:` opens the command prompt, `f1` lists the keys of the current tab.

The visualizer needs an MPD fifo output — see [Visualizer](visualizer.md).
