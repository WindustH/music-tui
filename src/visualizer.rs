//! Spectrum visualizer: reads s16le PCM from the MPD fifo output, runs an
//! FFT per frame, and forwards log-spaced bar values (0..=100) to the UI.

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
  /// Desired bar count, driven by the pane width reported by the UI
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

  /// Report the current pane width so each column gets its own band.
  pub fn set_columns(&self, columns: usize) {
    self.columns.store(columns.max(1), Ordering::Relaxed);
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

  let mut samples: Vec<f32> = Vec::with_capacity(window * channels * 2);
  let mut bars: Vec<u8> = vec![0; columns.load(Ordering::Relaxed).max(1)];
  let max_bars = config.bars.max(1);
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
        let target = columns.load(Ordering::Relaxed).clamp(1, max_bars);
        if bars.len() != target {
          bars.resize(target, 0);
        }
        let frame: Vec<f32> = samples
          .drain(..mono_needed)
          .collect::<Vec<f32>>()
          .chunks(channels)
          .map(|chunk| chunk.iter().sum::<f32>() / chunk.len() as f32)
          .collect();
        let spectrum = compute_spectrum(&frame, &fft, &hann, bars.len(), config.sample_rate);
        for (index, value) in spectrum.iter().enumerate() {
          let index = index.min(bars.len() - 1);
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
