//! Spectrum visualizer: a reader thread takes s16le PCM from the MPD fifo
//! output ([`fifo`]), runs an FFT per frame ([`analysis`]) and forwards
//! log-spaced band values (0..=100) to the UI; a second thread turns them
//! into styled lines ([`render`]). The band count follows the pane width
//! (one band per column, capped by `bars`).
//!
//! The reader blocks in `poll(2)` (no busy waiting), only analyzes fresh
//! samples (a paused player costs nothing), and leaves the fifo alone
//! while the visualizer pane is not on screen — MPD drains a full fifo
//! itself.

#[cfg(unix)]
mod analysis;
#[cfg(unix)]
mod fifo;
mod render;

use std::sync::{
  Arc,
  atomic::{AtomicBool, AtomicUsize, Ordering},
};
#[cfg(unix)]
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::{config::VisualizerConfig, event::AsyncEvent};

pub use render::{BandRendererHandle, spawn_band_renderer};

pub(crate) use render::VisualizerColors;

#[derive(Clone)]
pub struct VisualizerHandle {
  stop: Arc<AtomicBool>,
  /// Whether the visualizer pane is on screen; analysis pauses otherwise.
  active: Arc<AtomicBool>,
  /// Desired band count, driven by the pane width reported by the UI
  /// (one band per column, capped by `visualizer.bars`).
  columns: Arc<AtomicUsize>,
}

impl VisualizerHandle {
  pub fn stop(&self) {
    self.stop.store(true, Ordering::SeqCst);
  }

  /// Report whether the visualizer pane is currently displayed.
  pub fn set_active(&self, active: bool) {
    self.active.store(active, Ordering::Relaxed);
  }

  /// Report the current pane width so the analysis matches the columns.
  pub fn set_columns(&self, columns: usize) {
    self.columns.store(columns.max(1), Ordering::Relaxed);
  }
}

#[cfg(unix)]
pub fn spawn_visualizer(
  config: VisualizerConfig,
  events: mpsc::UnboundedSender<AsyncEvent>,
) -> Option<VisualizerHandle> {
  let handle = VisualizerHandle {
    stop: Arc::new(AtomicBool::new(false)),
    active: Arc::new(AtomicBool::new(false)),
    // Until the UI reports a pane width, analyze at the configured cap.
    columns: Arc::new(AtomicUsize::new(config.bars.max(1))),
  };
  let worker = handle.clone();
  let spawned = std::thread::Builder::new()
    .name("music-tui-visualizer".to_string())
    .spawn(move || run(config, events, worker));
  match spawned {
    Ok(_) => Some(handle),
    Err(error) => {
      tracing::warn!(%error, "failed to start the visualizer");
      None
    }
  }
}

#[cfg(not(unix))]
pub fn spawn_visualizer(
  _config: VisualizerConfig,
  _events: mpsc::UnboundedSender<AsyncEvent>,
) -> Option<VisualizerHandle> {
  None
}

#[cfg(unix)]
fn run(
  config: VisualizerConfig,
  events: mpsc::UnboundedSender<AsyncEvent>,
  handle: VisualizerHandle,
) {
  let mut analyzer = analysis::Analyzer::new(&config);
  let frame_period = Duration::from_secs_f64(1.0 / f64::from(config.fps.max(1)));
  let mut read_buf = vec![0u8; analyzer.window * analyzer.channels * 2 * 2];
  let mut last_error: Option<String> = None;

  while !handle.stop.load(Ordering::SeqCst) {
    let fifo = match fifo::open_fifo(&config.fifo_path) {
      Ok(file) => file,
      Err(error) => {
        let busy = error.kind() == std::io::ErrorKind::ResourceBusy;
        let message = error.to_string();
        if last_error.as_deref() != Some(message.as_str()) {
          tracing::info!("visualizer fifo unavailable: {message}");
          last_error = Some(message);
        }
        // Another instance owns the fifo: retry slowly until it exits.
        sleep_unless_stopped(&handle.stop, Duration::from_secs(if busy { 5 } else { 2 }));
        continue;
      }
    };
    last_error = None;
    analyzer.reset();
    if !fifo::pump_fifo(
      fifo,
      &handle,
      &mut analyzer,
      &mut read_buf,
      frame_period,
      &events,
    ) {
      return;
    }
    // The fifo failed: give whatever broke a moment before reopening.
    sleep_unless_stopped(&handle.stop, Duration::from_millis(500));
  }
}

#[cfg(unix)]
fn sleep_unless_stopped(stop: &AtomicBool, total: Duration) {
  let deadline = Instant::now() + total;
  while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
    std::thread::sleep(fifo::FIFO_WAIT.min(deadline.saturating_duration_since(Instant::now())));
  }
}
