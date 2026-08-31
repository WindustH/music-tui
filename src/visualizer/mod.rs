//! Spectrum analysis: reads s16le PCM from the MPD fifo output, runs an
//! FFT per frame, and forwards log-spaced band values (0..=100) to the UI.
//! The band count follows the pane width (one band per column, capped by
//! `bars`). Rendering lives in [`render`].

mod render;

use std::sync::{
  Arc,
  atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[cfg(unix)]
use std::{
  io::{Read, Result as IoResult},
  time::{Duration, Instant},
};

#[cfg(unix)]
use rustfft::{Fft, FftPlanner, num_complex::Complex};
use tokio::sync::mpsc;

use crate::{config::VisualizerConfig, event::AsyncEvent};

pub use render::{BandRendererHandle, spawn_band_renderer};

pub(crate) use render::VisualizerColors;

pub struct VisualizerHandle {
  stop: Arc<AtomicBool>,
  /// Desired band count, driven by the pane width reported by the UI
  /// (one band per column, capped by `visualizer.bars`).
  columns: Arc<AtomicUsize>,
}

impl Clone for VisualizerHandle {
  fn clone(&self) -> Self {
    Self {
      stop: self.stop.clone(),
      columns: self.columns.clone(),
    }
  }
}

impl VisualizerHandle {
  pub fn stop(&self) {
    self.stop.store(true, Ordering::SeqCst);
  }

  /// Report the current pane width so the analysis matches the columns.
  pub fn set_columns(&self, columns: usize) {
    self.columns.store(columns.max(1), Ordering::Relaxed);
  }
}

/// Band edges in Hz, log-spaced with a minimum linear step of one FFT bin:
/// every band owns at least one distinct bin, so neighboring low-frequency
/// bands (where a pure log grid is narrower than the FFT resolution) never
/// sample the same bin and render as duplicated identical bars.
#[cfg(unix)]
fn band_edges(hint: usize, hz_per_bin: f32, min_freq: f32, max_freq: f32) -> Vec<f32> {
  let hint = hint.max(2);
  let mut edges = build_band_edges(
    (max_freq / min_freq).powf(1.0 / hint as f32),
    hz_per_bin,
    min_freq,
    max_freq,
  );
  // Refine the ratio so the generated band count matches its own hint.
  for _ in 0..8 {
    let count = edges.len() - 1;
    if count < 2 {
      break;
    }
    let next_edges = build_band_edges(
      (max_freq / min_freq).powf(1.0 / count as f32),
      hz_per_bin,
      min_freq,
      max_freq,
    );
    if next_edges.len() == edges.len() {
      break;
    }
    edges = next_edges;
  }
  edges
}

#[cfg(unix)]
fn build_band_edges(ratio: f32, hz_per_bin: f32, min_freq: f32, max_freq: f32) -> Vec<f32> {
  let mut edges = vec![min_freq];
  loop {
    let prev = *edges.last().expect("non-empty");
    let next = (prev * ratio).max(prev + hz_per_bin);
    if next >= max_freq {
      edges.push(max_freq);
      return edges;
    }
    edges.push(next);
  }
}

/// Map band edges to FFT bin ranges `[start, end)`; every range contains
/// at least one bin and consecutive ranges are disjoint.
#[cfg(unix)]
fn band_bin_ranges(edges: &[f32], window: usize, sample_rate: u32) -> Vec<(usize, usize)> {
  let bins = window / 2;
  let nyquist = sample_rate as f32 / 2.0;
  edges
    .windows(2)
    .map(|pair| {
      let start_bin = ((pair[0] / nyquist) * bins as f32).floor().max(1.0) as usize;
      let end_bin =
        (((pair[1] / nyquist) * bins as f32).ceil() as usize).clamp(start_bin + 1, bins);
      (start_bin, end_bin)
    })
    .fold(Vec::new(), |mut ranges, range| {
      // Never overlap the previous band: keep every range disjoint so no
      // two bands read identical bins.
      let start = match ranges.last() {
        Some(&(_, prev_end)) => range.0.max(prev_end),
        None => range.0,
      };
      let end = range.1.max(start + 1).min(bins.max(1));
      ranges.push((start, end));
      ranges
    })
}

#[cfg(unix)]
pub fn spawn_visualizer(
  config: VisualizerConfig,
  events: mpsc::UnboundedSender<AsyncEvent>,
) -> Option<VisualizerHandle> {
  let stop = Arc::new(AtomicBool::new(false));
  // Until the UI reports a pane width, analyze at the configured cap.
  let columns = Arc::new(AtomicUsize::new(config.bars.max(1)));
  let handle = VisualizerHandle {
    stop: stop.clone(),
    columns: columns.clone(),
  };
  std::thread::Builder::new()
    .name("music-tui-visualizer".to_string())
    .spawn(move || {
      run(config, events, stop, columns);
    })
    .expect("failed to spawn visualizer thread");
  Some(handle)
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
  stop: Arc<AtomicBool>,
  columns: Arc<AtomicUsize>,
) {
  let window = config.window.max(256);
  let channels = config.channels.max(1) as usize;
  let mut planner: FftPlanner<f32> = FftPlanner::new();
  let fft: Arc<dyn Fft<f32>> = planner.plan_fft_forward(window);
  let hann: Vec<f32> = (0..window)
    .map(|index| 0.5 * (1.0 - (std::f32::consts::TAU * index as f32 / window as f32).cos()))
    .collect();

  let mut columns_now = config.bars.max(1);
  let hz_per_bin = config.sample_rate as f32 / window as f32;
  let min_freq = 40.0f32;
  let max_freq = (config.sample_rate as f32 / 2.0).min(16_000.0);
  let mut bin_ranges = band_bin_ranges(
    &band_edges(columns_now, hz_per_bin, min_freq, max_freq),
    window,
    config.sample_rate,
  );
  let mut samples: Vec<f32> = Vec::with_capacity(window * channels * 2);
  let mut bars: Vec<u8> = vec![0; bin_ranges.len()];
  let mut leftover = Vec::new();
  let frame_period = Duration::from_secs_f64(1.0 / config.fps.max(1) as f64);
  let mut last_frame = Instant::now();
  let mut read_buf = vec![0u8; window * channels * 2 * 2];

  let path = config.fifo_path.clone();
  loop {
    if stop.load(Ordering::SeqCst) {
      return;
    }
    let opened = open_fifo(&path);
    let mut fifo = match opened {
      Ok(file) => file,
      Err(error) if error.kind() == std::io::ErrorKind::ResourceBusy => {
        // Another instance owns the fifo; retry slowly until it exits.
        tracing::debug!("{error}");
        std::thread::sleep(Duration::from_secs(5));
        continue;
      }
      Err(_) => {
        std::thread::sleep(Duration::from_secs(2));
        continue;
      }
    };

    loop {
      if stop.load(Ordering::SeqCst) {
        return;
      }
      match fifo.read(&mut read_buf) {
        Ok(0) => {
          // Writer vanished; reopen.
          break;
        }
        Ok(read) => {
          leftover.extend_from_slice(&read_buf[..read]);
          while leftover.len() >= 2 {
            let bytes: [u8; 2] = [leftover[0], leftover[1]];
            leftover.drain(..2);
            let sample = i16::from_le_bytes(bytes) as f32 / 32768.0;
            samples.push(sample);
          }
        }
        Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
          std::thread::sleep(Duration::from_millis(5));
        }
        Err(_) => break,
      }

      let mono_needed = window * channels;
      if samples.len() >= mono_needed && last_frame.elapsed() >= frame_period {
        last_frame = Instant::now();
        // Follow the reported pane width: one band per column, capped,
        // then squeezed to the FFT's real frequency resolution.
        let target = columns.load(Ordering::Relaxed).clamp(1, config.bars.max(1));
        if target != columns_now {
          columns_now = target;
          bin_ranges = band_bin_ranges(
            &band_edges(target, hz_per_bin, min_freq, max_freq),
            window,
            config.sample_rate,
          );
          bars = vec![0; bin_ranges.len()];
        }
        let frame: Vec<f32> = samples
          .drain(..mono_needed)
          .collect::<Vec<f32>>()
          .chunks(channels)
          .map(|chunk| chunk.iter().sum::<f32>() / chunk.len() as f32)
          .collect();
        let spectrum = compute_spectrum(&frame, &fft, &hann, &bin_ranges);
        for (index, value) in spectrum.iter().enumerate() {
          let previous = f32::from(bars[index]);
          let smoothed = if *value < previous {
            previous * 0.75 + value * 0.25
          } else {
            *value
          };
          bars[index] = smoothed.clamp(0.0, 100.0) as u8;
        }
        samples.clear();
        if events.send(AsyncEvent::Spectrum(bars.clone())).is_err() {
          return;
        }
      }
    }
  }
}

