//! Secondary detail view rendering for a queue entry.

use super::cover::{draw_cover_art, fitted_cover_area};
use super::metadata::draw_metadata_entries;
use super::*;
use crate::app::SongView;

/// Secondary detail surface for a queue entry (`i`): a layout tree over the
/// cover and metadata panes (default side by side) — the sidebar data stays
/// untouched. Layout comes from `[layout].detail`.
pub(super) fn draw_detail_view(
  frame: &mut Frame,
  app: &App,
  detail: &SongView,
  images: &mut ImageSink<'_>,
  area: Rect,
) {
  let theme = &app.settings.theme;
  let block = Block::default()
    .borders(Borders::ALL)
    .border_style(Style::default().fg(theme.color(&theme.base.accent)))
    .title(format!(" detail: {} ", detail.title))
    .title_alignment(Alignment::Center);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.width < 2 || inner.height < 2 {
    return;
  }
  draw_detail_layout(frame, app, detail, images, inner, &app.detail_layout);
}

fn draw_detail_layout(
  frame: &mut Frame,
  app: &App,
  detail: &SongView,
  images: &mut ImageSink<'_>,
  area: Rect,
  layout: &PaneLayout,
) {
  match layout {
    PaneLayout::Pane(PaneKind::Cover, _) => {
      let theme = &app.settings.theme;
      let image_area = fitted_cover_area(detail.cover_dims, area, images.renderer.cell_pixels());
      let muted = Style::default().fg(theme.color(&theme.base.muted));
      draw_cover_art(frame, images, muted, detail, image_area, area);
    }
    PaneLayout::Pane(PaneKind::Metadata, _) => draw_detail_metadata(frame, app, detail, area),
    // The config validator only admits cover/metadata panes here.
    PaneLayout::Pane(..) => {}
    PaneLayout::Split {
      dir,
      ratio,
      first,
      second,
    } => {
      let [first_area, second_area] = split_areas(area, *dir, *ratio);
      draw_detail_layout(frame, app, detail, images, first_area, first);
      draw_detail_layout(frame, app, detail, images, second_area, second);
    }
  }
}

fn draw_detail_metadata(frame: &mut Frame, app: &App, detail: &SongView, metadata_area: Rect) {
  let theme = &app.settings.theme;
  let metadata_block = Block::default()
    .borders(Borders::ALL)
    .border_style(Style::default().fg(theme.color(&theme.base.border)))
    .title(" metadata (e edit · i close) ");
  let metadata_inner = metadata_block.inner(metadata_area);
  frame.render_widget(metadata_block, metadata_area);
  if metadata_inner.height == 0 {
    return;
  }
  draw_metadata_entries(frame, app, detail, metadata_inner, "reading metadata…");
}
