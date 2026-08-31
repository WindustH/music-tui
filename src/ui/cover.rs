//! Cover pane rendering (protocol images and Chafa symbols).

use std::path::Path;

use super::*;

/// Mark `area` as pixel-preserved: while a protocol image is being
/// prepared (or its file located), the previous frame's pixels stay on
/// screen instead of flashing placeholder text — pdf-tui/gallery-tui's
/// anti-flicker mechanism, executed by img-tui's overlay renderer.
pub(super) fn preserve_frame_area(
  area: Rect,
  preserve_overlays: &mut bool,
  preserve_areas: &mut Vec<Rect>,
) {
  *preserve_overlays = true;
  preserve_areas.push(area);
}

/// Draw cover art into `image_area` (aspect-fitted by the caller).
///
/// Protocol modes preserve the previous artwork's pixels while the next
/// one is in flight; a definitive error ("no cover") replaces them.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_cover_art(
  frame: &mut Frame,
  renderer: &mut CoverRenderStore,
  tx: &mpsc::UnboundedSender<AsyncEvent>,
  muted: Style,
  cover: Option<&Path>,
  cover_error: Option<&str>,
  image_area: Rect,
  text_area: Rect,
  overlays: &mut Vec<ProtocolOverlay>,
  preserve_overlays: &mut bool,
  preserve_areas: &mut Vec<Rect>,
) {
  let Some(path) = cover else {
    // No path yet: still locating the artwork → keep old pixels; a set
    // error means the song really has no cover → replace them.
    if cover_error.is_none() && renderer.draws_with_protocol() {
      preserve_frame_area(image_area, preserve_overlays, preserve_areas);
      return;
    }
    let hint = cover_error.unwrap_or("no cover");
    frame.render_widget(Paragraph::new(hint).style(muted), text_area);
    return;
  };
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
      // Render in flight: hold the old pixels instead of flashing text.
      if renderer.draws_with_protocol() {
        preserve_frame_area(image_area, preserve_overlays, preserve_areas);
        return;
      }
      frame.render_widget(Paragraph::new("rendering cover…").style(muted), text_area);
    }
  }
}

/// Cover of the current song (`:playing` source).
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_cover_pane(
  frame: &mut Frame,
  app: &mut App,
  renderer: &mut CoverRenderStore,
  tx: &mpsc::UnboundedSender<AsyncEvent>,
  area: Rect,
  source: PaneSource,
  overlays: &mut Vec<ProtocolOverlay>,
  preserve_overlays: &mut bool,
  preserve_areas: &mut Vec<Rect>,
) {
  if matches!(
    source,
    PaneSource::QueueHovered | PaneSource::LibraryHovered
  ) {
    draw_hover_cover_pane(
      frame,
      app,
      renderer,
      tx,
      area,
      overlays,
      source,
      preserve_overlays,
      preserve_areas,
    );
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

  let image_area = fitted_cover_area(app.cover_dims, inner, renderer.cell_pixels());
  let current_url = app.current_song_url().unwrap_or_default();
  let cover = app
    .cover_path
    .as_ref()
    .filter(|(url, _)| url == &current_url)
    .map(|(_, path)| path.as_path());
  let muted = Style::default().fg(theme.color(&theme.base.muted));
  draw_cover_art(
    frame,
    renderer,
    tx,
    muted,
    cover,
    app.cover_error.as_deref(),
    image_area,
    inner,
    overlays,
    preserve_overlays,
    preserve_areas,
  );
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
  preserve_overlays: &mut bool,
  preserve_areas: &mut Vec<Rect>,
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
        .style(Style::default().fg(theme.color(&theme.base.muted))),
      inner,
    );
    return;
  };

  let image_area = fitted_cover_area(hover.cover_dims, inner, renderer.cell_pixels());
  let muted = Style::default().fg(theme.color(&theme.base.muted));
  draw_cover_art(
    frame,
    renderer,
    tx,
    muted,
    hover.cover.as_deref(),
    hover.cover_error.as_deref(),
    image_area,
    inner,
    overlays,
    preserve_overlays,
    preserve_areas,
  );
}

/// Aspect-correct artwork rectangle: fit the intrinsic pixel size inside
/// `inner`, converting through cell pixels (cells are taller than wide) —
/// same math as gallery-tui's `fit_image_rect`.
pub(super) fn fitted_cover_area(
  dims: Option<(u32, u32)>,
  inner: Rect,
  (cell_width, cell_height): (u16, u16),
) -> Rect {
  let Some((image_width, image_height)) = dims else {
    return inner;
  };
  if image_width == 0 || image_height == 0 || inner.width < 2 || inner.height < 2 {
    return inner;
  }
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
