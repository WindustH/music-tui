//! Spectrum visualizer pane rendering: the heavy work (band layout and
//! styled-line construction) happens on the band-render worker thread; the
//! UI thread only records the pane geometry and blits the precomputed
//! lines cell by cell (no per-frame allocation).

use super::*;

pub(super) fn draw_visualizer_pane(frame: &mut Frame, app: &mut App, area: Rect) {
  let is_main = app.main_pane() == PaneKind::Visualizer;
  let block = pane_block(app, "visualizer", is_main);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.height == 0 || inner.width == 0 {
    return;
  }

  // Keep the analysis band count in sync with the pane width, and let the
  // app re-render the band lines off-thread when the geometry changes.
  if let Some(visualizer) = app.visualizer.as_ref() {
    visualizer.set_columns(inner.width as usize);
  }
  app.note_visualizer_geometry(inner.width, inner.height);

  let Some(lines) = app.visualizer_lines.as_ref() else {
    let theme = &app.settings.theme;
    frame.render_widget(
      Paragraph::new("waiting for audio on the mpd fifo…")
        .style(Style::default().fg(theme.color(&theme.base.muted))),
      inner,
    );
    return;
  };

  blit_lines(frame, inner, lines);
}

/// Write precomputed lines straight into the frame buffer. Lines are built
/// for the exact pane width (block elements and spaces are single-cell), so
/// no wrapping, measuring or cloning is needed.
fn blit_lines(frame: &mut Frame, area: Rect, lines: &[Line]) {
  let buffer = frame.buffer_mut();
  let right = area.x + area.width;
  for (row, line) in lines.iter().enumerate().take(area.height as usize) {
    let mut x = area.x;
    'spans: for span in &line.spans {
      for ch in span.content.chars() {
        if x >= right {
          break 'spans;
        }
        if let Some(cell) = buffer.cell_mut((x, area.y + row as u16)) {
          cell.set_char(ch).set_style(span.style);
        }
        x += 1;
      }
    }
  }
}
