//! Terminal UI: tab bar, layout-tree panes, footer, prompt and popups.

use std::time::Duration;

use framework_tui::{
  KeyHelpDialogStyle, KeyHintsStyle, PopupDialogStyle, PromptLineStyle, completion_list_style,
  draw_completion_list, draw_key_help_dialog_scrolled, draw_key_hints, draw_prompt_line,
  key_hint_columns, key_hint_rows, overlay_background,
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
use unicode_width::UnicodeWidthChar;

use crate::{
  app::{App, song_album, song_artist, song_title},
  event::{AsyncEvent, RenderedImage},
  layout::{PaneKind, PaneLayout, PaneSource, SplitDir},
  render::CoverRenderStore,
  strip::StrippedText,
  terminal::FrameOutput,
};

pub(crate) mod cover;
pub(crate) mod detail;
pub(crate) mod footer;
pub(crate) mod help;
pub(crate) mod library;
pub(crate) mod lyrics;
pub(crate) mod metadata;
pub(crate) mod queue;
pub(crate) mod visualizer;

use cover::draw_cover_pane;
use detail::draw_detail_view;
use footer::draw_footer;
use help::{draw_completion_popup, draw_help_dialog};
use library::draw_library_pane;
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
  let base_bg = app.settings.theme.base_background();
  if base_bg != ratatui::style::Color::Reset {
    frame.render_widget(Block::default().style(Style::default().bg(base_bg)), area);
  }

  // Footer height grows with the pending which-key hints (pdf-tui style).
  let hints: Vec<framework_tui::KeyHint> = if app.show_help {
    Vec::new()
  } else {
    app
      .dispatcher
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
      key_hint_columns(
        usize::from(app.settings.theme.which_key.columns),
        area.width,
      ),
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
  app.queue_bar_areas.clear();
  app.library_pane_areas.clear();
  app.library_bar_areas.clear();

  draw_tab_bar(frame, app, tab_bar);

  let mut overlays = Vec::new();
  // Anti-flicker state (pdf-tui/gallery-tui): while a protocol image is
  // in flight, its old pixels are preserved instead of being erased.
  let mut preserve_overlays = false;
  let mut preserve_areas = Vec::new();
  let tab = app.current_tab().clone();
  if let Some(detail) = app.detail.as_ref() {
    // Secondary detail view: replaces the tab content (gallery-tui style).
    draw_detail_view(
      frame,
      app,
      detail,
      renderer,
      tx,
      content,
      &mut overlays,
      &mut preserve_overlays,
      &mut preserve_areas,
    );
  } else {
    draw_layout(
      frame,
      app,
      renderer,
      tx,
      content,
      &tab.layout,
      &mut overlays,
      &mut preserve_overlays,
      &mut preserve_areas,
    );
  }

  let mut cursor_position = draw_footer(frame, app, footer, &hints);
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
    overlays,
    protocol_writes: Vec::new(),
    cursor_position,
    preserve_overlays,
    preserve_areas,
    occluders,
  }
}

