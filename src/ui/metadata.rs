//! Metadata pane rendering.

use super::*;

pub(super) fn draw_metadata_pane(frame: &mut Frame, app: &mut App, area: Rect, source: PaneSource) {
  if matches!(
    source,
    PaneSource::QueueHovered | PaneSource::LibraryHovered
  ) {
    draw_hover_metadata_pane(frame, app, area, source);
    return;
  }
  let theme = &app.settings.theme;
  let is_main = app.main_pane() == PaneKind::Metadata;
  let title = "metadata";
  let block = pane_block(app, title, is_main);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.height == 0 {
    return;
  }

  let Some(entries) = app.metadata_entries.as_ref() else {
    let hint = app
      .metadata_error
      .clone()
      .unwrap_or_else(|| "nothing playing".to_string());
    frame.render_widget(
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.base.muted))),
      inner,
    );
    return;
  };

  let lines: Vec<Line> = entries
    .iter()
    .skip(app.metadata_scroll)
    .map(|entry| metadata_line(app, &entry.name, &entry.value))
    .collect();
  frame.render_widget(Paragraph::new(lines), inner);
}

pub(super) fn metadata_line(app: &App, name: &str, value: &str) -> Line<'static> {
  let theme = &app.settings.theme;
  let mut label = format!("{name}:");
  let pad = 16usize.saturating_sub(label.chars().count());
  label.push_str(&" ".repeat(pad));
  Line::from(vec![
    Span::styled(
      label,
      Style::default()
        .fg(theme.color(&theme.base.accent))
        .add_modifier(Modifier::BOLD),
    ),
    Span::styled(
      value.to_string(),
      Style::default().fg(theme.color(&theme.base.foreground)),
    ),
  ])
}

/// Metadata of the hovered row (queue or library, per the pane source).
fn draw_hover_metadata_pane(frame: &mut Frame, app: &mut App, area: Rect, source: PaneSource) {
  let theme = &app.settings.theme;
  let is_main = app.main_pane() == PaneKind::Metadata;
  let title = match app.hover_view(source) {
    Some(hover) => format!("metadata · {}", hover.title),
    None => "metadata (hovered)".to_string(),
  };
  let block = pane_block(app, &title, is_main);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.height == 0 {
    return;
  }
  let Some(hover) = app.hover_view(source) else {
    frame.render_widget(
      Paragraph::new("hover a queue or library entry")
        .style(Style::default().fg(theme.color(&theme.base.muted))),
      inner,
    );
    return;
  };
  let Some(entries) = hover.metadata.as_ref() else {
    let hint = hover
      .metadata_error
      .clone()
      .unwrap_or_else(|| "reading…".to_string());
    frame.render_widget(
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.base.muted))),
      inner,
    );
    return;
  };
  let lines: Vec<Line> = entries
    .iter()
    .skip(hover.metadata_scroll)
    .map(|entry| metadata_line(app, &entry.name, &entry.value))
    .collect();
  frame.render_widget(Paragraph::new(lines), inner);
}
