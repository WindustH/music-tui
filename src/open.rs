//! Headless `open` subcommand: queue a folder or file according to the
//! requested mode, then hand an optional interrupt session to the TUI.

use std::collections::HashSet;
use std::fmt::Display;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use mpd_client::{
  Client,
  client::CommandError,
  commands::{Add, Delete, DeletePlaylist, Play, Queue, SetSingle, Status, Update},
  commands::{SingleMode, SongPosition},
  responses::PlayState,
};
use tracing::{info, warn};

use crate::{
  cli::{OpenArgs, OpenMode},
  config::{MpdConfig, Settings},
  library::{
    collect_audio_files, ensure_link, file_uri, is_audio_file, is_socket_host, links_dir,
    path_to_uri, resolve_music_dir, song_key,
  },
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
  let music_dir = resolve_music_dir(&settings.config.mpd).ok();
  let client = connect(&settings.config.mpd)
    .await
    .context("failed to connect to mpd")?;
  let (client, _events) = client;

  if path.is_dir() {
    let notice = if args.mode == OpenMode::Append {
      open_folder_append(
        &client,
        &path,
        &settings.config.mpd,
        music_dir.as_deref(),
        args.recursive,
        args.no_play,
        settings.config.behavior.queue_dedup,
      )
      .await?
    } else {
      open_folder(
        &client,
        &path,
        &settings.config.mpd,
        music_dir.as_deref(),
        args.recursive,
        args.no_play,
      )
      .await?
    };
    return Ok(OpenOutcome {
      notice,
      interrupt: None,
    });
  }

  if !path.is_file() {
    bail!("{} is neither a file nor a directory", path.display());
  }

  if let Some(kind) = playlist::playlist_kind(&path) {
    return open_playlist(
      &client,
      &path,
      kind,
      args,
      &settings.config.mpd,
      music_dir.as_deref(),
      settings.config.behavior.queue_dedup,
    )
    .await
    .map(|notice| OpenOutcome {
      notice,
      interrupt: None,
    });
  }

  let uri = resolve_open_uri(&client, &path, &settings.config.mpd, music_dir.as_deref()).await?;
  let dedup = settings.config.behavior.queue_dedup;
  let name = short_name(&path);
  let mut interrupt: Option<InterruptSession> = None;
  let notice = match args.mode {
    OpenMode::Interrupt if !args.no_play => {
      // Snapshot state, replace the queue with the single song, arm restore.
      let session = capture_interrupt_session(&client).await?;
      if let Err(error) = replace_queue(&client, std::slice::from_ref(&uri), &name).await {
        // The queue is untouched: drop the snapshot instead of leaving a
        // stray preview playlist behind.
        if let Some(playlist) = &session.playlist {
          let _ = client.command(DeletePlaylist(playlist.as_str())).await;
        }
        return Err(error);
      }
      client.command(SetSingle(SingleMode::Oneshot)).await?;
      client.command(Play::current()).await?;
      info!(playlist = ?session.playlist, "interrupt preview started");
      interrupt = Some(session);
      format!("previewing {name} (queue will be restored afterwards)")
    }
    OpenMode::Folder => {
      let folder = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
      let files = collect_audio_files(&folder, args.recursive)?;
      let target = files.iter().position(|file| file == &path);
      let uris =
        resolve_open_uris(&client, &files, &settings.config.mpd, music_dir.as_deref()).await?;
      if uris.is_empty() {
        bail!("no playable audio files under {}", folder.display());
      }
      let queued = replace_queue(
        &client,
        &uris,
        format_args!("the files in {}", folder.display()),
      )
      .await?;
      let skipped = queued.skipped_clause();
      if args.no_play {
        format!(
          "queued the folder of {name} ({} songs, not playing{skipped})",
          queued.added()
        )
      } else {
        // The chosen file may be one MPD refused; then play from the top.
        let position = target
          .and_then(|index| queued.positions[index])
          .unwrap_or(0);
        client.command(Play::song(SongPosition(position))).await?;
        format!(
          "playing {name} from folder queue ({} songs{skipped})",
          queued.added()
        )
      }
    }
    OpenMode::Next => {
      if dedup && queue_has(&client, &uri).await? {
        format!("{name} already queued")
      } else {
        let status = client.command(Status).await?;
        if let Some((position, _)) = status.current_song {
          add_one(&client, Add::uri(&uri).at(position.0 + 1), &name).await?;
        } else {
          add_one(&client, Add::uri(&uri), &name).await?;
          if !args.no_play {
            maybe_start_if_idle(&client).await?;
          }
        }
        format!("queued {name} next")
      }
    }
    // Append — and an interrupt preview that must not play, which is just
    // a queued song.
    OpenMode::Append | OpenMode::Interrupt => {
      let queued = dedup && queue_has(&client, &uri).await?;
      if !queued {
        add_one(&client, Add::uri(&uri), &name).await?;
      }
      if !args.no_play {
        maybe_start_if_idle(&client).await?;
      }
      match (queued, args.no_play) {
        (true, false) => format!("{name} already queued"),
        (true, true) => format!("{name} already queued (not playing)"),
        (false, false) => format!("appended {name}"),
        (false, true) => format!("queued {name} (not playing)"),
      }
    }
  };

  Ok(OpenOutcome { notice, interrupt })
}

