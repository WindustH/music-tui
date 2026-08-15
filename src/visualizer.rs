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
    mpsc as std_mpsc,
  },
  time::{Duration, Instant},
};

use ratatui::{
  style::{Color, Style},
  text::{Line, Span},
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

/// How `bands` analysis bands map onto `width` columns: every band gets an
/// equal-width strip. The strip count is chosen to minimize
/// `(leftover + slack) / strips` where `slack` grows with the available
/// width (its 1/8, at least 1) — the slack keeps a zero-leftover split
/// from dominating (it would otherwise beat every denser split and shrink
/// the band count) while the proportional term tracks the pane size. The
/// leftover is split onto the left/right margins so the visualization is
/// centered.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BandLayout {
  /// Number of band strips rendered.
  pub strips: usize,
  /// Columns per strip (>= 1, identical for every band).
  pub strip_width: usize,
  /// Empty columns on the left / right side.
  pub left_margin: usize,
  pub right_margin: usize,
}

pub(crate) fn band_layout(width: usize, bands: usize) -> BandLayout {
  let width = width.max(1);
  let max_strips = width.min(bands.max(1));
  // Exact search: minimize (leftover + slack)/strips with a slack that
  // grows with the width (its 1/8, at least 1); prefer more strips on
  // ties (e.g. an exact divisor of the width wins with the smallest
  // leftover term).
  let slack = (width as f32 / 8.0).max(1.0);
  let mut best = (f32::INFINITY, 1usize);
  for strips in 1..=max_strips {
    // A single strip trivially zeroes the ratio; skip it unless it is the
    // only option (degenerate one-band pane).
    if strips == 1 && max_strips > 1 {
      continue;
    }
    let leftover = width % strips;
    let ratio = (leftover as f32 + slack) / strips as f32;
    if ratio < best.0 || (ratio == best.0 && strips > best.1) {
      best = (ratio, strips);
    }
  }
  let strips = best.1;
  let strip_width = width / strips;
  let leftover = width % strips;
  let left_margin = leftover / 2;
  BandLayout {
    strips,
    strip_width,
    left_margin,
    right_margin: leftover - left_margin,
  }
}

/// Band edges in Hz, log-spaced with a minimum linear step of one FFT bin:
/// every band owns at least one distinct bin, so neighboring low-frequency
/// bands (where a pure log grid is narrower than the FFT resolution) never
/// sample the same bin and render as duplicated identical bars.
fn band_edges(hint: usize, hz_per_bin: f32, min_freq: f32, max_freq: f32) -> Vec<f32> {
  let hint = hint.max(2);
  let mut edges =
    build_band_edges((max_freq / min_freq).powf(1.0 / hint as f32), hz_per_bin, min_freq, max_freq);
  // Refine the ratio so the generated band count matches its own hint.
  for _ in 0..8 {
    let count = edges.len() - 1;
    if count < 2 {
      break;
    }
    let next_edges =
      build_band_edges((max_freq / min_freq).powf(1.0 / count as f32), hz_per_bin, min_freq, max_freq);
    if next_edges.len() == edges.len() {
      break;
    }
    edges = next_edges;
  }
  edges
}

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
#[derive(Clone, Copy)]
pub(crate) struct VisualizerColors {
  pub low: Color,
  pub mid: Color,
  pub high: Color,
}

struct BandRenderRequest {
  width: u16,
  height: u16,
  bars: Vec<u8>,
  colors: VisualizerColors,
}

/// Sender half of the off-thread band renderer: layout + styled-line
/// construction for the visualizer pane happens on a worker thread so the
/// UI thread only blits precomputed lines.
pub struct BandRendererHandle {
  tx: std_mpsc::Sender<BandRenderRequest>,
}

impl BandRendererHandle {
  pub fn render(&self, width: u16, height: u16, bars: Vec<u8>, colors: VisualizerColors) {
    let _ = self.tx.send(BandRenderRequest {
      width,
      height,
      bars,
      colors,
    });
  }
}

/// Spawn the band-render worker. It coalesces pending requests (only the
/// latest is rendered) and answers with [`AsyncEvent::VisualizerFrame`].
pub fn spawn_band_renderer(events: mpsc::UnboundedSender<AsyncEvent>) -> BandRendererHandle {
  let (tx, rx) = std_mpsc::channel::<BandRenderRequest>();
  std::thread::Builder::new()
    .name("music-tui-visualizer-render".to_string())
    .spawn(move || {
      while let Ok(mut request) = rx.recv() {
        while let Ok(next) = rx.try_recv() {
          request = next;
        }
        let lines = build_band_lines(
          request.width as usize,
          request.height as usize,
          &request.bars,
          &request.colors,
        );
        if events.send(AsyncEvent::VisualizerFrame(lines)).is_err() {
          return;
        }
      }
    })
    .expect("failed to spawn visualizer render thread");
  BandRendererHandle { tx }
}

