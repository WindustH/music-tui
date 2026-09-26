//! Metadata pane rendering.

use super::*;
use crate::app::SongView;

/// Metadata pane for any data source (playing song or a hovered row).
pub(super) fn draw_metadata_pane(frame: &mut Frame, app: &mut App, area: Rect, source: PaneSource) {
  let theme = &app.settings.theme;
  let is_main = app.main_pane() == PaneKind::Metadata;
  let view = app.song_view(source);
  let title = match (source, view) {
    (PaneSource::Playing, _) => "metadata".to_string(),
    (_, Some(view)) => format!("metadata · {}", view.title),
    (_, None) => "metadata (hovered)".to_string(),
  };
  let block = pane_block(app, &title, is_main);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.height == 0 {
    return;
  }
  let Some(view) = view else {
    let hint = match source {
      PaneSource::Playing if app.current_song().is_some() => "no local file for this song",
      PaneSource::Playing => "nothing playing",
      _ => "hover a queue or library entry",
    };
    frame.render_widget(
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.base.muted))),
      inner,
    );
    return;
  };
  draw_metadata_entries(frame, app, view, inner, "reading metadata…");
}

/// A song view's metadata entries (scrolled), or its load state.
pub(super) fn draw_metadata_entries(
  frame: &mut Frame,
  app: &App,
  view: &SongView,
  area: Rect,
  loading_hint: &str,
) {
  let theme = &app.settings.theme;
  let Some(entries) = view.metadata.as_ref() else {
    let hint = view.metadata_error.as_deref().unwrap_or(loading_hint);
    frame.render_widget(
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.base.muted))),
      area,
    );
    return;
  };
  let lines: Vec<Line> = entries
    .iter()
    .skip(view.metadata_scroll)
    .take(usize::from(area.height))
    .map(|entry| metadata_line(app, &entry.name, &entry.value))
    .collect();
  frame.render_widget(Paragraph::new(lines), area);
}

fn metadata_line(app: &App, name: &str, value: &str) -> Line<'static> {
  let theme = &app.settings.theme;
  let value = crate::sanitize::sanitize_text(value);
  let mut label = format!("{name}:");
  let pad = 16usize.saturating_sub(label.chars().count());
  label.push_str(&" ".repeat(pad));
  Line::from(vec![
    Span::styled(
      label,
      Style::default()
        .fg(theme.color(&theme.metadata.label))
        .add_modifier(Modifier::BOLD),
    ),
    Span::styled(
      value,
      Style::default().fg(theme.color(&theme.base.foreground)),
    ),
  ])
}
