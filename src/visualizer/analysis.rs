//! Spectrum analysis: s16le decoding and downmix, the FFT per frame, and
//! the log-spaced band layout (band edges and their FFT bin ranges).

use std::{collections::VecDeque, sync::Arc};

use rustfft::{Fft, FftPlanner, num_complex::Complex};

use crate::config::VisualizerConfig;

/// Band edges in Hz, log-spaced with a minimum linear step of one FFT bin:
/// every band owns at least one distinct bin, so neighboring low-frequency
/// bands (where a pure log grid is narrower than the FFT resolution) never
/// sample the same bin and render as duplicated identical bars.
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
      let start_bin = ((pair[0] / nyquist) * bins as f32)
        .floor()
        .max(1.0)
        .min(bins as f32) as usize;
      // Total even when the Nyquist limit collapses to min_freq: a plain
      // `.clamp(start_bin + 1, bins)` panics once start_bin hits `bins`.
      let end_bin = (((pair[1] / nyquist) * bins as f32).ceil() as usize)
        .max(start_bin + 1)
        .min(bins);
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

/// Rolling s16le decoder plus FFT analysis state. Buffers are allocated
/// once; a frame is only computed when new samples arrived since the last.
pub(super) struct Analyzer {
  pub(super) window: usize,
  pub(super) channels: usize,
  sample_rate: u32,
  bars_cap: usize,
  hz_per_bin: f32,
  min_freq: f32,
  max_freq: f32,
  fft: Arc<dyn Fft<f32>>,
  hann: Vec<f32>,
  window_sum: f32,
  /// Band count the bin ranges were built for.
  columns: usize,
  bin_ranges: Vec<(usize, usize)>,
  bars: Vec<u8>,
  /// Downmixed samples, newest last (bounded to `max_keep`).
  mono: VecDeque<f32>,
  max_keep: usize,
  /// Samples appended since the last computed frame.
  fresh: usize,
  /// Low byte of a sample split across two reads.
  pending_byte: Option<u8>,
  /// Channels of the current (incomplete) sample frame.
  channel_sum: f32,
  channel_count: usize,
  spectrum: Vec<Complex<f32>>,
  scratch: Vec<Complex<f32>>,
  levels: Vec<f32>,
}

impl Analyzer {
  pub(super) fn new(config: &VisualizerConfig) -> Self {
    let window = config.window.max(256);
    let channels = usize::from(config.channels.max(1));
    // A zero `sample_rate` (an explicit-but-invalid config: the schema
    // default is 44100) would make nyquist/hz-per-bin drop to zero below
    // and panic the thread. Clamp to 80 (= 2 * min_freq) so the Nyquist
    // limit never falls below the lowest band edge and band-to-bin mapping
    // stays valid; the schema re-clamps as well (see normalize_defaults).
    let sample_rate = config.sample_rate.max(80);
    let fft = FftPlanner::new().plan_fft_forward(window);
    let hann = hann_window(window);
    let window_sum = hann.iter().sum();
    let hz_per_bin = sample_rate as f32 / window as f32;
    let min_freq = 40.0f32;
    let max_freq = (sample_rate as f32 / 2.0).min(16_000.0);
    let columns = config.bars.max(1);
    let bin_ranges = band_bin_ranges(
      &band_edges(columns, hz_per_bin, min_freq, max_freq),
      window,
      sample_rate,
    );
    // Pace the analysis by time with overlapping windows (the newest
    // `window` samples each frame); keep a couple of strides of slack.
    let stride = (sample_rate / config.fps.max(1)).max(1) as usize;
    let scratch_len = fft.get_inplace_scratch_len();
    Self {
      window,
      channels,
      sample_rate,
      bars_cap: config.bars.max(1),
      hz_per_bin,
      min_freq,
      max_freq,
      fft,
      hann,
      window_sum,
      columns,
      bars: vec![0; bin_ranges.len()],
      bin_ranges,
      mono: VecDeque::with_capacity(window + stride * 2),
      max_keep: window + stride * 2,
      fresh: 0,
      pending_byte: None,
      channel_sum: 0.0,
      channel_count: 0,
      spectrum: vec![Complex::new(0.0, 0.0); window],
      scratch: vec![Complex::new(0.0, 0.0); scratch_len],
      levels: Vec::new(),
    }
  }

  /// Forget buffered audio and any partial sample (reopen, pane hidden).
  pub(super) fn reset(&mut self) {
    self.mono.clear();
    self.fresh = 0;
    self.pending_byte = None;
    self.channel_sum = 0.0;
    self.channel_count = 0;
  }