async fn open_playlist(
  client: &Client,
  path: &Path,
  kind: PlaylistKind,
  args: &OpenArgs,
  mpd_config: &MpdConfig,
  music_dir: Option<&Path>,
  dedup: bool,
) -> Result<String> {
  let _ = kind;
  let entries = playlist::parse_playlist(path).map_err(anyhow::Error::msg)?;
  let mut files = Vec::new();
  for entry in &entries {
    let Ok(resolved) = entry.canonicalize() else {
      continue;
    };
    if !is_audio_file(&resolved) {
      continue;
    }
    files.push(resolved);
  }
  if files.is_empty() {
    bail!("no playable entries in {}", short_name(path));
  }
  let mut uris = resolve_open_uris(client, &files, mpd_config, music_dir).await?;
  let name = short_name(path);
  // `interrupt` previews a single song; for whole playlists the natural
  // default is a plain replace (folder-style).
  let replace = matches!(args.mode, OpenMode::Folder | OpenMode::Interrupt);
  if dedup {
    skip_queued_and_batch_dups(client, &mut uris, !replace).await?;
  }
  if uris.is_empty() {
    return Ok(format!("all entries from {name} already queued"));
  }
  let what = format_args!("the entries of {name}");
  let (queued, notice) = if replace {
    let queued = replace_queue(client, &uris, what).await?;
    if !args.no_play {
      client.command(Play::song(SongPosition(0))).await?;
    }
    let notice = format!("queued {} song(s) from {name}", queued.added());
    (queued, notice)
  } else if args.mode == OpenMode::Next {
    let status = client.command(Status).await?;
    let start = status
      .current_song
      .map(|(position, _)| position.0 + 1)
      .unwrap_or(0);
    let queued = add_playable(client, &uris, Some(start))
      .await?
      .require_any(what)?;
    if !args.no_play {
      maybe_start_if_idle(client).await?;
    }
    let notice = format!("queued {} song(s) from {name} next", queued.added());
    (queued, notice)
  } else {
    let queued = add_playable(client, &uris, None).await?.require_any(what)?;
    if !args.no_play {
      maybe_start_if_idle(client).await?;
    }
    let notice = format!("appended {} song(s) from {name}", queued.added());
    (queued, notice)
  };
  // Missing, non-audio, already queued, or refused by MPD.
  let skipped = entries.len() - queued.added();
  let mut notice = notice;
  if skipped > 0 {
    notice.push_str(&format!(
      " ({skipped} entr{} skipped)",
      if skipped == 1 { "y" } else { "ies" }
    ));
  }
  Ok(notice)
}

/// Resolve one file to a playable MPD uri: in-library paths keep their
/// relative uri; outside paths become `file://` on socket connections or
/// a bridged symlink (plus a db update) on TCP connections.
pub(crate) async fn resolve_open_uri(
  client: &Client,
  path: &Path,
  mpd_config: &MpdConfig,
  music_dir: Option<&Path>,
) -> Result<String> {
  if let Some(uri) = direct_open_uri(path, mpd_config, music_dir) {
    return Ok(uri);
  }
  let owned = path.to_path_buf();
  resolve_outside_uris(client, &[owned], mpd_config, music_dir)
    .await
    .map(|mut uris| uris.remove(0))
}

/// Resolve a path without touching MPD. Relative library URIs are preferred
/// when a root is known; otherwise Unix socket connections can use `file://`.
pub(crate) fn direct_open_uri(
  path: &Path,
  mpd_config: &MpdConfig,
  music_dir: Option<&Path>,
) -> Option<String> {
  if let Some(music_dir) = music_dir
    && let Ok(uri) = path_to_uri(music_dir, path)
  {
    return Some(uri);
  }
  is_socket_host(&mpd_config.host).then(|| file_uri(path))
}

