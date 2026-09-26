# Windows

On Windows music-tui talks to MPD over TCP. There is no first-run MPD
setup: install and configure MPD yourself before starting music-tui.

## Install MPD

Download the Windows build from https://www.musicpd.org/download.html
or install it with [Chocolatey](https://community.chocolatey.org/packages/mpd):

```powershell
choco install mpd
```

## Create mpd.conf

A minimal config:

```
music_directory "C:/Users/<you>/Music"
bind_to_address "127.0.0.1"
port "6600"
```

Replace `<you>` with your user name and point `music_directory` at your
music. Without an `audio_output` block MPD picks an output itself; add one
(for example `type "wasapi"`) to choose. music-tui reads `music_directory`
from `%APPDATA%\mpd\mpd.conf`; if your config lives elsewhere, set
`mpd.music_dir` in music-tui's `config.toml` instead.

## Start MPD

### As a Windows service (recommended)

Register MPD so it starts at boot and runs in the background:

```powershell
sc.exe create mpd binPath= '"C:\path\to\mpd.exe" "C:\path\to\mpd.conf"' DisplayName= "Music Player Daemon" start= auto
net start mpd
```

Replace the paths with your `mpd.exe` and `mpd.conf` locations. Stop it
with `net stop mpd`, remove it with `sc.exe delete mpd`, or manage it from
`services.msc`.

### In a console

```powershell
mpd mpd.conf
```

This keeps the window busy; useful for testing.

### Check that MPD runs

```powershell
telnet 127.0.0.1 6600
```

You should see `OK MPD ...`.

## Launch music-tui

```powershell
music-tui
```

The default `config.toml` already uses the TCP host:

```toml
[mpd]
host = "127.0.0.1"
port = 6600
```

Files: configs, `library.db`, and `state.toml` live in
`%APPDATA%\music-tui`; the log and cover cache in `%LOCALAPPDATA%\music-tui`.
`~` in configured paths means `%USERPROFILE%`.

## Differences from Linux and macOS

| Feature | Linux / macOS | Windows |
| --- | --- | --- |
| MPD connection | Unix socket or TCP | TCP only |
| First-run MPD setup | automatic | manual (above) |
| Spectrum visualizer (fifo) | supported | not available |
| Files outside the music directory, over TCP | symlinked into `mpd.link_dir` | copied into `mpd.link_dir` |

## Troubleshooting

**"connection refused" (os error 10061)** — MPD is not running or not
listening on the configured host/port. Start it and check with
`telnet 127.0.0.1 6600`.

**Visualizer** — the default tabs on Windows have no visualizer tab; a
`visualizer` pane in a custom layout shows "spectrum visualizer is
unavailable on this platform". Everything else works normally.

**SQLite during the build** — Windows builds compile the bundled SQLite; no
system install is needed.
