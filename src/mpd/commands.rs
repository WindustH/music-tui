//! Execution of app commands against a connected client, including
//! add-time duplicate skipping.

use std::collections::HashMap;
use std::time::Duration;

use mpd_client::{
  Client,
  client::CommandError,
  commands::{
    self, Add, ClearQueue, Command, Delete, Play, Previous, Seek, SeekMode, SetConsume, SetPause,
    SetRandom, SetRepeat, SetSingle, SetVolume, Shuffle, SongId, Stop,
  },
  responses::{PlayState, SongInQueue},
};
use tracing::{debug, warn};

use super::{MpdCommand, worker::Cache};
use crate::{
  config::MpdConfig,
  library::{SongKey, song_key},
};

/// Longest absolute seek target accepted from the UI (sanity bound).
const MAX_SEEK_SECS: f64 = 24.0 * 60.0 * 60.0;

/// Dedup lookup over the live queue: song key → id of its first
/// occurrence. Built lazily (only when dedup needs it) from the cached
/// queue; commands that remove entries mark it stale so the next lookup
/// re-reads the queue, and successful adds record their new ids, so checks
/// inside one command batch stay exact without a queue fetch per command.
#[derive(Default)]
pub(super) struct QueueIndex {
  map: Option<HashMap<SongKey, SongId>>,
  /// The cached queue no longer reflects the server (entries removed).
  stale: bool,
}

impl QueueIndex {
  fn invalidate(&mut self) {
    self.map = None;
    self.stale = true;
  }

  async fn lookup(
    &mut self,
    client: &Client,
    cached: &[SongInQueue],
    key: &SongKey,
  ) -> Result<Option<SongId>, String> {
    if self.map.is_none() {
      let fetched;
      let queue = if self.stale {
        fetched = client
          .command(commands::Queue)
          .await
          .map_err(|e| e.to_string())?;
        fetched.as_slice()
      } else {
        cached
      };
      let mut map = HashMap::with_capacity(queue.len());
      for song in queue {
        map.entry(song_key(&song.song.url)).or_insert(song.id);
      }
      self.map = Some(map);
      self.stale = false;
    }
    Ok(self.map.as_ref().and_then(|map| map.get(key).copied()))
  }

  fn record_add(&mut self, key: SongKey, id: SongId) {
    match self.map.as_mut() {
      Some(map) => {
        map.entry(key).or_insert(id);
      }
      // Not built yet: the cached queue lacks this entry, so the next
      // lookup must read the server's queue.
      None => self.stale = true,
    }
  }
}

/// Run a command whose response carries nothing the worker needs.
async fn exec<C: Command>(client: &Client, command: C) -> Result<(), String> {
  client
    .command(command)
    .await
    .map(drop)
    .map_err(|error| error.to_string())
}

