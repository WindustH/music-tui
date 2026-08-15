//! Headless `open` subcommand: queue a folder or file according to the
//! requested mode, then hand an optional interrupt session to the TUI.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use mpd_client::{
  commands::{Add, ClearQueue, Play, SetSingle, Status},
  commands::{SingleMode, SongPosition},
  responses::PlayState,
  Client,
};
use tracing::info;

use crate::{
  cli::OpenArgs,
  config::Settings,
  library::{collect_audio_files, is_audio_file, path_to_uri, resolve_music_dir},
  mpd::{InterruptSession, capture_interrupt_session, connect},
  playlist::{self, PlaylistKind},
};

pub struct OpenOutcome {
  pub notice: String,
  pub interrupt: Option<InterruptSession>,
}

pub async fn run_open(args: &OpenArgs, settings: &Settings) -> Result<OpenOutcome> {
  let path = args
    .path
    .canonicalize()
    .with_context(|| format!("failed to resolve {}", args.path.display()))?;
  let music_dir = resolve_music_dir(&settings.config.mpd)?;
  let (client, _events) = connect(&settings.config.mpd)
    .await
    .context("failed to connect to mpd")?;

  if path.is_dir() {
    let notice = if args.mode == crate::cli::OpenMode::Append {
      open_folder_append(&client, &path, &music_dir, args.recursive, args.no_play).await?
    } else {
      open_folder(&client, &path, &music_dir, args.recursive, args.no_play).await?
    };
    return Ok(OpenOutcome { notice, interrupt: None });
  }

  if !path.is_file() {
    bail!("{} is neither a file nor a directory", path.display());
  }

  if let Some(kind) = playlist::playlist_kind(&path) {
    return open_playlist(&client, &path, kind, args, &music_dir)
      .await
      .map(|notice| OpenOutcome { notice, interrupt: None });
  }

  let uri = path_to_uri(&music_dir, &path)?;
  let mut interrupt: Option<InterruptSession> = None;
  let notice = match () {
    _ if args.no_play => {
      client.command(Add::uri(&uri)).await?;
      format!("queued {} (not playing)", path.file_name().unwrap_or_default().to_string_lossy())
    }
    _ if args.mode == crate::cli::OpenMode::Append => {
      client.command(Add::uri(&uri)).await?;
      maybe_start_if_idle(&client).await?;
      format!("appended {}", short_name(&path))
    }
    _ if args.mode == crate::cli::OpenMode::Next => {
      let status = client.command(Status).await?;
      if let Some((position, _)) = status.current_song {
        client
          .command(Add::uri(&uri).at(position.0 + 1))
          .await?;
      } else {
        client.command(Add::uri(&uri)).await?;
        maybe_start_if_idle(&client).await?;
      }
      format!("queued {} next", short_name(&path))
    }
    _ if args.mode == crate::cli::OpenMode::Folder => {
      let folder = path.parent().map(Path::to_path_buf).unwrap_or_else(|| music_dir.clone());
      let files = collect_audio_files(&folder, args.recursive)?;
      let target = files.iter().position(|file| file == &path);
      let uris = uris_for(&music_dir, &files)?;
      client.command(ClearQueue).await?;
      for file_uri in &uris {
        client.command(Add::uri(file_uri)).await?;
      }
      let position = target.unwrap_or(0);
      client
        .command(Play::song(SongPosition(position)))
        .await?;
      format!("playing {} from folder queue ({} songs)", short_name(&path), uris.len())
    }
    _ => {
      // Interrupt: snapshot state, replace queue with the single song, arm restore.
      let session = capture_interrupt_session(&client).await?;
      client.command(ClearQueue).await?;
      client.command(Add::uri(&uri)).await?;
      client.command(SetSingle(SingleMode::Oneshot)).await?;
      client.command(Play::current()).await?;
      info!(playlist = ?session.playlist, "interrupt preview started");
      interrupt = Some(session);
      format!("previewing {} (queue will be restored afterwards)", short_name(&path))
    }
  };

  Ok(OpenOutcome { notice, interrupt })
}

