//! Terminal UI: tab bar, layout-tree panes, footer, prompt and popups.

use std::time::Duration;

use framework_tui::{
  CompletionListStyle, KeyHelpDialogStyle, KeyHintsStyle, PopupDialogStyle, PromptLineStyle,
  draw_completion_list, draw_key_help_dialog_scrolled, draw_key_hints, draw_prompt_line,
  key_hint_columns, key_hint_rows,
};
use img_tui::ProtocolOverlay;
use mpd_client::commands::SingleMode;
use mpd_client::responses::{PlayState, SongInQueue};
use ratatui::{
  Frame,
  buffer::CellDiffOption,
  layout::{Alignment, Constraint, Layout, Rect},
  style::{Modifier, Style},
  text::{Line, Span},
  widgets::{Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, Wrap},
};
use tokio::sync::mpsc;

use crate::{
  app::App,
  event::{AsyncEvent, RenderedImage},
  layout::{PaneKind, PaneLayout, PaneSource, SplitDir},
  render::CoverRenderStore,
  terminal::FrameOutput,
};

pub(crate) mod cover;
pub(crate) mod detail;
pub(crate) mod footer;
pub(crate) mod help;
pub(crate) mod lyrics;
pub(crate) mod metadata;
pub(crate) mod queue;
pub(crate) mod visualizer;

use cover::{draw_cover_pane, reserve_protocol_area};
use detail::draw_detail_view;
use footer::draw_footer;
use help::{draw_completion_popup, draw_help_dialog};
use lyrics::draw_lyrics_pane;
use metadata::{draw_metadata_pane, metadata_line};
use queue::draw_queue_pane;
use visualizer::draw_visualizer_pane;

pub fn draw(
  frame: &mut Frame,
  app: &mut App,
  renderer: &mut CoverRenderStore,
  tx: &mpsc::UnboundedSender<AsyncEvent>,
) -> FrameOutput {
  let area = frame.area();

  // Footer height grows with the pending which-key hints (pdf-tui style).
  let hints: Vec<framework_tui::KeyHint> = if app.show_help {
    Vec::new()
  } else {
    app.dispatcher
      .hints()
      .iter()
      .map(|hint| framework_tui::KeyHint {
        key: hint.key.clone(),
        label: hint.label.clone(),
      })
      .collect()
  };
  let hint_rows = if hints.is_empty() {
    0
  } else {
    key_hint_rows(
      hints.len(),
      key_hint_columns(usize::from(app.settings.theme.which_key_columns), area.width),
    ) as u16
  };

  let [tab_bar, content, footer] = Layout::vertical([
    Constraint::Length(1),
    Constraint::Min(0),
    Constraint::Length(3 + hint_rows),
  ])
  .areas(area);

  app.lyrics_pane_areas.clear();
  app.lyrics_pane_sources.clear();
  app.queue_pane_areas.clear();

  draw_tab_bar(frame, app, tab_bar);

  let mut overlays = Vec::new();
  let tab = app.current_tab().clone();
  if let Some(detail) = app.detail.as_ref() {
    // Secondary detail view: replaces the tab content (gallery-tui style).
    draw_detail_view(frame, app, detail, renderer, tx, content, &mut overlays);
  } else {
    draw_layout(frame, app, renderer, tx, content, &tab.layout, &mut overlays);
  }

  let mut cursor_position = draw_footer(frame, app, footer, &hints);
  draw_completion_popup(frame, app, footer);

  if app.show_help {
    let (cleared_overlays, no_cursor) = draw_help_dialog(frame, app, area);
    overlays = cleared_overlays;
    cursor_position = no_cursor;
  }

  FrameOutput {
    overlays,
    protocol_writes: Vec::new(),
    cursor_position,
    preserve_overlays: false,
    preserve_areas: Vec::new(),
  }
}

fn draw_tab_bar(frame: &mut Frame, app: &mut App, area: Rect) {
  let theme = &app.settings.theme;
  let border = Style::default().fg(theme.color(&theme.border));
  app.tab_hit_areas.clear();
  let mut spans = Vec::new();
  let mut column = area.x;
  for (index, tab) in app.tabs.iter().enumerate() {
    if index > 0 {
      spans.push(Span::styled(" │ ", border));
      column += 3;
    }
    let active = index == app.tab;
    let label = format!(" {} ", tab.name);
    let style = if active {
      Style::default()
        .fg(theme.color(&theme.accent))
        .add_modifier(Modifier::BOLD)
    } else {
      Style::default().fg(theme.color(&theme.muted))
    };
    // Record the hit rectangle for mouse-based tab switching.
    if app.tab_hit_areas.len() < 9 {
      app.tab_hit_areas.push(Rect {
        x: column,
        y: area.y,
        width: label.chars().count() as u16,
        height: 1,
      });
    }
    column += label.chars().count() as u16;
    spans.push(Span::styled(label, style));
  }
  frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_layout(
  frame: &mut Frame,
  app: &mut App,
  renderer: &mut CoverRenderStore,
  tx: &mpsc::UnboundedSender<AsyncEvent>,
  area: Rect,
  layout: &PaneLayout,
  overlays: &mut Vec<ProtocolOverlay>,
) {
  match layout {
    PaneLayout::Pane(kind, source) => {
      let ctx = PaneCtx {
        frame,
        renderer,
        tx,
        overlays,
      };
      draw_pane(app, area, *kind, *source, ctx)
    }
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
      draw_layout(frame, app, renderer, tx, areas[0], first, overlays);
      draw_layout(frame, app, renderer, tx, areas[1], second, overlays);
    }
  }
}

/// Everything a pane needs to draw itself; keeps the `draw_*_pane`
/// signatures uniform and `draw_pane` under the clippy argument limit.
pub(super) struct PaneCtx<'a, 'f> {
  pub(super) frame: &'a mut Frame<'f>,
  pub(super) renderer: &'a mut CoverRenderStore,
  pub(super) tx: &'a mpsc::UnboundedSender<AsyncEvent>,
  pub(super) overlays: &'a mut Vec<ProtocolOverlay>,
}

fn draw_pane(
  app: &mut App,
  area: Rect,
  kind: PaneKind,
  source: PaneSource,
  ctx: PaneCtx<'_, '_>,
) {
  let PaneCtx {
    frame,
    renderer,
    tx,
    overlays,
  } = ctx;
  match kind {
    PaneKind::Queue => draw_queue_pane(frame, app, area),
    PaneKind::Cover => draw_cover_pane(frame, app, renderer, tx, area, source, overlays),
    PaneKind::Lyrics => draw_lyrics_pane(frame, app, area, source),
    PaneKind::Metadata => draw_metadata_pane(frame, app, area, source),
    PaneKind::Visualizer => draw_visualizer_pane(frame, app, area),
  }
}

fn pane_block(app: &App, title: &str, is_main: bool) -> Block<'static> {
  let theme = &app.settings.theme;
  let title_span = if is_main {
    Span::styled(
      format!(" {title} "),
      Style::default()
        .fg(theme.color(&theme.accent))
        .add_modifier(Modifier::BOLD),
    )
  } else {
    Span::styled(format!(" {title} "), Style::default().fg(theme.color(&theme.muted)))
  };
  Block::bordered()
    .title(title_span)
    .border_style(Style::default().fg(theme.color(&theme.border)))
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
