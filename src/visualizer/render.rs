//! Off-thread band rendering: band layout math (equal-width strips,
//! centered margins) and styled line construction for the visualizer
//! pane. The UI thread only blits the precomputed lines.

use std::sync::mpsc as std_mpsc;

use ratatui::{
  style::{Color, Style},
  text::{Line, Span},
};

use tokio::sync::mpsc;

use crate::event::AsyncEvent;

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
    let lines = build_band_lines(10, 5, &[100u8; 10], &colors);
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
}