async fn open_playlist(
  client: &Client,
  path: &Path,
  kind: PlaylistKind,
  args: &OpenArgs,
  music_dir: &Path,
) -> Result<String> {
  let _ = kind;
  let entries = playlist::parse_playlist(path).map_err(anyhow::Error::msg)?;
  let mut uris = Vec::new();
  for entry in &entries {
    let Ok(resolved) = entry.canonicalize() else { continue };
    if !is_audio_file(&resolved) {
      continue;
    }
    if let Ok(uri) = path_to_uri(music_dir, &resolved) {
      uris.push(uri);
    }
  }
  if uris.is_empty() {
    bail!("no playable entries in {}", short_name(path));
  }
  let skipped = entries.len() - uris.len();
  let name = short_name(path);
  let mut notice = String::new();
  // `interrupt` previews a single song; for whole playlists the natural
  // default is a plain replace (folder-style).
  let replace = matches!(
    args.mode,
    crate::cli::OpenMode::Folder | crate::cli::OpenMode::Interrupt
  );
  if replace {
    client.command(ClearQueue).await?;
    for uri in &uris {
      client.command(Add::uri(uri)).await?;
    }
    notice = format!("queued {} song(s) from {name}", uris.len());
    if !args.no_play {
      client.command(Play::song(SongPosition(0))).await?;
    }
  } else if args.mode == crate::cli::OpenMode::Next {
    let status = client.command(Status).await?;
    let start = status.current_song.map(|(position, _)| position.0 + 1).unwrap_or(0);
    for (offset, uri) in uris.iter().enumerate() {
      client.command(Add::uri(uri).at(start + offset)).await?;
    }
    notice = format!("queued {} song(s) from {name} next", uris.len());
    if !args.no_play {
      maybe_start_if_idle(client).await?;
    }
  } else {
    for uri in &uris {
      client.command(Add::uri(uri)).await?;
    }
    notice = format!("appended {} song(s) from {name}", uris.len());
    if !args.no_play {
      maybe_start_if_idle(client).await?;
    }
  }
  if skipped > 0 {
    notice.push_str(&format!(" ({skipped} entr{} skipped)", if skipped == 1 { "y" } else { "ies" }));
  }
  Ok(notice)
}

async fn open_folder(
  client: &Client,
  folder: &Path,
  music_dir: &Path,
  recursive: bool,
  no_play: bool,
) -> Result<String> {
  let files = collect_audio_files(folder, recursive)?;
  if files.is_empty() {
    bail!("no audio files found under {}", folder.display());
  }
  let uris = uris_for(music_dir, &files)?;
  client.command(ClearQueue).await?;
  for uri in &uris {
    client.command(Add::uri(uri)).await?;
  }
  let notice = format!("queued {} song(s) from {}", uris.len(), folder.display());
  if !no_play {
    client.command(Play::song(SongPosition(0))).await?;
  }
  Ok(notice)
}

/// Append every audio file under `folder` to the current queue without
/// clearing it.
async fn open_folder_append(
  client: &Client,
  folder: &Path,
  music_dir: &Path,
  recursive: bool,
  no_play: bool,
) -> Result<String> {
  let files = collect_audio_files(folder, recursive)?;
  if files.is_empty() {
    bail!("no audio files found under {}", folder.display());
  }
  let uris = uris_for(music_dir, &files)?;
  for uri in &uris {
    client.command(Add::uri(uri)).await?;
  }
  let notice = format!("appended {} song(s) from {}", uris.len(), folder.display());
  if !no_play {
    maybe_start_if_idle(client).await?;
  }
  Ok(notice)
}

fn uris_for(music_dir: &Path, files: &[PathBuf]) -> Result<Vec<String>> {
  files.iter().map(|file| path_to_uri(music_dir, file)).collect()
}

async fn maybe_start_if_idle(client: &Client) -> Result<()> {
  let status = client.command(Status).await?;
  if status.state == PlayState::Stopped {
    client.command(Play::current()).await?;
  }
  Ok(())
}

fn short_name(path: &Path) -> String {
  path
    .file_name()
    .map(|name| name.to_string_lossy().into_owned())
    .unwrap_or_else(|| path.display().to_string())
}
