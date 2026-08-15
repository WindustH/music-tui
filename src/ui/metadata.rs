//! Metadata pane rendering.

use super::*;

pub(super) fn draw_metadata_pane(frame: &mut Frame, app: &mut App, area: Rect) {
  let theme = &app.settings.theme;
  let is_main = app.main_pane() == PaneKind::Metadata;
  let title = if is_main { "metadata (e edit)" } else { "metadata" };
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
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.muted))),
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
        .fg(theme.color(&theme.accent))
        .add_modifier(Modifier::BOLD),
    ),
    Span::styled(value.to_string(), Style::default().fg(theme.color(&theme.foreground))),
  ])
}
