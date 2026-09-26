# music-tui

`music-tui` is a terminal client for [MPD](https://www.musicpd.org/) built
with Ratatui. It drives an existing MPD daemon through a tabbed,
mouse-friendly interface: queue, local library, cover art, synced lyrics,
tag editing, and a spectrum visualizer.

https://github.com/user-attachments/assets/ed318f7b-a40b-41c5-a3dc-1a8dc9ae14b7

## Features

- Playback control: play/pause, seek, next/previous, volume, and the
  repeat / random / single / consume modes.
- Tabs built from a small layout language (`H(2:1, queue, V(2:1, cover, metadata))`).
- Queue view with live filtering, duplicate hiding, and keyboard and mouse
  navigation.
- Library view: music-tui indexes your music folders itself and filters
  across every tag, file name, and lyrics text.
- Detail view for any queue or library entry: large cover plus all tags.
- Cover art through the Kitty, Sixel, and iTerm2 image protocols, with
  Chafa character art and ASCII as fallbacks.
- Synced lyrics (`.lrc`, including word-level timestamps) with karaoke
  highlighting, click-to-seek, and auto-follow; plain and embedded lyrics
  work too.
- Tag editor: `e` opens the tags as a TOML draft in `$EDITOR`.
- Spectrum visualizer fed by an MPD fifo output (Linux/macOS).
- Which-key hints, a scrollable `f1` key help, and a `:` command prompt.
- `music-tui open` for file-manager integration, including a preview mode
  that restores your queue afterwards.

## Usage

```sh
music-tui                       # start the interface
music-tui open ~/Music/album    # replace the queue with a folder and play it
music-tui open song.flac        # preview a song, then restore the queue
```

On its first launch music-tui writes commented default configs
(`~/.config/music-tui/` on Linux and macOS). If MPD has no configuration
yet either, it also creates a minimal MPD config that listens on a local
Unix socket and points music-tui at it (Linux and macOS). Existing MPD
configs are never touched, and MPD itself still has to be started by your
service manager.

`open` modes (`-m`/`--mode`, default `interrupt`):

- `append` — add the file (or the folder's songs) to the end of the queue.
- `next` — insert the file right after the playing song.
- `interrupt` — play the file now; when it ends, restore the previous queue
  and playback position.
- `folder` — replace the queue with the file's folder and play the file.

`-r`/`--recursive` includes subfolders; `--no-play` queues without starting
playback. See [doc/open.md](doc/open.md) for playlists and details.

## Installation

### Arch Linux (AUR)

```bash
yay -S music-tui-stable   # release build (from the v tag)
yay -S music-tui-git       # latest master
yay -S music-tui-bin       # prebuilt binary from the GitHub release
```

Note: the plain `music-tui` AUR name belongs to a different project;
this one ships as `music-tui-stable` / `music-tui-git` / `music-tui-bin`
(all `provide: music-tui` and conflict with each other).

### Homebrew

```bash
brew tap WindustH/tap https://github.com/WindustH/homebrew-tap
brew install music-tui
```

A `--HEAD` build (from master) is available until the first bottled
release lands.

### From source

```bash
git clone --recurse-submodules https://github.com/WindustH/music-tui
cd music-tui
cargo install --path .
```

Building needs a Rust toolchain and, on Linux/macOS, the SQLite library
(Windows builds bundle it). At runtime you need MPD; `chafa` is optional and
only used for character-art covers.

### Windows

See [doc/windows.md](doc/windows.md) for MPD setup and Windows-specific
notes. In short: install MPD, create a `mpd.conf` with
`bind_to_address "127.0.0.1"`, register it as a Windows service
(`sc.exe create mpd ...`), then run `music-tui`.

### macOS

Apple's built-in Terminal.app implements no image protocol (Kitty, Sixel,
or iTerm2), so cover art falls back to character art (symbols/ASCII).
To see real images, use a protocol-capable terminal such as iTerm2,
WezTerm, or kitty (protocols are auto-detected).

## Documentation

Start at [doc/index.md](doc/index.md).