/// Resolve a batch of files (mixed in/outside paths allowed), preserving
/// the caller's order.
async fn resolve_open_uris(
  client: &Client,
  files: &[PathBuf],
  mpd_config: &MpdConfig,
  music_dir: Option<&Path>,
) -> Result<Vec<String>> {
  let inside: Vec<(usize, String)> = files
    .iter()
    .enumerate()
    .filter_map(|(index, file)| {
      direct_open_uri(file, mpd_config, music_dir).map(|uri| (index, uri))
    })
    .collect();
  let outside: Vec<(usize, PathBuf)> = files
    .iter()
    .enumerate()
    .filter(|(_, file)| direct_open_uri(file, mpd_config, music_dir).is_none())
    .map(|(index, file)| (index, file.clone()))
    .collect();
  if outside.is_empty() {
    return Ok(inside.into_iter().map(|(_, uri)| uri).collect());
  }
  let outside_paths: Vec<PathBuf> = outside.iter().map(|(_, path)| path.clone()).collect();
  let resolved = resolve_outside_uris(client, &outside_paths, mpd_config, music_dir).await?;
  let mut mixed: Vec<(usize, String)> = inside;
  mixed.extend(outside.iter().map(|(index, _)| *index).zip(resolved));
  mixed.sort_by_key(|(index, _)| *index);
  Ok(mixed.into_iter().map(|(_, uri)| uri).collect())
}

/// Files outside the library: `file://` when connected via socket, else a
/// symlink bridge under `[mpd].link_dir` (default `<music_dir>/.music-tui-links`)
/// plus a scoped database update.
async fn resolve_outside_uris(
  client: &Client,
  outside: &[PathBuf],
  mpd_config: &MpdConfig,
  music_dir: Option<&Path>,
) -> Result<Vec<String>> {
  if is_socket_host(&mpd_config.host) {
    return Ok(outside.iter().map(|path| file_uri(path)).collect());
  }
  let Some(music_dir) = music_dir else {
    bail!(
      "cannot open local files over TCP without a music directory; configure mpd.music_dir or connect through a Unix socket"
    );
  };
  let dir = links_dir(music_dir, &mpd_config.link_dir);
  let mut links = Vec::with_capacity(outside.len());
  for path in outside {
    links.push(ensure_link(&dir, path).map_err(anyhow::Error::msg)?);
  }
  let dir_uri = path_to_uri(music_dir, &dir)?;
  update_and_wait(client, &dir_uri).await?;
  links
    .iter()
    .map(|link| path_to_uri(music_dir, link))
    .collect::<std::result::Result<Vec<_>, _>>()
}

