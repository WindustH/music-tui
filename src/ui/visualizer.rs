//! Spectrum visualizer pane rendering.

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

  // One band per column: report the pane width so the FFT analysis matches
  // (capped by `visualizer.bars` in the config).
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

  // Bars normally arrive pre-matched to the pane width (one band per
  // column); the max-resample below only bridges transient frames while
  // the worker catches up with a resize.
  let bars = &app.spectrum;
  let columns = inner.width as usize;
  let values: Vec<u8> = (0..columns)
    .map(|column| {
      let start = column * bars.len() / columns;
      let end = ((column + 1) * bars.len() / columns).max(start + 1);
      bars[start..end.min(bars.len())]
        .iter().copied()
        .max()
        .unwrap_or(0)
    })
    .collect();

  // Full-height vertical bars: '█' for fully filled rows, a partial block
  // at the top edge, empty cells above — ncmpcpp style, bottom-aligned.
  let height = inner.height as usize;
  let fraction_chars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇'];
  let mut lines: Vec<Line> = Vec::with_capacity(height);
  for row in 0..height {
    let from_bottom = height - 1 - row;
    let mut spans = Vec::with_capacity(columns);
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
      spans.push(Span::styled(ch.to_string(), style));
    }
    lines.push(Line::from(spans));
  }
  frame.render_widget(Paragraph::new(lines), inner);
}
