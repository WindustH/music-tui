//! Secondary detail view rendering for a queue entry.

use super::*;

/// Secondary detail surface for a queue entry (`i`): cover on top,
/// metadata below — the sidebar data stays untouched.
pub(super) fn draw_detail_view(
  frame: &mut Frame,
  app: &App,
  detail: &crate::app::DetailView,
  renderer: &mut CoverRenderStore,
  tx: &mpsc::UnboundedSender<AsyncEvent>,
  area: Rect,
  overlays: &mut Vec<ProtocolOverlay>,
) {
  let theme = &app.settings.theme;
  let block = Block::default()
    .borders(Borders::ALL)
    .border_style(Style::default().fg(theme.color(&theme.accent)))
    .title(format!(" detail: {} ", detail.title))
    .title_alignment(Alignment::Center);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.width < 2 || inner.height < 2 {
    return;
  }
  let [cover_area, metadata_area] =
    Layout::vertical([Constraint::Ratio(2, 3), Constraint::Ratio(1, 3)]).areas(inner);

  // --- cover (same fitting math as the cover pane) ---
  let (cell_width, cell_height) = renderer.cell_pixels();
  let image_area = match detail.cover_dims {
    Some((image_width, image_height)) if image_width > 0 && image_height > 0 => {
      let max_pixel_width = f64::from(cover_area.width) * f64::from(cell_width.max(1));
      let max_pixel_height = f64::from(cover_area.height) * f64::from(cell_height.max(1));
      let scale = (max_pixel_width / f64::from(image_width))
        .min(max_pixel_height / f64::from(image_height))
        .max(0.0);
      let fitted_width = ((f64::from(image_width) * scale) / f64::from(cell_width.max(1)))
        .round()
        .clamp(1.0, f64::from(cover_area.width)) as u16;
      let fitted_height = ((f64::from(image_height) * scale) / f64::from(cell_height.max(1)))
        .round()
        .clamp(1.0, f64::from(cover_area.height)) as u16;
      Rect {
        x: cover_area.x + cover_area.width.saturating_sub(fitted_width) / 2,
        y: cover_area.y + cover_area.height.saturating_sub(fitted_height) / 2,
        width: fitted_width,
        height: fitted_height,
      }
    }
    _ => cover_area,
  };
  match detail.cover.as_ref() {
    Some(path) => {
      renderer.request(path, image_area.width, image_area.height, tx);
      match renderer.get(path, image_area.width, image_area.height) {
        Some(RenderedImage::Symbols { text, .. }) => {
          frame.render_widget(
            Paragraph::new(text.clone()).wrap(Wrap { trim: false }),
            image_area,
          );
        }
        Some(RenderedImage::Protocol {
          mode,
          data,
          refresh,
          placement,
          fingerprint,
          erase,
        }) => {
          reserve_protocol_area(frame, image_area);
          overlays.push(ProtocolOverlay {
            area: image_area,
            mode: *mode,
            data: data.clone(),
            refresh: refresh.clone(),
            placement: placement.clone(),
            fingerprint: *fingerprint,
            erase: erase.clone(),
          });
        }
        None => {
          frame.render_widget(
            Paragraph::new("rendering cover…")
              .style(Style::default().fg(theme.color(&theme.muted)))
              .alignment(Alignment::Center),
            cover_area,
          );
        }
      }
    }
    None => {
      let hint = detail
        .cover_error
        .clone()
        .unwrap_or_else(|| "no cover".to_string());
      frame.render_widget(
        Paragraph::new(hint)
          .style(Style::default().fg(theme.color(&theme.muted)))
          .alignment(Alignment::Center),
        cover_area,
      );
    }
  }

  // --- metadata ---
  let metadata_block = Block::default()
    .borders(Borders::ALL)
    .border_style(Style::default().fg(theme.color(&theme.border)))
    .title(" metadata (e edit · i close) ");
  let metadata_inner = metadata_block.inner(metadata_area);
  frame.render_widget(metadata_block, metadata_area);
  if metadata_inner.height == 0 {
    return;
  }
  match detail.metadata.as_ref() {
    Some(entries) => {
      let lines: Vec<Line> = entries
        .iter()
        .skip(detail.metadata_scroll)
        .map(|entry| metadata_line(app, &entry.name, &entry.value))
        .collect();
      frame.render_widget(Paragraph::new(lines), metadata_inner);
    }
    None => {
      let hint = detail
        .metadata_error
        .clone()
        .unwrap_or_else(|| "reading metadata…".to_string());
      frame.render_widget(
        Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.muted))),
        metadata_inner,
      );
    }
  }
}