/// Send a scoped `update` and wait until MPD finishes (max ~15s).
async fn update_and_wait(client: &Client, uri: &str) -> Result<()> {
  client.command(Update::new().uri(uri)).await?;
  let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
  while tokio::time::Instant::now() < deadline {
    let status = client.command(Status).await?;
    if status.update_job.is_none() {
      return Ok(());
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
  }
  bail!("timed out waiting for the database update")
}

async fn open_folder(
  client: &Client,
  folder: &Path,
  mpd_config: &MpdConfig,
  music_dir: Option<&Path>,
  recursive: bool,
  no_play: bool,
) -> Result<String> {
  let files = collect_audio_files(folder, recursive)?;
  if files.is_empty() {
    bail!("no audio files found under {}", folder.display());
  }
  let uris = resolve_open_uris(client, &files, mpd_config, music_dir).await?;
  let queued = replace_queue(
    client,
    &uris,
    format_args!("the files in {}", folder.display()),
  )
  .await?;
  let notice = format!(
    "queued {} song(s) from {}{}",
    queued.added(),
    folder.display(),
    queued.skipped_note()
  );
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
  mpd_config: &MpdConfig,
  music_dir: Option<&Path>,
  recursive: bool,
  no_play: bool,
  dedup: bool,
) -> Result<String> {
  let files = collect_audio_files(folder, recursive)?;
  if files.is_empty() {
    bail!("no audio files found under {}", folder.display());
  }
  let mut uris = resolve_open_uris(client, &files, mpd_config, music_dir).await?;
  if dedup {
    skip_queued_and_batch_dups(client, &mut uris, true).await?;
  }
  if uris.is_empty() {
    return Ok(format!(
      "all songs from {} already queued",
      folder.display()
    ));
  }
  let queued = add_playable(client, &uris, None)
    .await?
    .require_any(format_args!("the files in {}", folder.display()))?;
  let notice = format!(
    "appended {} song(s) from {}{}",
    queued.added(),
    folder.display(),
    queued.skipped_note()
  );
  if !no_play {
    maybe_start_if_idle(client).await?;
  }
  Ok(notice)
}

/// Songs added by [`add_playable`].
struct Queued {
  /// For each requested uri, its position among the added songs, or `None`
  /// when MPD refused it.
  positions: Vec<Option<usize>>,
}

impl Queued {
  fn added(&self) -> usize {
    self.positions.iter().flatten().count()
  }

  fn skipped(&self) -> usize {
    self.positions.len() - self.added()
  }

  /// Fail when MPD refused every file, describing them as `what`.
  fn require_any(self, what: impl Display) -> Result<Self> {
    if self.added() == 0 {
      bail!("MPD can't play {what}");
    }
    Ok(self)
  }

  /// `, skipped N files MPD can't play`, or nothing when none were.
  fn skipped_clause(&self) -> String {
    match self.skipped() {
      0 => String::new(),
      1 => ", skipped 1 file MPD can't play".to_string(),
      skipped => format!(", skipped {skipped} files MPD can't play"),
    }
  }

  /// [`Self::skipped_clause`] as a parenthesized notice suffix.
  fn skipped_note(&self) -> String {
    match self.skipped_clause().strip_prefix(", ") {
      Some(clause) => format!(" ({clause})"),
      None => String::new(),
    }
  }
}

/// Add `uris` in order, at the end of the queue or from position `at`,
/// skipping files MPD refuses: a video-only `.mp4` has no audio for MPD to
/// decode (`ACK [50] No such song`). Other failures, such as a lost
/// connection, still abort.
async fn add_playable(client: &Client, uris: &[String], at: Option<usize>) -> Result<Queued> {
  let mut positions = Vec::with_capacity(uris.len());
  let mut added = 0;
  for uri in uris {
    let add = match at {
      Some(start) => Add::uri(uri).at(start + added),
      None => Add::uri(uri),
    };
    match client.command(add).await {
      Ok(_) => {
        positions.push(Some(added));
        added += 1;
      }
      Err(error @ CommandError::ErrorResponse { .. }) => {
        warn!(uri, %error, "MPD refused a file; skipping it");
        positions.push(None);
      }
      Err(error) => return Err(error.into()),
    }
  }
  Ok(Queued { positions })
}

/// Replace the queue with `uris`. The new songs go behind the current queue
/// and the old songs are removed only once MPD accepted at least one, so
/// files MPD can't play leave the queue as it was.
async fn replace_queue(client: &Client, uris: &[String], what: impl Display) -> Result<Queued> {
  let old_len = client.command(Status).await?.playlist_length;
  let queued = add_playable(client, uris, None).await?.require_any(what)?;
  if old_len > 0 {
    client
      .command(Delete::range(SongPosition(0)..SongPosition(old_len)))
      .await?;
  }
  Ok(queued)
}

/// Add one song, naming it when MPD refuses it.
async fn add_one(client: &Client, add: Add<'_>, name: &str) -> Result<()> {
  client
    .command(add)
    .await
    .with_context(|| format!("MPD can't play {name}"))?;
  Ok(())
}

pub(crate) async fn maybe_start_if_idle(client: &Client) -> Result<()> {
  let status = client.command(Status).await?;
  if status.state == PlayState::Stopped {
    client.command(Play::current()).await?;
  }
  Ok(())
}

/// Whether the queue already contains `uri` (add-time dedup).
async fn queue_has(client: &Client, uri: &str) -> Result<bool> {
  let key = song_key(uri);
  let queue = client.command(Queue).await?;
  Ok(queue.iter().any(|song| song_key(&song.song.url) == key))
}

/// Drop URIs already queued and duplicates within the batch itself;
/// `include_queue` false limits the check to batch-internal duplicates
/// (replace modes clear the queue first).
async fn skip_queued_and_batch_dups(
  client: &Client,
  uris: &mut Vec<String>,
  include_queue: bool,
) -> Result<()> {
  // Seed the seen-set with the queue: a uri is kept only the first time
  // its song key shows up.
  let mut seen: HashSet<_> = if include_queue {
    client
      .command(Queue)
      .await?
      .iter()
      .map(|song| song_key(&song.song.url))
      .collect()
  } else {
    HashSet::new()
  };
  uris.retain(|uri| seen.insert(song_key(uri)));
  Ok(())
}

fn short_name(path: &Path) -> String {
  path
    .file_name()
    .map(|name| name.to_string_lossy().into_owned())
    .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn config(host: &str) -> MpdConfig {
    MpdConfig {
      host: host.to_string(),
      ..MpdConfig::default()
    }
  }

  #[cfg(unix)]
  #[test]
  fn socket_uses_file_uri_without_music_dir() {
    let path = Path::new("/tmp/Music/a song.flac");
    assert_eq!(
      direct_open_uri(path, &config("/tmp/mpd.sock"), None),
      Some(file_uri(path)),
    );
  }

  #[test]
  fn tcp_requires_music_dir_for_local_files() {
    assert_eq!(
      direct_open_uri(Path::new("/tmp/song.flac"), &config("127.0.0.1"), None),
      None,
    );
  }

  #[test]
  fn configured_library_uses_relative_uri() {
    assert_eq!(
      direct_open_uri(
        Path::new("/music/Artist/song.flac"),
        &config("127.0.0.1"),
        Some(Path::new("/music")),
      ),
      Some("Artist/song.flac".to_string()),
    );
  }
}