/// Build the pane content: equal-width band strips laid out by
/// [`band_layout`], rendered as full-height vertical bars with a partial
/// block at the tip — ncmpcpp style, bottom-aligned.
pub(crate) fn build_band_lines(
  width: usize,
  height: usize,
  bars: &[u8],
  colors: &VisualizerColors,
) -> Vec<Line<'static>> {
  if width == 0 || height == 0 || bars.is_empty() {
    return Vec::new();
  }
  let layout = band_layout(width, bars.len());
  let values: Vec<u8> = (0..layout.strips)
    .map(|strip| {
      let start = strip * bars.len() / layout.strips;
      let end = ((strip + 1) * bars.len() / layout.strips).max(start + 1);
      bars[start..end.min(bars.len())]
        .iter()
        .copied()
        .max()
        .unwrap_or(0)
    })
    .collect();

  let fraction_chars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇'];
  let left = " ".repeat(layout.left_margin);
  let right = " ".repeat(layout.right_margin);
  let mut lines: Vec<Line> = Vec::with_capacity(height);
  for row in 0..height {
    let from_bottom = height - 1 - row;
    let mut spans: Vec<Span> = Vec::with_capacity(width);
    if !left.is_empty() {
      spans.push(Span::raw(left.clone()));
    }
    for value in &values {
      let value = (*value).min(100) as usize;
      let full = value * height / 100; // fully filled rows below the tip
      let remainder = value * height % 100; // fraction of the tip row
      let (ch, lit) = if from_bottom < full {
        ('█', true)
      } else if from_bottom == full && value > 0 {
        let index = (remainder * fraction_chars.len() / 100).max(1);
        (
          fraction_chars[(index - 1).min(fraction_chars.len() - 1)],
          true,
        )
      } else {
        (' ', false)
      };
      let color = if value < 34 {
        colors.low
      } else if value < 67 {
        colors.mid
      } else {
        colors.high
      };
      let style = if lit {
        Style::default().fg(color)
      } else {
        Style::default()
      };
      for _ in 0..layout.strip_width {
        spans.push(Span::styled(ch.to_string(), style));
      }
    }
    if !right.is_empty() {
      spans.push(Span::raw(right.clone()));
    }
    lines.push(Line::from(spans));
  }
  lines
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

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn one_band_per_column_within_cap() {
    let layout = band_layout(80, 256);
    assert_eq!(layout.strips, 80);
    assert_eq!(layout.strip_width, 1);
    assert_eq!(layout.left_margin + layout.right_margin, 0);
    assert_eq!(layout.strips * layout.strip_width, 80);
  }

  #[test]
  fn exact_divisor_minimizes_leftover() {
    // 300 columns, 256 bands: 150 strips of 2 keep the bars wide with no
    // leftover; the slack still prefers them over 256 cramped strips.
    let layout = band_layout(300, 256);
    assert_eq!(layout.strips, 150);
    assert_eq!(layout.strip_width, 2);
    assert_eq!(layout.left_margin + layout.right_margin, 0);
    assert_eq!(layout.strips * layout.strip_width, 300);
  }

  #[test]
  fn proportional_slack_prefers_denser_bands() {
    // 262 = 2 x 131 with zero leftover, but the width-proportional slack
    // makes the denser 256 single-column bands (6 leftover as margins)
    // score better: (0 + 32.75)/131 vs (6 + 32.75)/256.
    let layout = band_layout(262, 256);
    assert_eq!(layout.strips, 256);
    assert_eq!(layout.strip_width, 1);
    assert_eq!(layout.left_margin, 3);
    assert_eq!(layout.right_margin, 3);
  }

  #[test]
  fn leftover_centers_with_margins() {
    // 263 is prime: the slack favors the densest split — 256 strips of 1
    // with the 7 leftover columns centered as margins.
    let layout = band_layout(263, 256);
    assert_eq!(layout.strips, 256);
    assert_eq!(layout.strip_width, 1);
    assert_eq!(layout.left_margin, 3);
    assert_eq!(layout.right_margin, 4);
    assert_eq!(
      layout.strips * layout.strip_width + layout.left_margin + layout.right_margin,
      263
    );
  }

  #[test]
  fn odd_leftover_splits_around_center() {
    // Width 5 with 2 bands available: 2 strips of 2 leave 1 column,
    // placed on the right (left gets the floor of the split).
    let layout = band_layout(5, 2);
    assert_eq!(layout.strips, 2);
    assert_eq!(layout.strip_width, 2);
    assert_eq!(layout.left_margin, 0);
    assert_eq!(layout.right_margin, 1);
  }

  #[test]
  fn narrow_pane_clamps_to_width() {
    let layout = band_layout(4, 256);
    assert_eq!(layout.strips, 4);
    assert_eq!(layout.strip_width, 1);
  }

  #[test]
  fn zero_leftover_does_not_override_band_count() {
    // 135 = 45 x 3: a fixed +1 or zero-slack score would lock onto the
    // zero-leftover 45 strips and drop the band count; the proportional
    // slack lets the denser 134 single-column bands win (one margin
    // column).
    let layout = band_layout(135, 134);
    assert_eq!(layout.strips, 134);
    assert_eq!(layout.strip_width, 1);
    assert_eq!(layout.left_margin, 0);
    assert_eq!(layout.right_margin, 1);
  }

  #[test]
  fn band_lines_fill_exact_width() {
    let colors = VisualizerColors {
      low: Color::Green,
      mid: Color::Yellow,
      high: Color::Red,
    };
    let lines = build_band_lines(10, 5, &vec![100u8; 10], &colors);
    assert_eq!(lines.len(), 5);
    let width: usize = lines[0]
      .spans
      .iter()
      .map(|span| span.content.chars().count())
      .sum();
    assert_eq!(width, 10);
    // Fully lit band: every column of the bottom row is a block.
    assert!(lines[4].spans.iter().all(|span| span.content.contains('█')));
  }

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
    assert!((9..=11).contains(&edges.len()), "hint 8 -> {} edges", edges.len());
  }
}
