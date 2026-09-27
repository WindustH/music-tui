//! Cover pane rendering (protocol images and Chafa symbols).

use img_tui::reserve_protocol_area;

use super::*;

/// Draw cover art into `image_area` (aspect-fitted by the caller); hints
/// go to `text_area`.
///
/// Protocol modes preserve the previous artwork's pixels while the next
/// one is in flight (pdf-tui/gallery-tui's anti-flicker mechanism,
/// executed by img-tui's overlay renderer); a definitive error ("no
/// cover") replaces them.
pub(super) fn draw_cover_art(
  frame: &mut Frame,
  images: &mut ImageSink<'_>,
  muted: Style,
  view: &crate::app::SongView,
  image_area: Rect,
  text_area: Rect,
) {
  let Some(path) = view.cover.as_deref() else {
    // No path yet: still locating the artwork → keep old pixels; a set
    // error means the song really has no cover → replace them.
    if view.cover_error.is_none() && images.renderer.draws_with_protocol() {
      images.preserve(image_area);
      return;
    }
    let hint = view.cover_error.as_deref().unwrap_or("no cover");
    frame.render_widget(Paragraph::new(hint).style(muted), text_area);
    return;
  };
  images
    .renderer
    .request(path, image_area.width, image_area.height, images.tx);
  match images
    .renderer
    .get(path, image_area.width, image_area.height)
  {
    Some(RenderedImage::Symbols { paragraph, .. }) => {
      frame.render_widget(paragraph.as_ref(), image_area);
    }
    Some(RenderedImage::Protocol(image)) => {
      // Keep the TUI from touching cells under the protocol image.
      reserve_protocol_area(frame, image_area);
      images.overlays.push(image.overlay(image_area));
    }
    None => {
      if let Some(error) = images
        .renderer
        .error(path, image_area.width, image_area.height)
      {
        frame.render_widget(
          Paragraph::new(format!("cover could not be drawn: {error}"))
            .style(muted)
            .wrap(Wrap { trim: true }),
          text_area,
        );
        return;
      }
      // Render in flight: hold the old pixels instead of flashing text.
      if images.renderer.draws_with_protocol() {
        images.preserve(image_area);
        return;
      }
      frame.render_widget(Paragraph::new("rendering cover…").style(muted), text_area);
    }
  }
}

/// Cover pane for any data source (playing song or a hovered row).
pub(super) fn draw_cover_pane(
  frame: &mut Frame,
  app: &mut App,
  images: &mut ImageSink<'_>,
  area: Rect,
  source: PaneSource,
) {
  let theme = &app.settings.theme;
  let is_main = app.main_pane() == PaneKind::Cover;
  let view = app.song_view(source);
  let title = match (source, view) {
    (PaneSource::Playing, _) => "cover".to_string(),
    (_, Some(view)) => format!("cover · {}", view.title),
    (_, None) => "cover (hovered)".to_string(),
  };
  let block = pane_block(app, &title, is_main);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.width < 2 || inner.height < 2 {
    return;
  }
  let muted = Style::default().fg(theme.color(&theme.base.muted));
  let Some(view) = view else {
    let hint = match source {
      PaneSource::Playing if app.current_song().is_some() => "no local file for this song",
      PaneSource::Playing => "nothing playing",
      _ => "hover a queue or library entry",
    };
    frame.render_widget(Paragraph::new(hint).style(muted), inner);
    return;
  };
  let image_area = fitted_cover_area(view.cover_dims, inner, images.renderer.cell_pixels());
  draw_cover_art(frame, images, muted, view, image_area, inner);
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
