//! Secondary detail view rendering for a queue entry.

use super::*;
use super::cover::{draw_cover_art, fitted_cover_area};

/// Secondary detail surface for a queue entry (`i`): a layout tree over the
/// cover and metadata panes (default side by side) — the sidebar data stays
/// untouched. Layout comes from `[layout].detail`.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_detail_view(
  frame: &mut Frame,
  app: &App,
  detail: &crate::app::SongView,
  renderer: &mut CoverRenderStore,
  tx: &mpsc::UnboundedSender<AsyncEvent>,
  area: Rect,
  overlays: &mut Vec<ProtocolOverlay>,
  preserve_overlays: &mut bool,
  preserve_areas: &mut Vec<Rect>,
) {
  let theme = &app.settings.theme;
  let block = Block::default()
    .borders(Borders::ALL)
    .border_style(Style::default().fg(theme.color(&app.settings.theme.base.accent)))
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
    preserve_overlays,
    preserve_areas,
  };
  draw_detail_layout(frame, &mut ctx, inner, &app.detail_layout);
}

/// Shared borrow bundle so the layout recursion stays under clippy's
/// argument limit.
struct DetailCtx<'a> {
  app: &'a App,
  detail: &'a crate::app::SongView,
  renderer: &'a mut CoverRenderStore,
  tx: &'a mpsc::UnboundedSender<AsyncEvent>,
  overlays: &'a mut Vec<ProtocolOverlay>,
  preserve_overlays: &'a mut bool,
  preserve_areas: &'a mut Vec<Rect>,
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

/// Cover with the same aspect-correct fitting and anti-flicker preserve as
/// the cover pane.
fn draw_detail_cover(frame: &mut Frame, ctx: &mut DetailCtx<'_>, cover_area: Rect) {
  let theme = &ctx.app.settings.theme;
  let image_area = fitted_cover_area(ctx.detail.cover_dims, cover_area, ctx.renderer.cell_pixels());
  let muted = Style::default().fg(theme.color(&theme.base.muted));
  draw_cover_art(
    frame,
    ctx.renderer,
    ctx.tx,
    muted,
    ctx.detail.cover.as_deref(),
    ctx.detail.cover_error.as_deref(),
    image_area,
    cover_area,
    ctx.overlays,
    ctx.preserve_overlays,
    ctx.preserve_areas,
  );
}

fn draw_detail_metadata(frame: &mut Frame, ctx: &mut DetailCtx<'_>, metadata_area: Rect) {
  let theme = &ctx.app.settings.theme;
  let detail = ctx.detail;
  let metadata_block = Block::default()
    .borders(Borders::ALL)
    .border_style(Style::default().fg(theme.color(&theme.base.border)))
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
        Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.base.muted))),
        metadata_inner,
      );
    }
  }
}