  /// Decode s16le bytes and downmix to mono. Partial samples and partial
  /// channel frames carry over to the next read.
  pub(super) fn push_bytes(&mut self, mut bytes: &[u8]) {
    if let Some(low) = self.pending_byte.take() {
      let Some((&high, rest)) = bytes.split_first() else {
        self.pending_byte = Some(low);
        return;
      };
      self.push_sample(i16::from_le_bytes([low, high]));
      bytes = rest;
    }
    let (pairs, remainder) = bytes.as_chunks::<2>();
    for pair in pairs {
      self.push_sample(i16::from_le_bytes(*pair));
    }
    self.pending_byte = remainder.first().copied();
    while self.mono.len() > self.max_keep {
      self.mono.pop_front();
    }
  }

  fn push_sample(&mut self, sample: i16) {
    self.channel_sum += f32::from(sample) / 32768.0;
    self.channel_count += 1;
    if self.channel_count == self.channels {
      self.mono.push_back(self.channel_sum / self.channels as f32);
      self.channel_sum = 0.0;
      self.channel_count = 0;
      self.fresh += 1;
    }
  }

  /// Analyze the newest window into smoothed band levels, or `None` when
  /// no new audio arrived since the last frame (paused / stopped).
  pub(super) fn frame(&mut self, columns: usize) -> Option<Vec<u8>> {
    if self.fresh == 0 || self.mono.len() < self.window {
      return None;
    }
    self.fresh = 0;
    // Follow the reported pane width: one band per column, capped, then
    // squeezed to the FFT's real frequency resolution.
    let target = columns.clamp(1, self.bars_cap);
    if target != self.columns {
      self.columns = target;
      self.bin_ranges = band_bin_ranges(
        &band_edges(target, self.hz_per_bin, self.min_freq, self.max_freq),
        self.window,
        self.sample_rate,
      );
      self.bars = vec![0; self.bin_ranges.len()];
    }
    let start = self.mono.len() - self.window;
    for (slot, (sample, gain)) in self
      .spectrum
      .iter_mut()
      .zip(self.mono.range(start..).zip(&self.hann))
    {
      *slot = Complex::new(sample * gain, 0.0);
    }
    self
      .fft
      .process_with_scratch(&mut self.spectrum, &mut self.scratch);
    band_levels(
      &self.spectrum,
      self.window_sum,
      &self.bin_ranges,
      &mut self.levels,
    );
    for (bar, level) in self.bars.iter_mut().zip(&self.levels) {
      let previous = f32::from(*bar);
      let smoothed = if *level < previous {
        previous * 0.75 + level * 0.25
      } else {
        *level
      };
      *bar = smoothed.clamp(0.0, 100.0) as u8;
    }
    Some(self.bars.clone())
  }
}

fn hann_window(window: usize) -> Vec<f32> {
  (0..window)
    .map(|index| 0.5 * (1.0 - (std::f32::consts::TAU * index as f32 / window as f32).cos()))
    .collect()
}

/// Band levels (0..=100) of an FFT'd window. The window's coherent gain
/// (its sum, ~N/2 for Hann) leaves an exact-bin full-scale sine at N/4;
/// normalizing by the window sum while keeping the analytic-signal factor
/// of 2 maps that back to 0 dB so bar heights track the signal level.
fn band_levels(
  spectrum: &[Complex<f32>],
  window_sum: f32,
  bin_ranges: &[(usize, usize)],
  levels: &mut Vec<f32>,
) {
  levels.clear();
  for &(start_bin, end_bin) in bin_ranges {
    let peak = spectrum[start_bin.min(spectrum.len())..end_bin.min(spectrum.len())]
      .iter()
      .map(|bin| bin.norm() * 2.0 / window_sum)
      .fold(0.0f32, f32::max);
    let db = 20.0 * (peak + 1e-7).log10();
    // Display map: floor at -60 dB, full scale left under the 100 cap so
    // loud passages don't just pin at the clamp (more visible range).
    let normalized = ((db + 60.0) / 60.0 * 0.9).clamp(0.0, 1.0);
    levels.push(normalized * 100.0);
  }
}

/// One-shot analysis of `frame` (tests).
#[cfg(test)]
fn compute_spectrum(
  frame: &[f32],
  fft: &Arc<dyn Fft<f32>>,
  hann: &[f32],
  bin_ranges: &[(usize, usize)],
) -> Vec<f32> {
  let mut buffer: Vec<Complex<f32>> = frame
    .iter()
    .zip(hann)
    .map(|(sample, gain)| Complex::new(sample * gain, 0.0))
    .collect();
  fft.process(&mut buffer);
  let mut levels = Vec::new();
  band_levels(&buffer, hann.iter().sum(), bin_ranges, &mut levels);
  levels
}

