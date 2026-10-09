//! MPD worker session: connection event loop, status/queue refresh ticks,
//! batched command dispatch and the interrupt-session restore hook.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mpd_client::{
  Client,
  client::{ConnectionEvent, ConnectionEvents},
  commands,
  responses::{PlayState, SongInQueue, Status},
};
use tokio::{sync::mpsc, time::sleep};
use tracing::debug;

use super::{MpdCommand, commands::QueueIndex, interrupt::RestoreState};
use crate::{
  config::{BehaviorConfig, MpdConfig},
  event::{AsyncEvent, MpdEvent},
};

/// One connected session; borrows the worker's long-lived state.
pub(super) struct Session<'a> {
  pub client: &'a Client,
  pub commands: &'a mut mpsc::UnboundedReceiver<MpdCommand>,
  pub events: &'a mpsc::UnboundedSender<AsyncEvent>,
  pub config: &'a MpdConfig,
  pub behavior: &'a BehaviorConfig,
  pub dedup: &'a Arc<AtomicBool>,
  pub restore: &'a mut RestoreState,
}

/// What the worker last saw (and forwarded) of the server state.
#[derive(Default)]
pub(super) struct Cache {
  playing: bool,
  pub(super) queue: Arc<[SongInQueue]>,
  /// Playlist version the cached queue belongs to; `None` forces a
  /// re-fetch on the next refresh (first refresh, or a local change).
  queue_version: Option<u32>,
  /// Last status forwarded to the UI; an identical poll result (paused or
  /// stopped playback) is not re-sent, so an idle player costs the UI
  /// nothing.
  last_sent: Option<Status>,
}

impl Cache {
  /// The queue was replaced behind the cache's back: re-fetch it.
  fn invalidate_queue(&mut self) {
    self.queue_version = None;
  }
}

impl Session<'_> {
  pub(super) async fn run(
    mut self,
    connection_events: &mut ConnectionEvents,
  ) -> anyhow::Result<()> {
    let mut cache = Cache::default();
    self.refresh(&mut cache).await?;

    let (tick_idle, tick_playing) = self.behavior.refresh_durations();
    loop {
      let period = if cache.playing {
        tick_playing
      } else {
        tick_idle
      };
      tokio::select! {
        event = connection_events.next() => match event {
          Some(ConnectionEvent::SubsystemChange(subsystem)) => {
            debug!(subsystem = subsystem.as_str(), "mpd subsystem changed");
            self.refresh(&mut cache).await?;
          }
          Some(ConnectionEvent::ConnectionClosed(error)) => {
            return Err(anyhow::anyhow!(format!("{error}")));
          }
          None => {
            return Err(anyhow::anyhow!("connection event stream ended"));
          }
        },
        command = self.commands.recv() => {
          let Some(command) = command else {
            return Err(anyhow::anyhow!("command channel closed"));
          };
          // Run everything already queued before refreshing once: a burst
          // (`:add` of a folder, a held key) costs one queue fetch instead
          // of one per command. The dedup index lives for one batch (the
          // refresh below replaces the cached queue it derives from).
          let mut index = QueueIndex::default();
          self.dispatch(command, &mut cache, &mut index).await;
          while let Ok(command) = self.commands.try_recv() {
            self.dispatch(command, &mut cache, &mut index).await;
          }
          self.refresh(&mut cache).await?;
        }
        _ = sleep(period) => {
          self.refresh(&mut cache).await?;
        }
      }
    }
  }

  async fn dispatch(&mut self, command: MpdCommand, cache: &mut Cache, index: &mut QueueIndex) {
    if let MpdCommand::ArmInterrupt(session) = command {
      self.restore.arm(session);
      return;
    }
    if command_touches_queue(&command) {
      self.restore.disarm();
    }
    let volume = matches!(
      command,
      MpdCommand::SetVolume(_) | MpdCommand::NudgeVolume(_)
    );
    let dedup = self.dedup.load(Ordering::Relaxed);
    let outcome =
      super::commands::run(self.client, command, self.config, cache, index, dedup).await;
    // A volume key that silently does nothing is the usual sign of an
    // output without a working mixer; say why.
    if let Err(error) = outcome
      && volume
    {
      let _ = self.events.send(AsyncEvent::Mpd(MpdEvent::Notice(format!(
        "MPD can't change the volume: {error}"
      ))));
    }
  }

  /// Fetch the status, restore a finished interrupt preview, re-fetch the
  /// queue when its version moved, and forward what changed to the UI.
  async fn refresh(&mut self, cache: &mut Cache) -> anyhow::Result<()> {
    let status = loop {
      let status = self.client.command(commands::Status).await?;
      // A performed restore replaced the queue and playback state: read
      // the fresh status. It disarms itself, so this loops at most once.
      if !self
        .restore
        .maybe_restore(self.client, &status, self.events)
        .await?
      {
        break status;
      }
      cache.invalidate_queue();
    };
    cache.playing = status.state == PlayState::Playing;

    let queue = if cache.queue_version != Some(status.playlist_version) {
      let queue: Arc<[SongInQueue]> = self.client.command(commands::Queue).await?.into();
      cache.queue = queue.clone();
      cache.queue_version = Some(status.playlist_version);
      Some(queue)
    } else {
      None
    };
    if queue.is_none() && cache.last_sent.as_ref() == Some(&status) {
      return Ok(());
    }
    cache.last_sent = Some(status.clone());
    let _ = self
      .events
      .send(AsyncEvent::Mpd(MpdEvent::Snapshot { status, queue }));
    Ok(())
  }
}

/// Commands that hand the queue back to the user: an armed interrupt
/// preview no longer owns it once any of these ran.
fn command_touches_queue(command: &MpdCommand) -> bool {
  matches!(
    command,
    MpdCommand::PlaySong(_)
      | MpdCommand::PlayPauseToggle
      | MpdCommand::Stop
      | MpdCommand::Next
      | MpdCommand::Previous
      | MpdCommand::ClearQueue
      | MpdCommand::Shuffle
      | MpdCommand::Delete(_)
      | MpdCommand::AddUri(_)
      | MpdCommand::PlayLibrary { .. }
  )
}
