//! Terminal UI: tab bar, layout-tree panes, footer, prompt and popups.

use std::time::Duration;

use framework_tui::{
  KeyHelpDialogStyle, KeyHintsStyle, PopupDialogStyle, PromptLineStyle, completion_list_style,
  draw_completion_list, draw_key_help_dialog_scrolled, draw_key_hints, draw_prompt_line,
  key_hint_columns, key_hint_rows,
};
use img_tui::ProtocolOverlay;
use mpd_client::commands::SingleMode;
use mpd_client::responses::{PlayState, SongInQueue};
use ratatui::{
  Frame,
  layout::{Alignment, Constraint, Layout, Rect},
  style::{Modifier, Style},
  text::{Line, Span},
  widgets::{Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, Wrap},
};
use tokio::sync::mpsc;

use highlight::text_display_width;

use crate::{
  app::{App, song_album, song_artist, song_title},
  event::{AsyncEvent, RenderedImage},
  layout::{PaneKind, PaneLayout, PaneSource, SplitDir},
  render::CoverRenderStore,
  strip::StrippedText,
  terminal::FrameOutput,
};

mod cover;
mod detail;
mod footer;
mod help;
mod highlight;
mod library;
mod lyrics;
mod metadata;
mod queue;
mod visualizer;

use cover::draw_cover_pane;
use detail::draw_detail_view;
use footer::draw_footer;
use help::{draw_completion_popup, draw_help_dialog};
use library::draw_library_pane;
use lyrics::draw_lyrics_pane;
use metadata::draw_metadata_pane;
use queue::draw_queue_pane;
use visualizer::draw_visualizer_pane;

pub fn draw(
  frame: &mut Frame,
  app: &mut App,
  renderer: &mut CoverRenderStore,
  tx: &mpsc::UnboundedSender<AsyncEvent>,
) -> FrameOutput {
  let area = frame.area();
  let base_bg = app.settings.theme.base_background();
  if base_bg != ratatui::style::Color::Reset {
    frame.render_widget(Block::default().style(Style::default().bg(base_bg)), area);
  }

  // Footer height grows with the pending which-key hints (pdf-tui style).
  let hints: &[framework_tui::KeyHint] = if app.show_help {
    &[]
  } else {
    app.dispatcher.hints()
  };
  let hint_columns = key_hint_columns(
    usize::from(app.settings.theme.which_key.columns),
    area.width,
  );
  let hint_rows = if hints.is_empty() {
    0
  } else {
    key_hint_rows(hints.len(), hint_columns) as u16
  };
  let hints = hints.to_vec();

  let [tab_bar, content, footer] = Layout::vertical([
    Constraint::Length(1),
    Constraint::Min(0),
    Constraint::Length(3 + hint_rows),
  ])
  .areas(area);

  app.hit.clear_panes();
  draw_tab_bar(frame, app, tab_bar);

  let mut images = ImageSink::new(renderer, tx);
  if let Some(detail) = app.detail.as_ref() {
    // Secondary detail view: replaces the tab content (gallery-tui style).
    draw_detail_view(frame, app, detail, &mut images, content);
  } else {
    let layout = app.current_tab().layout.clone();
    draw_layout(frame, app, &mut images, content, &layout);
  }

  let mut cursor_position = draw_footer(frame, app, footer, &hints, hint_rows, hint_columns);
  // Modal rects replace kitty U=1 placeholder cells. Uncovered cells keep
  // displaying the image, and the regular text diff restores placeholders
  // when a modal closes.
  let mut occluders = Vec::new();
  if let Some(popup) = draw_completion_popup(frame, app, footer) {
    occluders.push(popup);
  }

  if app.show_help {
    let (help_popup, no_cursor) = draw_help_dialog(frame, app, area);
    cursor_position = no_cursor;
    if let Some(help_popup) = help_popup {
      occluders.push(help_popup);
    }
  }

  FrameOutput {
    overlays: images.overlays,
    protocol_writes: Vec::new(),
    cursor_position,
    preserve_overlays: images.preserve_overlays,
    preserve_areas: images.preserve_areas,
    occluders,
  }
}

/// Cover-art plumbing for one frame: the render store and event sender
/// plus the protocol overlays and anti-flicker areas the panes produce.
pub(super) struct ImageSink<'a> {
  pub(super) renderer: &'a mut CoverRenderStore,
  pub(super) tx: &'a mpsc::UnboundedSender<AsyncEvent>,
  pub(super) overlays: Vec<ProtocolOverlay>,
  /// Anti-flicker state (pdf-tui/gallery-tui): while a protocol image is
  /// in flight, its old pixels are preserved instead of being erased.
  pub(super) preserve_overlays: bool,
  pub(super) preserve_areas: Vec<Rect>,
}

impl<'a> ImageSink<'a> {
  fn new(renderer: &'a mut CoverRenderStore, tx: &'a mpsc::UnboundedSender<AsyncEvent>) -> Self {
    Self {
      renderer,
      tx,
      overlays: Vec::new(),
      preserve_overlays: false,
      preserve_areas: Vec::new(),
    }
  }

  /// Keep the previous frame's pixels in `area` (image still loading).
  pub(super) fn preserve(&mut self, area: Rect) {
    self.preserve_overlays = true;
    self.preserve_areas.push(area);
  }
}

