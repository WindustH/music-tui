//! Queue pane rendering.

use super::*;
use crate::strip::StrippedText;

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
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.base.muted))),
      inner,
    );
    return;
  }
  if app.queue_filter_matches.is_empty() {
    let hint = format!("no matches for /{}", app.queue_filter.as_deref().unwrap_or_default());
    frame.render_widget(
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.base.muted))),
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
      .fg(theme.color(&theme.queue.selection))
      .add_modifier(Modifier::BOLD),
  );
  frame.render_stateful_widget(list, inner, &mut app.queue_state);

  // The scrollbar mirrors the viewport (offset + size), not the selection,
  // and doubles as a mouse drag target.
  let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
    .style(Style::default().fg(theme.color(&theme.base.border)));
  let mut state = ratatui::widgets::ScrollbarState::new(app.queue_filter_matches.len())
    .position(app.queue_state.offset())
    .viewport_content_length(inner.height as usize);
  frame.render_stateful_widget(scrollbar, area, &mut state);
  // Scrollbar renders over the pane's last column; record the exact track
  // (full pane height) for mouse hit tests.
  app.queue_bar_areas.push(Rect {
    x: area.x + area.width.saturating_sub(1),
    y: area.y,
    width: 1,
    height: area.height,
  });
}

fn queue_line(app: &App, index: usize, song: &SongInQueue, playing: Option<usize>) -> Line<'static> {
  let theme = &app.settings.theme;
  let title = song_title(&song.song)
    .map(str::to_string)
    .unwrap_or_else(|| song.song.url.clone());
  let artist = song_artist(&song.song).map(str::to_string).unwrap_or_default();
  let marker = if playing == Some(index) {
    match app.status.as_ref().map(|status| status.state) {
      Some(PlayState::Playing) => Span::styled("▶ ", Style::default().fg(theme.color(&theme.queue.playing))),
      Some(PlayState::Paused) => Span::styled("⏸ ", Style::default().fg(theme.color(&theme.queue.paused))),
      _ => Span::raw("  "),
    }
  } else {
    Span::raw("  ")
  };
  // Space-separated terms; each one matches with spaces ignored inside
  // the field text. Every matched term gets highlighted.
  let terms: Vec<&str> = app
    .queue_filter
    .as_deref()
    .map(|filter| filter.split_whitespace().collect())
    .unwrap_or_default();
  let base = Style::default().fg(theme.color(&theme.base.foreground));
  let muted = Style::default().fg(theme.color(&theme.base.muted));
  let highlight = Style::default()
    .fg(theme.color(&theme.queue.highlight))
    .add_modifier(Modifier::BOLD);
  let mut spans = vec![marker];
  if terms.is_empty() {
    let text = if artist.is_empty() {
      title
    } else {
      format!("{title} — {artist}")
    };
    spans.push(Span::styled(text, base));
  } else {
    let title_text = StrippedText::new(&title);
    let title_ranges: Vec<(usize, usize)> = terms
      .iter()
      .flat_map(|term| title_text.find_all(term))
      .collect();
    spans.extend(highlighted_ranges_spans(
      &title, &title, 0, title_ranges, base, highlight,
    ));
    if !artist.is_empty() {
      let artist_text = StrippedText::new(&artist);
      let artist_ranges: Vec<(usize, usize)> = terms
        .iter()
        .flat_map(|term| artist_text.find_all(term))
        .collect();
      spans.push(Span::styled(" — ", base));
      spans.extend(highlighted_ranges_spans(
        &artist, &artist, 0, artist_ranges, base, highlight,
      ));
    }
    // Terms that only match the album or the URL still need a visible
    // highlight, so append those fields when they contain a match.
    if let Some(album) = song_album(&song.song) {
      let album_text = StrippedText::new(album);
      let ranges: Vec<(usize, usize)> = terms
        .iter()
        .flat_map(|term| album_text.find_all(term))
        .collect();
      if !ranges.is_empty() {
        spans.push(Span::styled(" · ", muted));
        spans.extend(highlighted_ranges_spans(album, album, 0, ranges, muted, highlight));
      }
    }
    let url_text = StrippedText::new(&song.song.url);
    let url_ranges: Vec<(usize, usize)> = terms
      .iter()
      .flat_map(|term| url_text.find_all(term))
      .collect();
    if !url_ranges.is_empty() {
      spans.push(Span::styled(" ⟨", muted));
      spans.extend(highlighted_ranges_spans(
        &song.song.url, &song.song.url, 0, url_ranges, muted, highlight,
      ));
      spans.push(Span::styled("⟩", muted));
    }
  }
  let duration = song
    .song
    .duration
    .map(format_duration_line)
    .unwrap_or_default();
  spans.push(Span::raw(" "));
  spans.push(Span::styled(duration, muted));
  Line::from(spans)
}
