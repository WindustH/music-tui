//! Cover pane rendering (protocol images and Chafa symbols).

use super::*;

pub(super) fn draw_cover_pane(
  frame: &mut Frame,
  app: &mut App,
  renderer: &mut CoverRenderStore,
  tx: &mpsc::UnboundedSender<AsyncEvent>,
  area: Rect,
  source: PaneSource,
  overlays: &mut Vec<ProtocolOverlay>,
) {
  if matches!(source, PaneSource::QueueHovered | PaneSource::LibraryHovered) {
    draw_hover_cover_pane(frame, app, renderer, tx, area, overlays, source);
    return;
  }
  let theme = &app.settings.theme;
  let is_main = app.main_pane() == PaneKind::Cover;
  let block = pane_block(app, "cover", is_main);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.width < 2 || inner.height < 2 {
    return;
  }

  // Center the artwork: fit the intrinsic aspect ratio inside the pane,
  // converting through cell pixels (same math as gallery-tui's
  // `fit_image_rect` — cells are taller than they are wide).
  let (cell_width, cell_height) = renderer.cell_pixels();
  let image_area = match app.cover_dims {
    Some((image_width, image_height)) if image_width > 0 && image_height > 0 => {
      let max_pixel_width = f64::from(inner.width) * f64::from(cell_width.max(1));
      let max_pixel_height = f64::from(inner.height) * f64::from(cell_height.max(1));
      let scale = (max_pixel_width / f64::from(image_width))
        .min(max_pixel_height / f64::from(image_height))
        .max(0.0);
      let fitted_width = ((f64::from(image_width) * scale) / f64::from(cell_width.max(1)))
        .round()
        .clamp(1.0, f64::from(inner.width)) as u16;
      let fitted_height = ((f64::from(image_height) * scale) / f64::from(cell_height.max(1)))
        .round()
        .clamp(1.0, f64::from(inner.height)) as u16;
      Rect {
        x: inner.x + inner.width.saturating_sub(fitted_width) / 2,
        y: inner.y + inner.height.saturating_sub(fitted_height) / 2,
        width: fitted_width,
        height: fitted_height,
      }
    }
    _ => inner,
  };

  let current_url = app.current_song_url().unwrap_or_default();
  let cover = app
    .cover_path
    .as_ref()
    .filter(|(url, _)| url == &current_url)
    .map(|(_, path)| path.clone());

  match cover {
    Some(path) => {
      renderer.request(&path, image_area.width, image_area.height, tx);
      match renderer.get(&path, image_area.width, image_area.height) {
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
          // Keep the TUI from touching cells under the protocol image.
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
              .style(Style::default().fg(theme.color(&theme.muted))),
            inner,
          );
        }
      }
    }
    None => {
      let hint = app
        .cover_error
        .clone()
        .unwrap_or_else(|| "no cover".to_string());
      frame.render_widget(
        Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.muted))),
        inner,
      );
    }
  }
}

pub(super) fn reserve_protocol_area(frame: &mut Frame, area: Rect) {
  let buffer = frame.buffer_mut();
  for y in area.y..area.y.saturating_add(area.height) {
    for x in area.x..area.x.saturating_add(area.width) {
      if let Some(cell) = buffer.cell_mut((x, y)) {
        cell.set_diff_option(CellDiffOption::Skip);
      }
    }
  }
}

/// Cover of the hovered row (queue or library, per the pane source).
#[allow(clippy::too_many_arguments)]
fn draw_hover_cover_pane(
  frame: &mut Frame,
  app: &mut App,
  renderer: &mut CoverRenderStore,
  tx: &mpsc::UnboundedSender<AsyncEvent>,
  area: Rect,
  overlays: &mut Vec<ProtocolOverlay>,
  source: PaneSource,
) {
  let theme = &app.settings.theme;
  let is_main = app.main_pane() == PaneKind::Cover;
  let title = match app.hover_view(source) {
    Some(hover) => format!("cover · {}", hover.title),
    None => "cover (hovered)".to_string(),
  };
  let block = pane_block(app, &title, is_main);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.width < 2 || inner.height < 2 {
    return;
  }

  let Some(hover) = app.hover_view(source) else {
    frame.render_widget(
      Paragraph::new("hover a queue or library entry")
        .style(Style::default().fg(theme.color(&theme.muted))),
      inner,
    );
    return;
  };

  let (cell_width, cell_height) = renderer.cell_pixels();
  let image_area = match hover.cover_dims {
    Some((image_width, image_height)) if image_width > 0 && image_height > 0 => {
      let max_pixel_width = f64::from(inner.width) * f64::from(cell_width.max(1));
      let max_pixel_height = f64::from(inner.height) * f64::from(cell_height.max(1));
      let scale = (max_pixel_width / f64::from(image_width))
        .min(max_pixel_height / f64::from(image_height))
        .max(0.0);
      let fitted_width = ((f64::from(image_width) * scale) / f64::from(cell_width.max(1)))
        .round()
        .clamp(1.0, f64::from(inner.width)) as u16;
      let fitted_height = ((f64::from(image_height) * scale) / f64::from(cell_height.max(1)))
        .round()
        .clamp(1.0, f64::from(inner.height)) as u16;
      Rect {
        x: inner.x + inner.width.saturating_sub(fitted_width) / 2,
        y: inner.y + inner.height.saturating_sub(fitted_height) / 2,
        width: fitted_width,
        height: fitted_height,
      }
    }
    _ => inner,
  };

  match &hover.cover {
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
              .style(Style::default().fg(theme.color(&theme.muted))),
            inner,
          );
        }
      }
    }
    None => {
      let hint = hover
        .cover_error
        .clone()
        .unwrap_or_else(|| "no cover".to_string());
      frame.render_widget(
        Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.muted))),
        inner,
      );
    }
  }
}