fn draw_tab_bar(frame: &mut Frame, app: &mut App, area: Rect) {
  let theme = &app.settings.theme;
  let border = Style::default().fg(theme.color(&theme.base.border));
  app.hit.tabs.clear();
  let mut spans = Vec::new();
  let mut column = area.x;
  for (index, tab) in app.tabs.iter().enumerate() {
    if index > 0 {
      spans.push(Span::styled(" │ ", border));
      column = column.saturating_add(3);
    }
    let active = index == app.tab;
    let label = format!(" {} ", tab.name);
    let width = text_display_width(&label).min(usize::from(u16::MAX)) as u16;
    let style = if active {
      Style::default()
        .fg(theme.color(&theme.tab_bar.active))
        .add_modifier(Modifier::BOLD)
    } else {
      Style::default().fg(theme.color(&theme.tab_bar.inactive))
    };
    // Record the hit rectangle for mouse-based tab switching.
    app.hit.tabs.push(Rect {
      x: column,
      y: area.y,
      width,
      height: 1,
    });
    column = column.saturating_add(width);
    spans.push(Span::styled(label, style));
  }
  frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_layout(
  frame: &mut Frame,
  app: &mut App,
  images: &mut ImageSink<'_>,
  area: Rect,
  layout: &PaneLayout,
) {
  match layout {
    PaneLayout::Pane(kind, source) => match kind {
      PaneKind::Queue => draw_queue_pane(frame, app, area),
      PaneKind::Library => draw_library_pane(frame, app, area),
      PaneKind::Cover => draw_cover_pane(frame, app, images, area, *source),
      PaneKind::Lyrics => draw_lyrics_pane(frame, app, area, *source),
      PaneKind::Metadata => draw_metadata_pane(frame, app, area, *source),
      PaneKind::Visualizer => draw_visualizer_pane(frame, app, area),
    },
    PaneLayout::Split {
      dir,
      ratio,
      first,
      second,
    } => {
      let [first_area, second_area] = split_areas(area, *dir, *ratio);
      draw_layout(frame, app, images, first_area, first);
      draw_layout(frame, app, images, second_area, second);
    }
  }
}

/// The two child areas of a split node.
pub(super) fn split_areas(area: Rect, dir: SplitDir, ratio: (u32, u32)) -> [Rect; 2] {
  let total = ratio.0.saturating_add(ratio.1);
  let constraints = [
    Constraint::Ratio(ratio.0, total),
    Constraint::Ratio(ratio.1, total),
  ];
  match dir {
    SplitDir::Horizontal => Layout::horizontal(constraints).areas(area),
    SplitDir::Vertical => Layout::vertical(constraints).areas(area),
  }
}

/// Rows `[start, end)` a stateful list or table shows for `len` one-line
/// rows in `height` lines — ratatui's own scrolling rule (the offset is
/// clamped to the content, then moved just enough to keep the selection
/// visible). Panes build only these rows instead of the whole list.
pub(super) fn visible_window(
  len: usize,
  height: usize,
  offset: usize,
  selected: Option<usize>,
) -> (usize, usize) {
  if len == 0 || height == 0 {
    return (0, 0);
  }
  let mut start = offset.min(len - 1);
  if let Some(selected) = selected.map(|selected| selected.min(len - 1)) {
    if selected >= start + height {
      start = selected + 1 - height;
    }
    start = start.min(selected);
  }
  (start, (start + height).min(len))
}

fn pane_block(app: &App, title: &str, is_main: bool) -> Block<'static> {
  let theme = &app.settings.theme;
  let title_span = if is_main {
    Span::styled(
      format!(" {title} "),
      Style::default()
        .fg(theme.color(&theme.base.accent))
        .add_modifier(Modifier::BOLD),
    )
  } else {
    Span::styled(
      format!(" {title} "),
      Style::default().fg(theme.color(&theme.base.muted)),
    )
  };
  Block::bordered()
    .title(title_span)
    .border_style(Style::default().fg(theme.color(&theme.base.border)))
}

/// `mm:ss` (or `h:mm:ss` for long tracks) for queue and footer labels.
pub(crate) fn format_duration_line(duration: Duration) -> String {
  let total = duration.as_secs();
  let hours = total / 3600;
  let minutes = (total % 3600) / 60;
  let seconds = total % 60;
  if hours > 0 {
    format!("{hours}:{minutes:02}:{seconds:02}")
  } else {
    format!("{minutes}:{seconds:02}")
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn visible_window_matches_ratatui_scrolling() {
    use ratatui::{
      backend::TestBackend,
      widgets::{List, ListState},
    };
    // Compare against ratatui's own List for many offset/selection pairs.
    let len = 12;
    let height = 5;
    let items: Vec<ListItem> = (0..len)
      .map(|index| ListItem::new(index.to_string()))
      .collect();
    let mut terminal = ratatui::Terminal::new(TestBackend::new(10, height as u16)).unwrap();
    for offset in [0, 3, 7, 11, 20] {
      for selected in [None, Some(0), Some(2), Some(6), Some(11), Some(30)] {
        let mut state = ListState::default();
        state.select(selected);
        *state.offset_mut() = offset;
        terminal
          .draw(|frame| {
            frame.render_stateful_widget(List::new(items.clone()), frame.area(), &mut state)
          })
          .unwrap();
        let (start, end) = visible_window(len, height, offset, selected);
        assert_eq!(
          start,
          state.offset(),
          "offset {offset} selected {selected:?}"
        );
        assert_eq!(end, (start + height).min(len));
      }
    }
    assert_eq!(visible_window(0, 5, 3, Some(1)), (0, 0));
  }
}
