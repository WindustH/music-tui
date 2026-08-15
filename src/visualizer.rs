//! Spectrum visualizer: reads s16le PCM from the MPD fifo output, runs an
//! FFT per frame, and forwards log-spaced band values (0..=100) to the UI.
//! The band count follows the pane width (one band per column, capped by
//! `bars`); wider panes give every band an equal-width strip, with the
//! remainder spread as gaps so the full width is used.

use std::{
  io::{Read, Result as IoResult},
  sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
  },
  time::{Duration, Instant},
};

use rustfft::{Fft, FftPlanner, num_complex::Complex};
use tokio::sync::mpsc;

use crate::{config::VisualizerConfig, event::AsyncEvent};

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

/// How `bands` analysis bands map onto `width` columns: equal-width strips
/// (one band per column while `width <= bands`), remainder spread evenly
/// as gaps so the whole width is used.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BandLayout {
  /// Number of columns strips rendered.
  pub strips: usize,
  /// Columns per strip (>= 1, identical for every band).
  pub strip_width: usize,
  /// Gap columns after band `i` (`gap_after.len() == strips`, sums to the
  /// remainder).
  pub gap_after: Vec<usize>,
}

pub(crate) fn band_layout(width: usize, bands: usize) -> BandLayout {
  let width = width.max(1);
  let strips = width.min(bands.max(1));
  let strip_width = width / strips;
  let leftover = width % strips;
  let gap_after = (0..strips)
    .map(|band| (band + 1) * leftover / strips - band * leftover / strips)
    .collect();
  BandLayout {
    strips,
    strip_width,
    gap_after,
  }
}

pub fn spawn_visualizer(
  config: VisualizerConfig,
  events: mpsc::UnboundedSender<AsyncEvent>,
) -> VisualizerHandle {
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
  handle
}

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
    .map(|index| {
      0.5 * (1.0 - (std::f32::consts::TAU * index as f32 / window as f32).cos())
    })
    .collect();

  let mut columns_now = config.bars.max(1);
  let mut samples: Vec<f32> = Vec::with_capacity(window * channels * 2);
  let mut bars: Vec<u8> = vec![0; columns_now];
  let mut leftover = Vec::new();
  let frame_period = Duration::from_secs_f64(1.0 / config.fps.max(1) as f64);
  let mut last_frame = Instant::now();
  let mut read_buf = vec![0u8; window * channels * 2 * 2];

  let path = config.fifo_path.clone();
  loop {
    if stop.load(Ordering::SeqCst) {
      return;
    }
    let Ok(mut fifo) = open_fifo(&path) else {
      std::thread::sleep(Duration::from_secs(2));
      continue;
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
        // Follow the reported pane width: one band per column, capped.
        let target = columns.load(Ordering::Relaxed).clamp(1, config.bars.max(1));
        if target != columns_now {
          columns_now = target;
          bars.resize(target, 0);
        }
        let frame: Vec<f32> = samples
          .drain(..mono_needed)
          .collect::<Vec<f32>>()
          .chunks(channels)
          .map(|chunk| chunk.iter().sum::<f32>() / chunk.len() as f32)
          .collect();
        let spectrum = compute_spectrum(&frame, &fft, &hann, columns_now, config.sample_rate);
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

fn open_fifo(path: &str) -> IoResult<std::fs::File> {
  use std::os::unix::fs::OpenOptionsExt;
  let file = std::fs::OpenOptions::new()
    .read(true)
    .write(true)
    .custom_flags(libc::O_NONBLOCK)
    .open(path)?;
  Ok(file)
}

fn compute_spectrum(
  frame: &[f32],
  fft: &Arc<dyn Fft<f32>>,
  hann: &[f32],
  bars: usize,
  sample_rate: u32,
) -> Vec<f32> {
  let window = frame.len();
  let mut buffer: Vec<Complex<f32>> = frame
    .iter()
    .zip(hann)
    .map(|(sample, gain)| Complex::new(sample * gain, 0.0))
    .collect();
  fft.process(&mut buffer);

  let nyquist = sample_rate as f32 / 2.0;
  let bins = window / 2;
  let min_freq = 40.0f32;
  let max_freq = nyquist.min(16_000.0);
  let log_min = min_freq.ln();
  let log_max = max_freq.ln();

  let mut values = Vec::with_capacity(bars);
  for bar in 0..bars {
    let start = if bar == 0 {
      min_freq
    } else {
      (log_min + (log_max - log_min) * bar as f32 / bars as f32).exp()
    };
    let end = (log_min + (log_max - log_min) * (bar + 1) as f32 / bars as f32).exp();
    let start_bin = ((start / nyquist) * bins as f32).floor().max(1.0) as usize;
    let end_bin = (((end / nyquist) * bins as f32).ceil() as usize).clamp(start_bin + 1, bins);
    let mut peak = 0.0f32;
    for sample in &buffer[start_bin..end_bin] {
      let magnitude = sample.norm() * 2.0 / window as f32;
      peak = peak.max(magnitude);
    }
    let db = 20.0 * (peak + 1e-7).log10();
    let normalized = (db + 55.0) / 50.0;
    values.push(normalized.clamp(0.0, 1.0) * 100.0);
  }
  values
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn one_band_per_column_within_cap() {
    let layout = band_layout(80, 256);
    assert_eq!(layout.strips, 80);
    assert_eq!(layout.strip_width, 1);
    assert!(layout.gap_after.iter().all(|gap| *gap == 0));
    assert_eq!(layout.strips * layout.strip_width, 80);
  }

  #[test]
  fn equal_strips_spread_remainder_as_gaps() {
    // 300 columns, 256 bands: strips of 1, 44 leftover gaps spread evenly.
    let layout = band_layout(300, 256);
    assert_eq!(layout.strips, 256);
    assert_eq!(layout.strip_width, 1);
    assert_eq!(layout.gap_after.iter().sum::<usize>(), 44);
    assert_eq!(layout.strips * layout.strip_width + layout.gap_after.iter().sum::<usize>(), 300);
  }

  #[test]
  fn equal_multi_column_strips() {
    // 512 columns, 256 bands: every band 2 columns, nothing left over.
    let layout = band_layout(512, 256);
    assert_eq!(layout.strips, 256);
    assert_eq!(layout.strip_width, 2);
    assert!(layout.gap_after.iter().all(|gap| *gap == 0));
    assert_eq!(layout.strips * layout.strip_width, 512);
  }

  #[test]
  fn narrow_pane_clamps_to_width() {
    let layout = band_layout(4, 256);
    assert_eq!(layout.strips, 4);
    assert_eq!(layout.strip_width, 1);
  }
}
