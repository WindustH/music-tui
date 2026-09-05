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
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.base.muted))),
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
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.base.muted))),
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
    .and_then(|url| crate::library::uri_to_path(app.music_dir.as_deref(), &url));

  let header = Row::new(
    std::iter::once(Cell::from(""))
      .chain(columns.iter().map(|column| {
        Cell::from(column.label()).style(
          Style::default()
            .fg(theme.color(&theme.base.border))
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
    .chain(
      widths
        .iter()
        .map(|width| ratatui::layout::Constraint::Length(*width)),
    )
    .collect::<Vec<_>>();

  let table = Table::new(rows, constraints)
    .header(header)
    .column_spacing(1);
  frame.render_stateful_widget(table, inner, &mut app.library_state);

  // Viewport scrollbar (offset + size), draggable via the mouse.
  let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
    .style(Style::default().fg(theme.color(&theme.base.border)));
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
    .map(|column| ((u32::from(budget) * column.weight / total.max(1)) as u16).clamp(1, budget))
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
    TrackField::Title | TrackField::Album | TrackField::Filename => &theme.base.foreground,
    TrackField::Artist | TrackField::Genre | TrackField::Lyrics => &theme.base.accent_alt,
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

  // Full-row hover bar: the Row/Cell styles paint the background across
  // the entire row (cells + gaps); spans only set foreground colors so
  // they cannot punch holes in the bar.
  let base_bg = theme.base_background();
let row_style = if is_selected {
Style::default()
.fg(theme.color(&theme.library.selection_foreground))
.bg(theme.color(&theme.library.selection_background))
} else if base_bg != ratatui::style::Color::Reset {
Style::default()
.fg(theme.color(&theme.base.foreground))
.bg(base_bg)
} else {
Style::default().fg(theme.color(&theme.base.foreground))
};
  let plain_fg = if is_selected {
    theme.color(&theme.library.selection_foreground)
  } else {
    theme.color(&theme.base.foreground)
  };

  let marker_fg = if is_selected {
    plain_fg
  } else if playing_path == Some(track.path.as_path()) {
    match app.status.as_ref().map(|status| status.state) {
      Some(PlayState::Playing) => theme.color(&theme.footer.playing),
      Some(PlayState::Paused) => theme.color(&theme.footer.paused),
      _ => plain_fg,
    }
  } else {
    plain_fg
  };
  let marker = if playing_path == Some(track.path.as_path()) {
    match app.status.as_ref().map(|status| status.state) {
      Some(PlayState::Playing) => Span::styled("▶ ", Style::default().fg(marker_fg)),
      Some(PlayState::Paused) => Span::styled("⏸ ", Style::default().fg(marker_fg)),
      _ => Span::raw("  "),
    }
  } else {
    Span::raw("  ")
  };

  let mut cells = vec![Cell::from(Line::from(marker)).style(row_style)];
  for (column, width) in columns.iter().zip(widths.iter()) {
    let width = usize::from(*width).max(1);
    let cell = match column.kind {
      ColumnKind::Duration => {
        let label = format_duration_line(Duration::from_secs_f64(track.duration_secs.max(0.0)));
        let pad = width.saturating_sub(label.chars().count());
        let fg = if is_selected {
          plain_fg
        } else {
          theme.color(&theme.base.muted)
        };
        Cell::from(Line::from(Span::styled(
          format!("{}{label}", " ".repeat(pad)),
          Style::default().fg(fg),
        )))
        .style(row_style)
      }
      ColumnKind::Field(field) => {
        let text = field.text(track);
        // Every column highlights all term matches it contains (spaces
        // inside the text are ignored when matching).
        let terms: Vec<&str> = app
          .library_filter
          .as_deref()
          .map(|filter| filter.split_whitespace().collect())
          .unwrap_or_default();
        let stripped = StrippedText::new(text);
        let ranges: Vec<(usize, usize)> = terms
          .iter()
          .flat_map(|term| stripped.find_all(term))
          .collect();
        // Long fields scroll so matches stay visible; the window is
        // measured in display columns and anchored on the leftmost match.
        let (window, window_start) = filter_window(text, &ranges, width);
        let base = if is_selected {
          Style::default().fg(plain_fg)
        } else {
          Style::default().fg(field_color(field, theme))
        };
        let highlight = Style::default()
          .fg(theme.color(&theme.library.highlight))
          .add_modifier(Modifier::BOLD);
        Cell::from(Line::from(highlighted_ranges_spans(
          &window,
          text,
          window_start,
          ranges,
          base,
          highlight,
        )))
        .style(row_style)
      }
    };
    cells.push(cell);
  }
  Row::new(cells).height(1).style(row_style)
}