#[cfg(test)]
mod tests {
  use super::*;

  fn test_config(window: usize) -> VisualizerConfig {
    VisualizerConfig {
      window,
      channels: 2,
      sample_rate: 44_100,
      bars: 16,
      fps: 30,
      ..VisualizerConfig::default()
    }
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
    assert!(
      (9..=11).contains(&edges.len()),
      "hint 8 -> {} edges",
      edges.len()
    );
  }

  #[test]
  fn band_ranges_stay_valid_when_nyquist_below_min_freq() {
    // A sample_rate below 2 * min_freq (40 Hz) puts the Nyquist limit under
    // the lowest band edge; the mapping must keep every range start <= end
    // (it used to invert and panic the spectrum slice).
    let window = 512;
    let ranges = band_bin_ranges(&[40.0, 20.0], window, 40);
    assert!(!ranges.is_empty());
    for &(start, end) in &ranges {
      assert!(start <= end, "band range must not invert: ({start}, {end})");
    }
  }

  #[test]
  fn band_ranges_stay_valid_at_minimum_sample_rate() {
    // run() clamps sample_rate to 80 (= 2 * min_freq), where the final
    // edge equals Nyquist: the degenerate [min_freq, max_freq] pair must
    // not panic the range clamp.
    let window = 256;
    let ranges = band_bin_ranges(&[40.0, 40.0], window, 80);
    assert!(!ranges.is_empty());
    for &(start, end) in &ranges {
      assert!(start <= end, "band range must not invert: ({start}, {end})");
    }
  }

  #[test]
  fn spectrum_scaling_tracks_signal_level() {
    let window = 2048;
    let sample_rate = 16_384u32;
    let fft = FftPlanner::new().plan_fft_forward(window);
    let hann = hann_window(window);
    // Exact-bin sines (1000 Hz at N=2048 over 16.384 kHz -> bin 125): with
    // the window-sum normalization, full scale reads ~90 (0 dB under the
    // display cap) and -20 dB reads ~60, so the bars span the display
    // instead of hugging the top.
    let sine = |amplitude: f32| -> Vec<f32> {
      (0..window)
        .map(|index| {
          amplitude * (std::f32::consts::TAU * index as f32 * 1000.0 / sample_rate as f32).sin()
        })
        .collect()
    };
    let ranges = vec![(125usize, 126usize)];
    let full = compute_spectrum(&sine(1.0), &fft, &hann, &ranges)[0];
    let quiet = compute_spectrum(&sine(0.1), &fft, &hann, &ranges)[0];
    assert!(
      (86.0..=92.0).contains(&full),
      "full-scale sine reads {full}"
    );
    assert!((55.0..=65.0).contains(&quiet), "-20 dB sine reads {quiet}");
    assert!(
      full - quiet > 20.0,
      "dynamic range compressed: {full} vs {quiet}"
    );
  }

  #[test]
  fn decoder_handles_split_samples_and_channel_frames() {
    let mut analyzer = Analyzer::new(&test_config(256));
    // Two stereo frames (L=1000,R=3000), (L=-1000,R=-3000) fed one byte
    // at a time: every split point must decode identically.
    let bytes: Vec<u8> = [1000i16, 3000, -1000, -3000]
      .iter()
      .flat_map(|sample| sample.to_le_bytes())
      .collect();
    for byte in &bytes {
      analyzer.push_bytes(std::slice::from_ref(byte));
    }
    let mono: Vec<f32> = analyzer.mono.iter().copied().collect();
    assert_eq!(mono.len(), 2);
    assert!((mono[0] - 2000.0 / 32768.0).abs() < 1e-6);
    assert!((mono[1] + 2000.0 / 32768.0).abs() < 1e-6);
    assert_eq!(analyzer.fresh, 2);
  }

  #[test]
  fn frames_need_fresh_samples() {
    let window = 256;
    let mut analyzer = Analyzer::new(&test_config(window));
    assert!(analyzer.frame(16).is_none(), "no audio yet");
    let bytes: Vec<u8> = (0..window * 2)
      .flat_map(|index| ((index as i16).wrapping_mul(97)).to_le_bytes())
      .collect();
    analyzer.push_bytes(&bytes);
    assert!(analyzer.frame(16).is_some());
    // Paused playback: nothing new arrives, so nothing is recomputed.
    assert!(analyzer.frame(16).is_none());
    analyzer.push_bytes(&bytes[..4]);
    assert!(analyzer.frame(16).is_some());
    // The rolling buffer stays bounded.
    for _ in 0..50 {
      analyzer.push_bytes(&bytes);
    }
    assert!(analyzer.mono.len() <= analyzer.max_keep);
  }
}
