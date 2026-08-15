//! Secondary detail view rendering for a queue entry.

use super::*;

/// Secondary detail surface for a queue entry (`i`): a layout tree over the
/// cover and metadata panes (default side by side) — the sidebar data stays
/// untouched. Layout comes from `[layout].detail`.
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
  let mut ctx = DetailCtx {
    app,
    detail,
    renderer,
    tx,
    overlays,
  };
  draw_detail_layout(frame, &mut ctx, inner, &app.detail_layout);
}

/// Shared borrow bundle so the layout recursion stays under clippy's
/// argument limit.
struct DetailCtx<'a> {
  app: &'a App,
  detail: &'a crate::app::DetailView,
  renderer: &'a mut CoverRenderStore,
  tx: &'a mpsc::UnboundedSender<AsyncEvent>,
  overlays: &'a mut Vec<ProtocolOverlay>,
}

fn draw_detail_layout(
  frame: &mut Frame,
  ctx: &mut DetailCtx<'_>,
  area: Rect,
  layout: &PaneLayout,
) {
  match layout {
    PaneLayout::Pane(kind, _) => match kind {
      PaneKind::Cover => draw_detail_cover(frame, ctx, area),
      PaneKind::Metadata => draw_detail_metadata(frame, ctx, area),
      // The config validator only admits cover/metadata panes here.
      _ => {}
    },
    PaneLayout::Split {
      dir,
      ratio,
      first,
      second,
    } => {
      let constraints = [
        Constraint::Ratio(ratio.0, ratio.0 + ratio.1),
        Constraint::Ratio(ratio.1, ratio.0 + ratio.1),
      ];
      let areas: [Rect; 2] = match dir {
        SplitDir::Horizontal => Layout::horizontal(constraints).areas(area),
        SplitDir::Vertical => Layout::vertical(constraints).areas(area),
      };
      draw_detail_layout(frame, ctx, areas[0], first);
      draw_detail_layout(frame, ctx, areas[1], second);
    }
  }
}

/// Cover with the same aspect-correct fitting math as the cover pane.
fn draw_detail_cover(frame: &mut Frame, ctx: &mut DetailCtx<'_>, cover_area: Rect) {
  let theme = &ctx.app.settings.theme;
  let detail = ctx.detail;
  let (cell_width, cell_height) = ctx.renderer.cell_pixels();
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
      ctx.renderer
        .request(path, image_area.width, image_area.height, ctx.tx);
      match ctx
        .renderer
        .get(path, image_area.width, image_area.height)
      {
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
          ctx.overlays.push(ProtocolOverlay {
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
}

fn draw_detail_metadata(frame: &mut Frame, ctx: &DetailCtx<'_>, metadata_area: Rect) {
  let theme = &ctx.app.settings.theme;
  let detail = ctx.detail;
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
        .map(|entry| metadata_line(ctx.app, &entry.name, &entry.value))
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
