//! MPD connection worker.
//!
//! Owns the [`Client`], executes commands from the app, watches connection
//! events for subsystem changes, and periodically refreshes a full
//! status + queue snapshot which is forwarded to the UI. Also implements the
//! "interrupt preview" lifecycle used by `music-tui open --mode interrupt`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mpd_client::{
  client::ConnectionEvent,
  commands::{
    self, Add, ClearQueue, Delete, DeletePlaylist, LoadPlaylist, Play, Previous,
    SaveQueueAsPlaylist, Seek, SeekMode, SetConsume, SetPause, SetRandom, SetRepeat, SetSingle,
    SetVolume, Shuffle, Stop, SingleMode, SongPosition,
  },
  responses::{PlayState, SongInQueue, Status},
  Client,
};
use tokio::{net::TcpStream, sync::mpsc, time::sleep};
use tracing::{debug, info, warn};

use crate::{
  config::MpdConfig,
  event::{AsyncEvent, MpdEvent},
};

/// Saved playback state to restore after an interrupt preview finishes.
#[derive(Debug, Clone)]
pub struct InterruptSession {
  /// Stored playlist holding the previous queue, if the queue was non-empty.
  pub playlist: Option<String>,
  pub was_playing: bool,
  pub position: Option<u32>,
  pub elapsed_secs: Option<f64>,
  pub single: SingleMode,
}

#[derive(Debug)]
pub enum MpdCommand {
  PlayPosition(u32),
  PlayPauseToggle,
  Pause(bool),
  Stop,
  Next,
  Previous,
  SetVolume(u8),
  NudgeVolume(i16),
  SeekCurrent(f64),
  NudgeSeek(i64),
  SetRepeat(bool),
  SetRandom(bool),
  SetSingle(SingleMode),
  SetConsume(bool),
  ClearQueue,
  /// Shuffle the entire queue.
  Shuffle,
  DeleteAt(usize),
  /// Queue-dedup maintenance: drop duplicate positions (highest first).
  /// Deliberately NOT in `command_touches_queue` — housekeeping must not
  /// cancel an armed interrupt-preview session.
  DedupDelete(Vec<usize>),
  AddUri(String),
  /// Play (or append) a local file from the library pane. The path is
  /// resolved to an MPD URI (music dir relative, `file://`, or symlink
  /// bridge) before being added to the queue.
  PlayLibrary {
    path: std::path::PathBuf,
    append: bool,
  },
  Rescan,
  /// Incremental database update for one URI (used after tag writes).
  UpdateUri(String),
  ArmInterrupt(InterruptSession),
}

#[derive(Clone)]
pub struct MpdHandle {
  tx: mpsc::UnboundedSender<MpdCommand>,
  queue_dedup: Arc<AtomicBool>,
}

impl MpdHandle {
  pub fn send(&self, command: MpdCommand) {
    let _ = self.tx.send(command);
  }

  /// Live toggle for add-time duplicate skipping in the worker.
  pub fn set_queue_dedup(&self, on: bool) {
    self.queue_dedup.store(on, Ordering::Relaxed);
  }
}

pub fn spawn_mpd_worker(
  config: MpdConfig,
  events: mpsc::UnboundedSender<AsyncEvent>,
) -> MpdHandle {
  let (tx, mut rx) = mpsc::unbounded_channel();
  let queue_dedup = Arc::new(AtomicBool::new(false));
  let worker_dedup = queue_dedup.clone();
  tokio::spawn(async move {
    let mut backoff = Duration::from_secs(1);
    loop {
      match connect(&config).await {
        Ok((client, mut connection_events)) => {
          backoff = Duration::from_secs(1);
          let address = describe_address(&config);
          info!(%address, "connected to mpd");
          let _ = events.send(AsyncEvent::Mpd(MpdEvent::Connected(address)));
          if let Err(error) = run_session(
            &client,
            &mut connection_events,
            &mut rx,
            &events,
            &config,
            &worker_dedup,
          )
          .await
          {
            warn!(%error, "mpd session ended");
          }
          let _ = events.send(AsyncEvent::Mpd(MpdEvent::ConnectionLost(
            "connection closed".to_string(),
          )));
        }
        Err(error) => {
          warn!(%error, "failed to connect to mpd");
          let _ = events.send(AsyncEvent::Mpd(MpdEvent::ConnectionLost(format!(
            "connect failed: {error}"
          ))));
        }
      }
      debug!(?backoff, "reconnecting to mpd");
      sleep(backoff).await;
      backoff = (backoff * 2).min(Duration::from_secs(30));
    }
  });
  MpdHandle { tx, queue_dedup }
}

pub async fn connect(config: &MpdConfig) -> anyhow::Result<(Client, mpd_client::client::ConnectionEvents)> {
  let host = crate::config::expand_home(&config.host);
  if host.to_string_lossy().starts_with('/') {
    let stream = tokio::net::UnixStream::connect(&host).await?;
    let (client, events) = Client::connect_with_password_opt(stream, config.password.as_deref())
      .await
      .map_err(|error| anyhow::anyhow!(format!("{error}")))?;
    return Ok((client, events));
  }

  let stream = TcpStream::connect((host.to_string_lossy().as_ref(), config.port)).await?;
  let (client, events) = Client::connect_with_password_opt(stream, config.password.as_deref())
    .await
    .map_err(|error| anyhow::anyhow!(format!("{error}")))?;
  Ok((client, events))
}

fn describe_address(config: &MpdConfig) -> String {
  if config.host.starts_with('/') {
    config.host.clone()
  } else {
    format!("{}:{}", config.host, config.port)
  }
}

mod interrupt;
mod worker;
use worker::run_session;

pub use interrupt::capture_interrupt_session;
