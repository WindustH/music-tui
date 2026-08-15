//! Lyrics pane rendering (synced/plain, karaoke highlight).

use super::*;

pub(super) fn draw_lyrics_pane(frame: &mut Frame, app: &mut App, area: Rect, source: PaneSource) {
  if source == PaneSource::Hovered {
    draw_hover_lyrics_pane(frame, app, area);
    return;
  }
  let theme = &app.settings.theme;
  let is_main = app.main_pane() == PaneKind::Lyrics;
  let title = if app.lyrics_follow {
    "lyrics (follow)"
  } else {
    "lyrics (manual · enter jump)"
  };
  let block = pane_block(app, title, is_main);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.height == 0 || inner.width == 0 {
    return;
  }

  let Some(lyrics) = app.lyrics.as_ref() else {
    let hint = app
      .lyrics_error
      .clone()
      .unwrap_or_else(|| "no lyrics".to_string());
    frame.render_widget(
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.muted))),
      inner,
    );
    return;
  };

  let elapsed = Duration::from_secs_f64(app.elapsed());
  let active = lyrics.active_index(elapsed);
  let karaoke = lyrics.karaoke(elapsed);
  let cursor = (!app.lyrics_follow).then_some(app.lyrics_cursor).flatten();

  // Scrolling: follow centers the active line; manual keeps the stored
  // viewport offset and only adjusts it to keep the pointer visible
  // (viewport is the source of truth, the pointer passively follows).
  let scroll = if app.lyrics_follow {
    active
      .map(|active| active.saturating_sub(inner.height as usize / 2))
      .unwrap_or(app.lyrics_scroll)
  } else {
    let mut scroll = app.lyrics_scroll;
    if let Some(cursor) = cursor
      && cursor < scroll
    {
      scroll = cursor;
    } else if let Some(cursor) = cursor
      && cursor >= scroll + inner.height as usize
    {
      scroll = cursor + 1 - inner.height as usize;
    }
    scroll
  };
  app.lyrics_scroll = scroll;
  app.lyrics_pane_areas.push(inner);
  app.lyrics_pane_sources.push(source);

  let line_count = lyrics.line_count();
  let mut lines: Vec<Line> = Vec::new();
  for row in 0..inner.height as usize {
    let index = scroll + row;
    if index >= line_count {
      break;
    }
    let is_active = active == Some(index);
    let is_cursor = cursor == Some(index);
    let mut spans: Vec<Span> = Vec::new();
    if is_cursor {
      spans.push(Span::styled(
        "❯ ",
        Style::default()
          .fg(theme.color(&theme.accent))
          .add_modifier(Modifier::BOLD),
      ));
    } else {
      spans.push(Span::raw("  "));
    }

    let text = lyrics.line(index).unwrap_or_default();
    let sung = if is_active { karaoke.map(|(_, sung)| sung).unwrap_or(0) } else { 0 };
    if is_active && sung > 0 {
      // Karaoke: sung prefix highlighted, remainder in the base style.
      let chars: Vec<char> = text.chars().collect();
      let split = sung.min(chars.len());
      spans.push(Span::styled(
        chars[..split].iter().collect::<String>(),
        Style::default()
          .fg(theme.color(&theme.lyrics_active))
          .add_modifier(Modifier::BOLD),
      ));
      spans.push(Span::styled(
        chars[split..].iter().collect::<String>(),
        Style::default().fg(theme.color(&theme.foreground)),
      ));
    } else {
      let style = if is_active {
        Style::default()
          .fg(theme.color(&theme.foreground))
          .add_modifier(Modifier::BOLD)
      } else {
        Style::default().fg(theme.color(&theme.muted))
      };
      spans.push(Span::styled(text.to_string(), style));
    }
    lines.push(Line::from(spans));
  }
  frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// Lyrics for the hovered song: no playback state, so no sync highlight,
/// no follow, no cursor — a plain scrollable list (wheel / j-k style keys).
fn draw_hover_lyrics_pane(frame: &mut Frame, app: &mut App, area: Rect) {
  let theme = &app.settings.theme;
  let is_main = app.main_pane() == PaneKind::Lyrics;
  let title = match &app.hover {
    Some(hover) => format!("lyrics · {}", hover.title),
    None => "lyrics (hovered)".to_string(),
  };
  let block = pane_block(app, &title, is_main);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.height == 0 || inner.width == 0 {
    return;
  }
  let Some(hover) = app.hover.as_mut() else {
    let hint = "hover a queue entry";
    frame.render_widget(
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.muted))),
      inner,
    );
    return;
  };
  let Some(lyrics) = hover.lyrics.as_ref() else {
    let hint = hover
      .lyrics_error
      .clone()
      .unwrap_or_else(|| "no lyrics".to_string());
    frame.render_widget(
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.muted))),
      inner,
    );
    return;
  };
  let line_count = lyrics.line_count();
  let scroll = hover.lyrics_scroll;
  let mut lines: Vec<Line> = Vec::new();
  for row in 0..inner.height as usize {
    let index = scroll + row;
    if index >= line_count {
      break;
    }
    let text = lyrics.line(index).unwrap_or_default();
    lines.push(Line::styled(
      text.to_string(),
      Style::default().fg(theme.color(&theme.foreground)),
    ));
  }
  frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
