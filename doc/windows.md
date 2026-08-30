# Windows

`music-tui` runs on Windows via TCP. The MPD auto-bootstrap
(first-run socket generation) is Unix-only; on Windows you must install
and configure MPD yourself before launching music-tui.

## Install MPD

Download the Windows build from https://www.musicpd.org/download.html
or install via [Chocolatey](https://community.chocolatey.org/packages/mpd):

```powershell
choco install mpd
```

## Create mpd.conf

Create `mpd.conf` in a location MPD can find (e.g. the same directory as
`mpd.exe`, or `%APPDATA%\mpd\mpd.conf`). A minimal config:

```
music_directory "C:/Users/<you>/Music"
bind_to_address "127.0.0.1"
port "6600"
```

Replace `<you>` with your Windows username. Adjust `music_directory` to
point at your music library. The `null` audio output is a safe default;
replace it with a real output (e.g. `sox`, `wasapi`) when you are ready.

## Start MPD

### As a Windows service (recommended)

Register MPD so it starts automatically at boot and runs in the background:

```powershell
sc.exe create mpd binPath= '"C:\path\to\mpd.exe" "C:\path\to\mpd.conf"' DisplayName= "Music Player Daemon" start= auto
```

Replace the paths with your actual `mpd.exe` and `mpd.conf` locations.
Then start it:

```powershell
net start mpd
```

Other commands:

```powershell
net stop mpd          # stop
sc.exe delete mpd     # remove the service
```

You can also manage the service from `services.msc`.

### Manual (console)

```powershell
mpd mpd.conf
```

This keeps a terminal occupied; useful for testing or debugging.

### Verify MPD is running

```powershell
telnet 127.0.0.1 6600
```

You should see `OK MPD ...`.

## Launch music-tui

```powershell
music-tui
```

Make sure `config.toml` uses the TCP host (this is the default):

```toml
[mpd]
host = "127.0.0.1"
port = 6600
```

## Differences from Unix

| Feature                              | Unix               | Windows            |
| ------------------------------------ | ------------------ | ------------------ |
| MPD connection                       | Unix socket or TCP | TCP only           |
| First-run MPD bootstrap              | Automatic          | Manual (see above) |
| Spectrum visualizer (FIFO)           | Supported          | Not supported      |
| File bridge (`open` outside library) | Symlink            | File copy          |

## Troubleshooting

**"connection refused" (os error 10061)**
MPD is not running or not listening on the configured host/port. Start
MPD and verify with `telnet 127.0.0.1 6600`.

**"FIFO visualizer is not supported on Windows"**
The visualizer reads MPD's fifo audio output, which is a Unix-only
mechanism. Disable the visualizer tab or ignore the warning; all other
features work normally.

**SQLite not found during build**
The Windows build uses the `bundled` rusqlite feature, which compiles
SQLite from source automatically. No system SQLite install is needed.
