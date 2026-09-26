//! Lyrics pane rendering (synced/plain, karaoke highlight).

use super::*;

pub(super) fn draw_lyrics_pane(frame: &mut Frame, app: &mut App, area: Rect, source: PaneSource) {
  if source != PaneSource::Playing {
    draw_hover_lyrics_pane(frame, app, area, source);
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

  let Some(playing) = app.playing.as_ref() else {
    let hint = if app.current_song().is_some() {
      "no local file for this song"
    } else {
      "nothing playing"
    };
    frame.render_widget(
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.base.muted))),
      inner,
    );
    return;
  };
  let Some(lyrics) = playing.lyrics.as_ref() else {
    let hint = playing.lyrics_error.as_deref().unwrap_or("loading lyrics…");
    frame.render_widget(
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.base.muted))),
      inner,
    );
    return;
  };

  let elapsed = Duration::from_secs_f64(app.elapsed());
  let group = lyrics.active_group(elapsed);
  let (group_start, group_end) = group.unwrap_or((usize::MAX, usize::MAX));
  let cursor = (!app.lyrics_follow).then_some(app.lyrics_cursor).flatten();

  // Scrolling: follow centers the active group's first line; manual keeps
  // the stored viewport offset and only adjusts it to keep the pointer
  // visible (viewport is the source of truth, the pointer passively
  // follows).
  let scroll = if app.lyrics_follow {
    group
      .map(|(start, _)| start.saturating_sub(inner.height as usize / 2))
      .unwrap_or(playing.lyrics_scroll)
  } else {
    let mut scroll = playing.lyrics_scroll;
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

  let line_count = lyrics.line_count();
  let mut lines: Vec<Line> = Vec::new();
  for row in 0..inner.height as usize {
    let index = scroll + row;
    if index >= line_count {
      break;
    }
    let is_active = index >= group_start && index < group_end;
    let is_cursor = cursor == Some(index);
    let mut spans: Vec<Span> = Vec::new();
    if is_cursor {
      spans.push(Span::styled(
        "❯ ",
        Style::default()
          .fg(theme.color(&theme.lyrics.cursor))
          .add_modifier(Modifier::BOLD),
      ));
    } else {
      spans.push(Span::raw("  "));
    }

    let text = lyrics.line(index).unwrap_or_default();
    // Each line of the group tracks its own karaoke progress: word-timed
    // originals follow their word tags, translations interpolate.
    let sung = if is_active {
      lyrics.karaoke_at(index, elapsed)
    } else {
      0
    };
    if is_active && sung > 0 {
      // Karaoke: sung prefix highlighted, remainder in the base style.
      let chars: Vec<char> = text.chars().collect();
      let split = sung.min(chars.len());
      spans.push(Span::styled(
        chars[..split].iter().collect::<String>(),
        Style::default()
          .fg(theme.color(&theme.lyrics.active))
          .add_modifier(Modifier::BOLD),
      ));
      spans.push(Span::styled(
        chars[split..].iter().collect::<String>(),
        Style::default().fg(theme.color(&theme.base.foreground)),
      ));
    } else {
      let style = if is_active {
        Style::default()
          .fg(theme.color(&theme.base.foreground))
          .add_modifier(Modifier::BOLD)
      } else {
        Style::default().fg(theme.color(&theme.base.muted))
      };
      spans.push(Span::styled(text.to_string(), style));
    }
    lines.push(Line::from(spans));
  }
  frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
  if let Some(playing) = app.playing.as_mut() {
    playing.lyrics_scroll = scroll;
  }
  app.hit.lyrics_panes.push((inner, source));
}

/// Lyrics for the hovered song: no playback state, so no sync highlight,
/// no follow, no cursor — a plain scrollable list (wheel / j-k style keys).
fn draw_hover_lyrics_pane(frame: &mut Frame, app: &mut App, area: Rect, source: PaneSource) {
  let is_main = app.main_pane() == PaneKind::Lyrics;
  let title = match app.song_view(source) {
    Some(hover) => format!("lyrics · {}", hover.title),
    None => "lyrics (hovered)".to_string(),
  };
  let block = pane_block(app, &title, is_main);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.height == 0 || inner.width == 0 {
    return;
  }
  app.hit.lyrics_panes.push((inner, source));
  let theme = &app.settings.theme;
  let Some(hover) = app.song_view(source) else {
    let hint = "hover a queue or library entry";
    frame.render_widget(
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.base.muted))),
      inner,
    );
    return;
  };
  let Some(lyrics) = hover.lyrics.as_ref() else {
    let hint = hover
      .lyrics_error
      .clone()
      .unwrap_or_else(|| "loading lyrics…".to_string());
    frame.render_widget(
      Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.base.muted))),
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
      Style::default().fg(theme.color(&theme.base.foreground)),
    ));
  }
  frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
