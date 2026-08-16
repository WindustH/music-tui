# music-tui

`music-tui` is a terminal music player client for [MPD](https://www.musicpd.org/)
built with Ratatui. It drives an existing MPD daemon and adds a tabbed,
mouse-friendly interface: queue, cover art, synced lyrics, metadata editing,
and a spectrum visualizer.

## Features

- MPD playback control: play/pause, seek, next/previous, volume, repeat/random/single/consume.
- Tabbed interface with a configurable pane layout DSL (`H(2:1, queue, V(2:1, cover, metadata))`).
- Queue view with filtering, selection, keyboard and mouse navigation.
- Secondary detail view for any queue entry: large cover plus full metadata.
- Cover art rendering with Kitty, Sixel, and iTerm2 graphics protocols, Chafa
  symbols, and ASCII fallback.
- Synced lyrics (`.lrc`, including word-level timestamps) with karaoke
  highlighting, click-to-seek, and auto-follow; plain/embedded lyrics supported.
- Metadata viewer and editor (`e` opens a TOML draft in `$EDITOR`).
- Spectrum visualizer fed by the MPD fifo output.
- Which-key style hints, scrollable `f1` key-binding help, command prompt (`:`).
- `music-tui open` subcommand for file-manager integration with four play modes.

## Usage

```sh
music-tui              # connect to MPD (127.0.0.1:6600 by default)
music-tui open ~/Music/album   # replace the queue with a folder
music-tui open song.flac      # see the open modes below
```

`open` modes (`-m`/`--mode`, default `interrupt`):

- `append` — append the file/folder to the queue.
- `next` — insert the file right after the currently playing song.
- `interrupt` — play the file immediately; when it ends, restore the previous
  queue and playback state.
- `folder` — play the file immediately and rebuild the queue from its folder.

Options: `-r`/`--recursive` recurses into subfolders; `--no-play` queues
without starting playback.

## Installation

### Arch Linux (AUR)

```bash
yay -S music-tui        # release build (from the v tag)
yay -S music-tui-git    # latest master
yay -S music-tui-bin    # prebuilt binary from the GitHub release
```

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

Requires `mpd`, `chafa` and `sqlite`.

## Documentation

[doc/index.md](doc/index.md).
