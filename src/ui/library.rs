//! Library pane rendering, styled after calibre-tui's book table:
//! a bold header row, weighted columns, per-field colors, an inverted
//! hover bar, keyword highlighting, and a draggable viewport scrollbar.

use super::*;
use crate::library_db::TrackField;
use ratatui::widgets::{Cell, Row, Table};

/// Header row height (label + blank separator line below it).
const HEADER_ROWS: u16 = 2;

/// A resolved display column (parsed from `[library] columns`).
struct DisplayColumn {
  weight: u32,
  kind: ColumnKind,
}

enum ColumnKind {
  Field(TrackField),
  Duration,
}

impl DisplayColumn {
  fn label(&self) -> &'static str {
    match self.kind {
      ColumnKind::Field(TrackField::Title) => "title",
      ColumnKind::Field(TrackField::Artist) => "artist",
      ColumnKind::Field(TrackField::Album) => "album",
      ColumnKind::Field(TrackField::Genre) => "genre",
      ColumnKind::Field(TrackField::Filename) => "file",
      ColumnKind::Field(TrackField::Lyrics) => "lyrics",
      ColumnKind::Duration => "time",
    }
  }
}

pub(super) fn draw_library_pane(frame: &mut Frame, app: &mut App, area: Rect) {
  let theme = &app.settings.theme;
  let is_main = app.main_pane() == PaneKind::Library;
  let title = match app.library_scanning {
    Some((scanned, changed)) => format!("library scanning {scanned} (+{changed})"),
    None => match app.library_filter.as_deref() {
      Some(filter) => format!(
        "library {}/{} · /{filter}",
        app.library_rows.len(),
        app.library.len()
      ),
      None => format!("library ({})", app.library.len()),
    },
  };
  let block = pane_block(app, &title, is_main);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.height == 0 || inner.width == 0 {
    return;
  }

  // Data viewport sits below the header row; mouse row mapping and the
  // scrollbar both key off this rect.
  let viewport = Rect {
    x: inner.x,
    y: inner.y + HEADER_ROWS.min(inner.height),
    width: inner.width.saturating_sub(1),
    height: inner.height.saturating_sub(HEADER_ROWS),
  };
  if viewport.height == 0 {
    return;
  }
  app.library_pane_areas.push(viewport);

  if app.library.is_empty() {
    let hint = if app.library_scanning.is_some() {
      "scanning…".to_string()
    } else if app.library_scan_tx.is_some() {
      "library is empty — press u to rescan".to_string()
    } else {
      "library not configured — set [library] paths in config.toml".to_string()
    };
    frame.render_widget(
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.muted))),
      inner,
    );
    return;
  }
  if app.library_rows.is_empty() {
    let hint = format!(
      "no matches for /{}",
      app.library_filter.as_deref().unwrap_or_default()
    );
    frame.render_widget(
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.muted))),
      inner,
    );
    return;
  }

  let columns = display_columns(app);
  let widths = column_widths(&columns, viewport.width);
  let selected = app.library_state.selected();

  // Currently playing song path (to mark the row like the queue does).
  let playing_path = app
    .current_song_url()
    .and_then(|url| app.music_dir.as_ref().map(|dir| crate::library::uri_to_path(dir, &url)))
    .or_else(|| {
      app
        .current_song_url()
        .as_deref()
        .and_then(crate::library::file_uri_to_path)
    });

  let header = Row::new(
    std::iter::once(Cell::from(""))
      .chain(columns.iter().map(|column| {
        Cell::from(column.label()).style(
          Style::default()
            .fg(theme.color(&theme.border))
            .add_modifier(Modifier::BOLD),
        )
      }))
      .collect::<Vec<_>>(),
  )
  .height(1)
  .bottom_margin(1);

  let rows = app
    .library_rows
    .iter()
    .enumerate()
    .map(|(row, matched)| {
      library_row(
        app,
        matched,
        &columns,
        &widths,
        selected == Some(row),
        playing_path.as_deref(),
      )
    })
    .collect::<Vec<_>>();

  let constraints = std::iter::once(ratatui::layout::Constraint::Length(2))
    .chain(widths.iter().map(|width| {
      ratatui::layout::Constraint::Length(*width)
    }))
    .collect::<Vec<_>>();

  let table = Table::new(rows, constraints)
    .header(header)
    .column_spacing(1);
  frame.render_stateful_widget(table, inner, &mut app.library_state);

  // Viewport scrollbar (offset + size), draggable via the mouse.
  let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
    .style(Style::default().fg(theme.color(&theme.border)));
  let mut state = ratatui::widgets::ScrollbarState::new(app.library_rows.len())
    .position(app.library_state.offset())
    .viewport_content_length(viewport.height as usize);
  frame.render_stateful_widget(scrollbar, area, &mut state);
  app.library_bar_areas.push(Rect {
    x: area.x + area.width.saturating_sub(1),
    y: area.y,
    width: 1,
    height: area.height,
  });
}

fn display_columns(app: &App) -> Vec<DisplayColumn> {
  let configured = &app.library_columns;
  let columns: Vec<DisplayColumn> = configured
    .iter()
    .filter_map(|column| {
      let kind = if column.field.trim() == "duration" {
        ColumnKind::Duration
      } else {
        ColumnKind::Field(TrackField::parse(&column.field)?)
      };
      Some(DisplayColumn {
        weight: column.width.max(1),
        kind,
      })
    })
    .collect();
  if columns.is_empty() {
    vec![DisplayColumn {
      weight: 1,
      kind: ColumnKind::Field(TrackField::Title),
    }]
  } else {
    columns
  }
}

