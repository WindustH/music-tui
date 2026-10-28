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

  // Static glyphs: spans borrow them, so building a frame allocates no
  // per-cell strings.
  const FRACTIONS: [&str; 7] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇"];
  let left = " ".repeat(layout.left_margin);
  let right = " ".repeat(layout.right_margin);
  let mut lines: Vec<Line> = Vec::with_capacity(height);
  for row in 0..height {
    let from_bottom = height - 1 - row;
    let mut spans: Vec<Span> = Vec::with_capacity(layout.strips * layout.strip_width + 2);
    if !left.is_empty() {
      spans.push(Span::raw(left.clone()));
    }
    for value in &values {
      let value = (*value).min(100) as usize;
      let full = value * height / 100; // fully filled rows below the tip
      let remainder = value * height % 100; // fraction of the tip row
      let glyph = if from_bottom < full {
        Some("█")
      } else if from_bottom == full && value > 0 && remainder > 0 {
        // Ceil the tip fraction into a glyph bucket (1..=7) so the top
        // glyph is reachable; a plain integer division capped the highest
        // bucket (index 7 -> '▇') off and omitted every fractional tip.
        let index = (remainder * FRACTIONS.len()).div_ceil(100).max(1);
        Some(FRACTIONS[(index - 1).min(FRACTIONS.len() - 1)])
      } else {
        None
      };
      let span = match glyph {
        Some(glyph) => {
          let color = if value < 34 {
            colors.low
          } else if value < 67 {
            colors.mid
          } else {
            colors.high
          };
          Span::styled(glyph, Style::default().fg(color))
        }
        None => Span::raw(" "),
      };
      for _ in 0..layout.strip_width {
        spans.push(span.clone());
      }
    }
    if !right.is_empty() {
      spans.push(Span::raw(right.clone()));
    }
    lines.push(Line::from(spans));
  }
  lines
}
