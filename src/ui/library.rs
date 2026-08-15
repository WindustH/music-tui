//! Library pane rendering: configurable columns, filter highlight and a
//! viewport scrollbar (drag with the mouse).

use super::*;
use crate::library_db::TrackField;

/// A resolved display column (parsed from `[library] columns`).
struct DisplayColumn {
  weight: u32,
  kind: ColumnKind,
}

enum ColumnKind {
  Field(TrackField),
  Duration,
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
  app.library_pane_areas.push(inner);

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
  let widths = column_widths(&columns, inner.width.saturating_sub(1));
  let selected = app.library_state.selected();

  // Currently playing song path (to mark the row like the queue does).
  let playing_path = app
    .current_song_url()
    .and_then(|url| app.music_dir.as_ref().map(|dir| crate::library::uri_to_path(dir, &url)))
    .or_else(|| {
      app.current_song_url()
        .as_deref()
        .and_then(crate::library::file_uri_to_path)
    });

  let items: Vec<ListItem> = app
    .library_rows
    .iter()
    .enumerate()
    .skip(app.library_state.offset())
    .take(inner.height as usize)
    .map(|(row, matched)| {
      ListItem::new(library_line(
        app,
        row,
        matched,
        &columns,
        &widths,
        selected == Some(row),
        playing_path.as_deref(),
      ))
    })
    .collect();

  let list = List::new(items).highlight_style(
    Style::default()
      .fg(theme.color(&theme.accent))
      .add_modifier(Modifier::BOLD),
  );
  frame.render_stateful_widget(list, inner, &mut app.library_state);

  // Viewport scrollbar (offset + size), draggable via the mouse.
  let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
    .style(Style::default().fg(theme.color(&theme.border)));
  let mut state = ratatui::widgets::ScrollbarState::new(app.library_rows.len())
    .position(app.library_state.offset())
    .viewport_content_length(inner.height as usize);
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

/// Divide `available` cells between columns by weight. The playing marker
/// column takes 2 cells from the first field column.
fn column_widths(columns: &[DisplayColumn], available: u16) -> Vec<u16> {
  let total: u32 = columns.iter().map(|column| column.weight).sum();
  let count = columns.len() as u16;
  // One gap between each pair of columns; leave 1 spare cell.
  let gaps = count.saturating_sub(1);
  let budget = available.saturating_sub(gaps).max(count);
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

#[allow(clippy::too_many_arguments)]
fn library_line(
  app: &App,
  _row: usize,
  matched: &crate::library_db::TrackMatch,
  columns: &[DisplayColumn],
  widths: &[u16],
  is_selected: bool,
  playing_path: Option<&std::path::Path>,
) -> Line<'static> {
  let theme = &app.settings.theme;
  let track = &matched.track;
  let filter_active = app.library_filter.is_some();

  let marker = if playing_path == Some(track.path.as_path()) {
    match app.status.as_ref().map(|status| status.state) {
      Some(PlayState::Playing) => Span::styled(
        "▶ ",
        Style::default().fg(theme.color(&theme.playing)),
      ),
      Some(PlayState::Paused) => Span::styled(
        "⏸ ",
        Style::default().fg(theme.color(&theme.paused)),
      ),
      _ => Span::raw("  "),
    }
  } else {
    Span::raw("  ")
  };

  let mut spans = vec![marker];
  let mut first = true;
  for (column, width) in columns.iter().zip(widths.iter()) {
    if !first {
      spans.push(Span::raw(" "));
    }
    first = false;
    let width = usize::from(*width).max(1);
    match column.kind {
      ColumnKind::Duration => {
        let label = format_duration_line(Duration::from_secs_f64(track.duration_secs.max(0.0)));
        let pad = width.saturating_sub(label.chars().count());
        spans.push(Span::styled(
          format!("{}{label}", " ".repeat(pad)),
          Style::default().fg(theme.color(&theme.muted)),
        ));
      }
      ColumnKind::Field(field) => {
        let text = field.text(track);
        // The first column shares its width with the playing marker.
        let budget = if spans.len() == 1 { width.saturating_sub(2) } else { width };
        let is_match_field = filter_active && matched.field == field;
        let (window_start, range) = if is_match_field {
          match_window(text, matched.range, budget)
        } else {
          (0, None)
        };
        let base = Style::default().fg(if is_selected {
          theme.color(&theme.accent)
        } else {
          theme.color(&theme.foreground)
        });
        let window: String = text
          .chars()
          .skip(window_start)
          .take(budget)
          .collect();
        if let Some((start, end)) = range {
          // Byte offsets of the visible window inside `text`.
          let window_bytes: usize = text
            .chars()
            .take(window_start)
            .map(char::len_utf8)
            .sum();
          let start = start.saturating_sub(window_bytes);
          let end = end.saturating_sub(window_bytes).min(window.len());
          if start < end && end <= window.len() {
            let split_start = char_boundary_index(&window, start);
            let split_end = char_boundary_index(&window, end);
            spans.push(Span::styled(
              window[..split_start].to_string(),
              base,
            ));
            spans.push(Span::styled(
              window[split_start..split_end].to_string(),
              base
                .fg(theme.color(&theme.accent_alt))
                .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
              window[split_end..].to_string(),
              base,
            ));
            continue;
          }
        }
        spans.push(Span::styled(window, base));
      }
    }
  }
  Line::from(spans)
}

/// Pick a horizontal window for long field values so the match sits
/// visibly inside it. Returns the char offset to start drawing at plus
/// the match range adjusted to that window (unchanged bytes when the
/// match is already visible).
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