/// Resolved band colors handed to the render worker.
#[cfg(unix)]
fn open_fifo(path: &str) -> IoResult<std::fs::File> {
  use std::os::fd::AsRawFd;
  use std::os::unix::fs::OpenOptionsExt;
  // mpd only creates the fifo when it loads its config; anything that
  // wipes it afterwards (e.g. a /tmp cleaner) leaves both sides stranded
  // until the fifo exists again. Recreate it so mpd can reconnect on its
  // next output open.
  if !std::path::Path::new(path).exists()
    && let Ok(cpath) = std::ffi::CString::new(path)
  {
    unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) };
    // Ignore mkfifo errors: a racing creator (or a bad path) surfaces
    // as the open error below.
  }
  let file = std::fs::OpenOptions::new()
    .read(true)
    .write(true)
    .custom_flags(libc::O_NONBLOCK)
    .open(path)?;
  // The fifo stream is single-consumer: if a second music-tui instance
  // (or any other reader) opened it first, the kernel would split the
  // samples between both readers and garble every spectrum. An exclusive
  // advisory lock on the fifo makes the loser back off cleanly. The lock
  // lives on the open file description and is released on close/drop.
  let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
  if locked != 0 {
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock {
      tracing::info!("visualizer fifo {path} is held by another instance; backing off");
      return Err(std::io::Error::new(
        std::io::ErrorKind::ResourceBusy,
        format!("fifo {path} is already read by another instance"),
      ));
    }
    return Err(error);
  }
  tracing::info!("visualizer locked fifo {path}");
  Ok(file)
}