/// Divide `available` cells between columns by weight. The leading
/// playing-marker column (2 cells) and the single gap between each
/// column are reserved first.
fn column_widths(columns: &[DisplayColumn], available: u16) -> Vec<u16> {
  let total: u32 = columns.iter().map(|column| column.weight).sum();
  let count = columns.len() as u16;
  let gaps = count; // one gap after the marker and between each column
  let budget = available.saturating_sub(2 + gaps).max(count);
  let mut widths: Vec<u16> = columns
    .iter()
    .map(|column| {
      ((u32::from(budget) * column.weight / total.max(1)) as u16).clamp(1, budget)
    })
    .collect();
  // Hand out the remainder left to right.
  let used: u16 = widths.iter().sum();
  let mut remainder = budget.saturating_sub(used);
  for width in widths.iter_mut() {
    if remainder == 0 {
      break;
    }
    *width += 1;
    remainder -= 1;
  }
  widths
}

/// Per-field text color, calibre-tui style: a couple of quiet accent
/// tones so columns read apart at a glance.
fn field_color(field: TrackField, theme: &crate::theme::ThemeConfig) -> ratatui::style::Color {
  let name = match field {
    TrackField::Title | TrackField::Album | TrackField::Filename => &theme.foreground,
    TrackField::Artist | TrackField::Genre | TrackField::Lyrics => &theme.accent_alt,
  };
  theme.color(name)
}

fn library_row(
  app: &App,
  matched: &crate::library_db::TrackMatch,
  columns: &[DisplayColumn],
  widths: &[u16],
  is_selected: bool,
  playing_path: Option<&std::path::Path>,
) -> Row<'static> {
  let theme = &app.settings.theme;
  let track = &matched.track;
  let filter_active = app.library_filter.is_some();

  // Inverted hover bar like calibre-tui's row highlight.
  let base_bg = if is_selected {
    theme.color(&theme.accent)
  } else {
    theme.color(&theme.background)
  };

  let marker = if playing_path == Some(track.path.as_path()) {
    match app.status.as_ref().map(|status| status.state) {
      Some(PlayState::Playing) => Span::styled(
        "▶ ",
        Style::default().fg(theme.color(&theme.playing)).bg(base_bg),
      ),
      Some(PlayState::Paused) => Span::styled(
        "⏸ ",
        Style::default().fg(theme.color(&theme.paused)).bg(base_bg),
      ),
      _ => Span::raw("  "),
    }
  } else {
    Span::raw("  ")
  };

  let mut cells = vec![Cell::from(Line::from(marker))];
  for (column, width) in columns.iter().zip(widths.iter()) {
    let width = usize::from(*width).max(1);
    let cell = match column.kind {
      ColumnKind::Duration => {
        let label = format_duration_line(Duration::from_secs_f64(track.duration_secs.max(0.0)));
        let pad = width.saturating_sub(label.chars().count());
        Cell::from(Line::from(Span::styled(
          format!("{}{label}", " ".repeat(pad)),
          Style::default().fg(theme.color(&theme.muted)).bg(base_bg),
        )))
      }
      ColumnKind::Field(field) => {
        let text = field.text(track);
        let is_match_field = filter_active && matched.field == field;
        let (window_start, range) = if is_match_field {
          match_window(text, matched.range, width)
        } else {
          (0, None)
        };
        let window: String = text.chars().skip(window_start).take(width).collect();
        let base = Style::default().fg(field_color(field, theme)).bg(base_bg);
        let highlight = Style::default()
          .fg(theme.color(&theme.library_highlight))
          .bg(base_bg)
          .add_modifier(Modifier::BOLD);
        Cell::from(Line::from(highlighted_spans(
          &window,
          text,
          window_start,
          range,
          base,
          highlight,
        )))
      }
    };
    cells.push(cell);
  }
  Row::new(cells).height(1)
}

/// Split the visible window into plain/highlighted spans. `range` holds
/// byte offsets into the full `text`; they are shifted by the window
/// start first.
fn highlighted_spans(
  window: &str,
  text: &str,
  window_start: usize,
  range: Option<(usize, usize)>,
  base: Style,
  highlight: Style,
) -> Vec<Span<'static>> {
  let Some((start, end)) = range else {
    return vec![Span::styled(window.to_string(), base)];
  };
  // Byte offset of the visible window inside `text`.
  let window_bytes: usize = text.chars().take(window_start).map(char::len_utf8).sum();
  let start = start.saturating_sub(window_bytes);
  let end = end.saturating_sub(window_bytes).min(window.len());
  if start < end && end <= window.len() {
    let split_start = char_boundary_index(window, start);
    let split_end = char_boundary_index(window, end);
    vec![
      Span::styled(window[..split_start].to_string(), base),
      Span::styled(window[split_start..split_end].to_string(), highlight),
      Span::styled(window[split_end..].to_string(), base),
    ]
  } else {
    vec![Span::styled(window.to_string(), base)]
  }
}

/// Pick a horizontal window for long field values so the match sits
/// visibly inside it. Returns the char offset to start drawing at plus
/// the match range (kept in full-text byte coordinates).
fn match_window(text: &str, range: (usize, usize), budget: usize) -> (usize, Option<(usize, usize)>) {
  let char_count = text.chars().count();
  if budget == 0 || char_count <= budget {
    return (0, Some(range));
  }
  // Char offset of the match start.
  let start_char = text
    .char_indices()
    .take_while(|(byte, _)| *byte < range.0)
    .count();
  let lead = budget / 3;
  let window_start = start_char.saturating_sub(lead).min(char_count - budget);
  (window_start, Some(range))
}

/// Largest char boundary <= `index` in `text`.
fn char_boundary_index(text: &str, index: usize) -> usize {
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