fn draw_tab_bar(frame: &mut Frame, app: &mut App, area: Rect) {
  let theme = &app.settings.theme;
  let border = Style::default().fg(theme.color(&theme.base.border));
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
        .fg(theme.color(&theme.tab_bar.active))
        .add_modifier(Modifier::BOLD)
    } else {
      Style::default().fg(theme.color(&theme.tab_bar.inactive))
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

#[allow(clippy::too_many_arguments)]
fn draw_layout(
  frame: &mut Frame,
  app: &mut App,
  renderer: &mut CoverRenderStore,
  tx: &mpsc::UnboundedSender<AsyncEvent>,
  area: Rect,
  layout: &PaneLayout,
  overlays: &mut Vec<ProtocolOverlay>,
  preserve_overlays: &mut bool,
  preserve_areas: &mut Vec<Rect>,
) {
  match layout {
    PaneLayout::Pane(kind, source) => {
      let ctx = PaneCtx {
        frame,
        renderer,
        tx,
        overlays,
        preserve_overlays,
        preserve_areas,
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
      draw_layout(
        frame,
        app,
        renderer,
        tx,
        areas[0],
        first,
        overlays,
        preserve_overlays,
        preserve_areas,
      );
      draw_layout(
        frame,
        app,
        renderer,
        tx,
        areas[1],
        second,
        overlays,
        preserve_overlays,
        preserve_areas,
      );
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
  pub(super) preserve_overlays: &'a mut bool,
  pub(super) preserve_areas: &'a mut Vec<Rect>,
}

fn draw_pane(app: &mut App, area: Rect, kind: PaneKind, source: PaneSource, ctx: PaneCtx<'_, '_>) {
  let PaneCtx {
    frame,
    renderer,
    tx,
    overlays,
    preserve_overlays,
    preserve_areas,
  } = ctx;
  match kind {
    PaneKind::Queue => draw_queue_pane(frame, app, area),
    PaneKind::Library => draw_library_pane(frame, app, area),
    PaneKind::Cover => draw_cover_pane(
      frame,
      app,
      renderer,
      tx,
      area,
      source,
      overlays,
      preserve_overlays,
      preserve_areas,
    ),
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

/// Display width of one char in terminal columns (CJK chars are 2).
pub(super) fn char_display_width(ch: char) -> usize {
  ch.width().unwrap_or(0)
}

/// Total display width of `text` in terminal columns.
fn text_display_width(text: &str) -> usize {
  text.chars().map(char_display_width).sum()
}

/// Number of chars starting strictly before `byte` (its char index).
fn chars_starting_before(text: &str, byte: usize) -> usize {
  text.char_indices().take_while(|(b, _)| *b < byte).count()
}

/// Pick a horizontal window for long field values so the match sits
/// visibly inside it, measured in terminal columns: CJK chars occupy
/// two columns, so a char-counted budget can overflow the cell. Returns
/// the char offset to start drawing at.
pub(super) fn match_window(text: &str, range: (usize, usize), budget: usize) -> usize {
  let total = text_display_width(text);
  if budget == 0 || total <= budget {
    return 0;
  }
  let chars: Vec<char> = text.chars().collect();
  let anchor = chars_starting_before(text, range.0);
  let match_end = chars_starting_before(text, range.1);
  // Lead the match in by up to a third of the budget, walking back by
  // display columns (never splitting a wide char).
  let lead_budget = budget / 3;
  let mut start = anchor;
  let mut lead = 0;
  while start > 0 {
    let width = char_display_width(chars[start - 1]);
    if lead + width > lead_budget {
      break;
    }
    lead += width;
    start -= 1;
  }
  // If the match still does not fit, slide the window forward until it does.
  while start < match_end
    && chars[start..match_end]
      .iter()
      .map(|ch| char_display_width(*ch))
      .sum::<usize>()
      > budget
  {
    start += 1;
  }
  start.min(chars.len() - 1)
}

/// The visible slice of `text` starting at char `start`, cut at the
/// first char that would overflow `budget` display columns.
pub(super) fn window_text(text: &str, start: usize, budget: usize) -> String {
  let mut used = 0;
  let mut window = String::new();
  for ch in text.chars().skip(start) {
    let width = char_display_width(ch);
    if used + width > budget {
      break;
    }
    used += width;
    window.push(ch);
  }
  window
}

/// Window for a filtered cell: anchored on the leftmost term match so
/// whichever match sorts first stays visible (anchoring on
/// `ranges.first()` can scroll an earlier match out of view). Returns
/// the visible text and its char offset into `text`.
pub(super) fn filter_window(
  text: &str,
  ranges: &[(usize, usize)],
  budget: usize,
) -> (String, usize) {
  let start = match ranges.iter().copied().min() {
    Some(range) => match_window(text, range, budget),
    None => 0,
  };
  (window_text(text, start, budget), start)
}

/// Largest char boundary <= `index` in `text`.
pub(super) fn char_boundary_index(text: &str, index: usize) -> usize {
  if index >= text.len() {
    text.len()
  } else {
    let mut index = index;
    while index > 0 && !text.is_char_boundary(index) {
      index -= 1;
    }
    index
  }
}

/// Multi-range variant: `ranges` hold byte offsets into the full `text`
/// (e.g. every filter term match); all matches inside the window are
/// highlighted.
pub(super) fn highlighted_ranges_spans(
  window: &str,
  text: &str,
  window_start: usize,
  ranges: Vec<(usize, usize)>,
  base: Style,
  highlight: Style,
) -> Vec<Span<'static>> {
  // Byte offset of the visible window inside `text`.
  let window_bytes: usize = text.chars().take(window_start).map(char::len_utf8).sum();
  let mut shifted: Vec<(usize, usize)> = ranges
    .into_iter()
    .map(|(start, end)| {
      (
        start.saturating_sub(window_bytes),
        end.saturating_sub(window_bytes).min(window.len()),
      )
    })
    .filter(|(start, end)| start < end && *end <= window.len())
    .collect();
  shifted.sort();
  // Merge overlapping ranges, then emit plain/highlight segments.
  let mut merged: Vec<(usize, usize)> = Vec::new();
  for (start, end) in shifted {
    match merged.last_mut() {
      Some((_, last_end)) if start <= *last_end => *last_end = (*last_end).max(end),
      _ => merged.push((start, end)),
    }
  }
  let mut spans = Vec::new();
  let mut cursor = 0;
  for (start, end) in merged {
    let start = char_boundary_index(window, start);
    let end = char_boundary_index(window, end);
    if start < cursor || start >= end {
      continue;
    }
    if cursor < start {
      spans.push(Span::styled(window[cursor..start].to_string(), base));
    }
    spans.push(Span::styled(window[start..end].to_string(), highlight));
    cursor = end;
  }
  if cursor < window.len() {
    spans.push(Span::styled(window[cursor..].to_string(), base));
  }
  if spans.is_empty() {
    spans.push(Span::styled(window.to_string(), base));
  }
  spans
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn match_window_measures_display_columns() {
    // 28 CJK chars = 56 columns; a 30-column budget must scroll.
    let text = format!("{}夜曲{}", "曲".repeat(20), "词".repeat(6));
    let start_byte = text.find("夜曲").unwrap();
    let range = (start_byte, start_byte + "夜曲".len());
    let start = match_window(&text, range, 30);
    let window = window_text(&text, start, 30);
    assert!(window.contains("夜曲"), "window {window:?} hides the match");
    assert!(
      window.chars().map(char_display_width).sum::<usize>() <= 30,
      "window overflows the column budget"
    );
  }

  #[test]
  fn filter_window_anchors_on_leftmost_match() {
    // "love" matches late in the text, "beatles" at the very start:
    // the leftmost match must stay visible.
    let text = format!("Beatles Band Song{} Love Song", " pad".repeat(10));
    let love = text.find("Love").unwrap();
    let beatles = text.find("Beatles").unwrap();
    let ranges = [(love, love + 4), (beatles, beatles + 7)];
    let (window, _) = filter_window(&text, &ranges, 20);
    assert!(
      window.contains("Beatles"),
      "window {window:?} hides the leftmost match"
    );
  }

  #[test]
  fn window_text_never_splits_wide_chars() {
    // 7 columns fit only 3 double-width chars.
    assert_eq!(window_text("曲曲曲", 0, 7).chars().count(), 3);
    assert_eq!(window_text("曲曲曲", 0, 5).chars().count(), 2);
    assert_eq!(window_text("夜曲", 1, 10), "曲");
  }
}