#[cfg(unix)]
fn compute_spectrum(
  frame: &[f32],
  fft: &Arc<dyn Fft<f32>>,
  hann: &[f32],
  bin_ranges: &[(usize, usize)],
) -> Vec<f32> {
  let window = frame.len();
  let mut buffer: Vec<Complex<f32>> = frame
    .iter()
    .zip(hann)
    .map(|(sample, gain)| Complex::new(sample * gain, 0.0))
    .collect();
  fft.process(&mut buffer);

  let mut values = Vec::with_capacity(bin_ranges.len());
  for &(start_bin, end_bin) in bin_ranges {
    let mut peak = 0.0f32;
    for sample in &buffer[start_bin..end_bin.min(buffer.len())] {
      let magnitude = sample.norm() * 2.0 / window as f32;
      peak = peak.max(magnitude);
    }
    let db = 20.0 * (peak + 1e-7).log10();
    let normalized = (db + 55.0) / 50.0;
    values.push(normalized.clamp(0.0, 1.0) * 100.0);
  }
  values
}

#[cfg(all(test, unix))]
mod tests {
  use super::*;

  #[test]
  fn edges_keep_bands_on_distinct_bins() {
    // A 256-band log grid over 40..16k Hz at 2048/44.1k (≈21.5 Hz/bin)
    // collapses to the real resolution; every band keeps its own bin.
    let window = 2048;
    let sample_rate = 44_100u32;
    let hz_per_bin = sample_rate as f32 / window as f32;
    let edges = band_edges(256, hz_per_bin, 40.0, 16_000.0);
    let ranges = band_bin_ranges(&edges, window, sample_rate);
    assert!(ranges.len() < 256, "resolution must squeeze the band count");
    for pair in ranges.windows(2) {
      assert!(pair[0].1 <= pair[1].0, "bands must stay disjoint: {pair:?}");
    }
    for &(start, end) in &ranges {
      assert!(end > start, "every band needs at least one bin");
    }
  }

  #[test]
  fn edges_honor_hint_when_resolution_allows() {
    // A small hint (8 bands) with a fine 8192 window stays near the hint
    // (the final log step may overshoot 16 kHz and consume one extra band).
    let window = 8192;
    let sample_rate = 44_100u32;
    let hz_per_bin = sample_rate as f32 / window as f32;
    let edges = band_edges(8, hz_per_bin, 40.0, 16_000.0);
    assert!(
      (9..=11).contains(&edges.len()),
      "hint 8 -> {} edges",
      edges.len()
    );
  }

  #[test]
  fn open_fifo_recreates_a_deleted_fifo() {
    // A /tmp cleaner can delete the fifo under us; the reader must
    // recreate it so mpd can reconnect on its next output open.
    let dir = std::env::temp_dir().join(format!("music-tui-fifo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("feed.fifo");
    let path = path.to_string_lossy().into_owned();

    let first = open_fifo(&path).unwrap();
    assert!(std::path::Path::new(&path).exists());
    drop(first);

    std::fs::remove_file(&path).unwrap();
    let _second = open_fifo(&path).unwrap();
    assert!(
      std::path::Path::new(&path).exists(),
      "deleted fifo must be recreated"
    );

    std::fs::remove_file(&path).unwrap();
    let _ = std::fs::remove_dir(&dir);
  }

  #[test]
  fn open_fifo_is_single_consumer() {
    // Two readers on one fifo would each get half the samples and both
    // spectra would be garbage. The second open must refuse (busy) until
    // the first reader closes.
    let dir = std::env::temp_dir().join(format!("music-tui-fifo-busy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("feed.fifo");
    let path = path.to_string_lossy().into_owned();

    let first = open_fifo(&path).unwrap();
    let second = open_fifo(&path);
    assert_eq!(second.unwrap_err().kind(), std::io::ErrorKind::ResourceBusy);
    drop(first);
    assert!(open_fifo(&path).is_ok(), "lock must release on close");

    std::fs::remove_file(&path).unwrap();
    let _ = std::fs::remove_dir(&dir);
  }
}