/// Run one app command; failures are logged here and returned so the
/// caller can surface the ones the user should see.
pub(super) async fn run(
  client: &Client,
  command: MpdCommand,
  config: &MpdConfig,
  cache: &mut Cache,
  index: &mut QueueIndex,
  dedup: bool,
) -> Result<(), String> {
  let outcome = match command {
    MpdCommand::PlaySong(id) => exec(client, Play::song(id)).await,
    MpdCommand::PlayPauseToggle => match client.command(commands::Status).await {
      Ok(status) if status.state == PlayState::Playing => exec(client, SetPause(true)).await,
      _ => exec(client, Play::current()).await,
    },
    MpdCommand::Pause(pause) => exec(client, SetPause(pause)).await,
    MpdCommand::Stop => exec(client, Stop).await,
    MpdCommand::Next => exec(client, commands::Next).await,
    MpdCommand::Previous => exec(client, Previous).await,
    MpdCommand::SetVolume(volume) => set_volume(client, volume).await,
    MpdCommand::NudgeVolume(delta) => match client.command(commands::Status).await {
      Ok(status) => {
        let next = (i32::from(status.volume) + i32::from(delta)).clamp(0, 100) as u8;
        set_volume(client, next).await
      }
      Err(error) => Err(error.to_string()),
    },
    MpdCommand::SeekCurrent(seconds) => {
      let seconds = if seconds.is_finite() {
        seconds.clamp(0.0, MAX_SEEK_SECS)
      } else {
        0.0
      };
      exec(
        client,
        Seek(SeekMode::Absolute(Duration::from_secs_f64(seconds))),
      )
      .await
    }
    MpdCommand::NudgeSeek(delta) => {
      let step = Duration::from_secs(delta.unsigned_abs());
      let mode = if delta >= 0 {
        SeekMode::Forward(step)
      } else {
        SeekMode::Backward(step)
      };
      exec(client, Seek(mode)).await
    }
    MpdCommand::SetRepeat(repeat) => exec(client, SetRepeat(repeat)).await,
    MpdCommand::SetRandom(random) => exec(client, SetRandom(random)).await,
    MpdCommand::SetSingle(mode) => exec(client, SetSingle(mode)).await,
    MpdCommand::SetConsume(consume) => exec(client, SetConsume(consume)).await,
    MpdCommand::ClearQueue => {
      index.invalidate();
      exec(client, ClearQueue).await
    }
    MpdCommand::Shuffle => exec(client, Shuffle::all()).await,
    MpdCommand::Delete(id) => {
      index.invalidate();
      exec(client, Delete::id(id)).await
    }
    MpdCommand::AddUri(uri) => add_uri(client, &uri, cache, index, dedup).await,
    MpdCommand::PlayLibrary { path, append } => {
      play_library_file(client, &path, append, config, cache, index, dedup).await
    }
    MpdCommand::Rescan => exec(client, commands::Rescan::new()).await,
    MpdCommand::UpdateUri(uri) => exec(client, commands::Update::new().uri(&uri)).await,
    MpdCommand::ArmInterrupt(_) => Ok(()),
  };
  if let Err(error) = &outcome {
    warn!(%error, "mpd command failed");
  }
  outcome
}

/// `setvol`, failing with MPD's own reason (such as "No mixer") so the
/// notice stays short.
async fn set_volume(client: &Client, volume: u8) -> Result<(), String> {
  match client.command(SetVolume(volume.min(100))).await {
    Ok(()) => Ok(()),
    Err(CommandError::ErrorResponse { error, .. }) => Err(error.message.into()),
    Err(error) => Err(error.to_string()),
  }
}

async fn add_uri(
  client: &Client,
  uri: &str,
  cache: &mut Cache,
  index: &mut QueueIndex,
  dedup: bool,
) -> Result<(), String> {
  let key = song_key(uri);
  if dedup && index.lookup(client, &cache.queue, &key).await?.is_some() {
    debug!(uri = %uri, "skip add: already queued (dedup on)");
    return Ok(());
  }
  let id = client
    .command(Add::uri(uri))
    .await
    .map_err(|error| error.to_string())?;
  index.record_add(key, id);
  Ok(())
}

/// Resolve a library-pane file to an MPD uri and insert it into the queue:
/// `append` adds to the end (starting playback when idle), otherwise the
/// track is inserted right after the current song (or at the end when
/// nothing plays) and starts immediately. With dedup on, a song that is
/// already queued is never re-added — playback simply jumps to the
/// existing entry.
async fn play_library_file(
  client: &Client,
  path: &std::path::Path,
  append: bool,
  config: &MpdConfig,
  cache: &mut Cache,
  index: &mut QueueIndex,
  dedup: bool,
) -> Result<(), String> {
  let music_dir = crate::library::resolve_music_dir(config).ok();
  let uri = crate::open::resolve_open_uri(client, path, config, music_dir.as_deref())
    .await
    .map_err(|error| error.to_string())?;
  let key = song_key(&uri);

  if dedup && let Some(id) = index.lookup(client, &cache.queue, &key).await? {
    // Already queued: skip the add and reuse the existing entry.
    if append {
      return crate::open::maybe_start_if_idle(client)
        .await
        .map_err(|error| error.to_string());
    }
    return exec(client, Play::song(id)).await;
  }

  if append {
    let id = client
      .command(Add::uri(&uri))
      .await
      .map_err(|error| error.to_string())?;
    index.record_add(key, id);
    return crate::open::maybe_start_if_idle(client)
      .await
      .map_err(|error| error.to_string());
  }

  let status = client
    .command(commands::Status)
    .await
    .map_err(|error| error.to_string())?;
  let add = match status.current_song {
    Some(_) => Add::uri(&uri).after_current(0),
    None => Add::uri(&uri),
  };
  let id = client
    .command(add)
    .await
    .map_err(|error| error.to_string())?;
  index.record_add(key, id);
  exec(client, Play::song(id)).await
}
