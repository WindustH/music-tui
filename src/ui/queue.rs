//! Queue pane rendering.

use super::*;

pub(super) fn draw_queue_pane(frame: &mut Frame, app: &mut App, area: Rect) {
  let theme = &app.settings.theme;
  let is_main = app.main_pane() == PaneKind::Queue;
  let title = match app.queue_filter.as_deref() {
    Some(filter) => format!("queue {}/{} · /{filter}", app.queue_filter_matches.len(), app.queue.len()),
    None => format!("queue ({})", app.queue.len()),
  };
  let block = pane_block(app, &title, is_main);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.height == 0 || inner.width == 0 {
    return;
  }
  app.queue_pane_areas.push(inner);

  if app.queue.is_empty() {
    let hint = if app.connection_error.is_some() {
      format!("mpd connection lost: {}", app.connection_error.as_deref().unwrap_or_default())
    } else if app.connected.is_some() {
      "queue is empty — try `music-tui open <path>` or :add <path>".to_string()
    } else {
      "connecting to mpd…".to_string()
    };
    frame.render_widget(
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.muted))),
      inner,
    );
    return;
  }
  if app.queue_filter_matches.is_empty() {
    let hint = format!("no matches for /{}", app.queue_filter.as_deref().unwrap_or_default());
    frame.render_widget(
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.muted))),
      inner,
    );
    return;
  }

  let (position, _) = app
    .status
    .as_ref()
    .and_then(|status| status.current_song)
    .unzip();
  let playing = position.map(|pos| pos.0);

  let items: Vec<ListItem> = app
    .queue_filter_matches
    .iter()
    .filter_map(|position| app.queue.get(*position).map(|song| (position, song)))
    .map(|(position, song)| ListItem::new(queue_line(app, *position, song, playing)))
    .collect();

  let list = List::new(items).highlight_style(
    Style::default()
      .fg(theme.color(&theme.accent))
      .add_modifier(Modifier::BOLD),
  );
  frame.render_stateful_widget(list, inner, &mut app.queue_state);

  let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
    .style(Style::default().fg(theme.color(&theme.border)));
  let mut state = ratatui::widgets::ScrollbarState::new(app.queue_filter_matches.len())
    .position(app.queue_state.selected().unwrap_or(0));
  frame.render_stateful_widget(scrollbar, area, &mut state);
}

fn queue_line(app: &App, index: usize, song: &SongInQueue, playing: Option<usize>) -> Line<'static> {
  let theme = &app.settings.theme;
  let title = song_title(&song.song)
    .map(str::to_string)
    .unwrap_or_else(|| song.song.url.clone());
  let artist = song_artist(&song.song).map(str::to_string).unwrap_or_default();
  let marker = if playing == Some(index) {
    match app.status.as_ref().map(|status| status.state) {
      Some(PlayState::Playing) => Span::styled("▶ ", Style::default().fg(theme.color(&theme.playing))),
      Some(PlayState::Paused) => Span::styled("⏸ ", Style::default().fg(theme.color(&theme.paused))),
      _ => Span::raw("  "),
    }
  } else {
    Span::raw("  ")
  };
  let main = if artist.is_empty() {
    Span::styled(title, Style::default().fg(theme.color(&theme.foreground)))
  } else {
    Span::styled(
      format!("{title} — {artist}"),
      Style::default().fg(theme.color(&theme.foreground)),
    )
  };
  let duration = song
    .song
    .duration
    .map(format_duration_line)
    .unwrap_or_default();
  Line::from(vec![marker, main, Span::raw(" "), Span::styled(duration, Style::default().fg(theme.color(&theme.muted)))])
}
