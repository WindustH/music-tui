//! Spectrum visualizer pane rendering: the analysis band count follows the
//! pane width (one band per column, capped by `visualizer.bars`); wider
//! panes give every band an equal-width strip, with the remainder spread
//! as gaps so the full width is used.

use super::*;

pub(super) fn draw_visualizer_pane(frame: &mut Frame, app: &mut App, area: Rect) {
  let theme = &app.settings.theme;
  let is_main = app.main_pane() == PaneKind::Visualizer;
  let block = pane_block(app, "visualizer", is_main);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.height == 0 || inner.width == 0 {
    return;
  }

  // Keep the analysis band count in sync with the pane width.
  if let Some(visualizer) = app.visualizer.as_ref() {
    visualizer.set_columns(inner.width as usize);
  }

  if app.spectrum.is_empty() {
    frame.render_widget(
      Paragraph::new("waiting for audio on the mpd fifo…")
        .style(Style::default().fg(theme.color(&theme.muted))),
      inner,
    );
    return;
  }

  // Equal-width strips: while the pane is narrower than the band count
  // every column is its own band; wider panes group neighboring bands so
  // each strip keeps the same width, and the remainder becomes evenly
  // spread gaps (the full width is always used).
  let bars = &app.spectrum;
  let height = inner.height as usize;
  let layout = crate::visualizer::band_layout(inner.width as usize, bars.len().max(1));

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

  // Full-height vertical bars: '█' for fully filled rows, a partial block
  // at the top edge, empty cells above — ncmpcpp style, bottom-aligned.
  let fraction_chars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇'];
  let left = " ".repeat(layout.left_margin);
  let right = " ".repeat(layout.right_margin);
  let mut lines: Vec<Line> = Vec::with_capacity(height);
  for row in 0..height {
    let from_bottom = height - 1 - row;
    let mut spans: Vec<Span> = Vec::with_capacity(inner.width as usize);
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
        theme.color(&theme.visualizer_low)
      } else if value < 67 {
        theme.color(&theme.visualizer_mid)
      } else {
        theme.color(&theme.visualizer_high)
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
  frame.render_widget(Paragraph::new(lines), inner);
}
