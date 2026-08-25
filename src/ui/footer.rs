//! Footer: which-key hints, status line, prompt, and the progress band.

use super::*;

fn hint_rows_for(hints: &[framework_tui::KeyHint], width: u16) -> u16 {
  if hints.is_empty() {
    return 0;
  }
  key_hint_rows(hints.len(), key_hint_columns(3, width)) as u16
}


pub(super) fn draw_footer(
  frame: &mut Frame,
  app: &mut App,
  area: Rect,
  hints: &[framework_tui::KeyHint],
) -> Option<(u16, u16)> {
  let theme = &app.settings.theme;
  let [hints_area, status_line, input_line, band_line] = Layout::vertical([
    Constraint::Length(hint_rows_for(hints, area.width)),
    Constraint::Length(1),
    Constraint::Length(1),
    Constraint::Length(1),
  ])
  .areas(area);

  // --- which-key hints (pending key sequences) ---
  if !hints.is_empty() {
    let background = overlay_background();
    draw_key_hints(
      frame,
      hints,
      hints_area,
      &KeyHintsStyle {
        base: Style::default()
          .fg(theme.color(&theme.which_key.foreground))
          .bg(background),
        key: Style::default()
          .fg(theme.color(&theme.which_key.key))
          .bg(background)
          .add_modifier(Modifier::BOLD),
        separator: Style::default()
          .fg(theme.color(&theme.which_key.separator_color))
          .bg(background),
        description: Style::default()
          .fg(theme.color(&theme.which_key.description))
          .bg(background),
        separator_text: theme.which_key.separator.clone(),
        columns: key_hint_columns(usize::from(theme.which_key.columns), area.width),
      },
    );
  }

  // --- status line ---
  let mut spans = Vec::new();
  let state_style = |color: &str| Style::default().fg(theme.color(color));
  match app.status.as_ref().map(|status| status.state) {
    Some(PlayState::Playing) => spans.push(Span::styled("▶ ", state_style(&theme.footer.playing))),
    Some(PlayState::Paused) => spans.push(Span::styled("⏸ ", state_style(&theme.footer.paused))),
    Some(PlayState::Stopped) | None => spans.push(Span::styled("■ ", state_style(&theme.footer.stopped))),
  }
  if let Some(song) = app.current_song() {
    let title = song_title(&song.song)
      .map(str::to_string)
      .unwrap_or_else(|| song.song.url.clone());
    let artist = song_artist(&song.song).map(str::to_string).unwrap_or_default();
    let label = if artist.is_empty() { title } else { format!("{title} — {artist}") };
    spans.push(Span::styled(label, Style::default().fg(theme.color(&theme.base.foreground))));
  } else if let Some(error) = app.connection_error.as_ref() {
    spans.push(Span::styled(
      format!("mpd offline: {error}"),
      Style::default().fg(theme.color(&theme.footer.stopped)),
    ));
  } else {
    spans.push(Span::styled("idle", Style::default().fg(theme.color(&theme.base.muted))));
  }

  let mut flags = String::new();
  if let Some(status) = app.status.as_ref() {
    if status.repeat {
      flags.push('R');
    }
    if status.random {
      flags.push('z');
    }
    if status.single != SingleMode::Disabled {
      flags.push('s');
    }
    if status.consume {
      flags.push('c');
    }
  }
  let volume = app.status.as_ref().map(|status| status.volume).unwrap_or(0);
  let right = format!(
    " {}vol:{}%{} ",
    if flags.is_empty() { String::new() } else { format!("[{flags}] ") },
    volume,
    if app.follow_current { " ⌖" } else { "" },
  );
  spans.push(Span::styled(right, Style::default().fg(theme.color(&theme.base.muted))));
  frame.render_widget(
    Paragraph::new(Line::from(spans)).wrap(Wrap { trim: true }),
    status_line,
  );

  let mut cursor_position = None;

  // --- prompt / message / hints line ---
  if let Some(prompt) = app.prompt.as_ref() {
    let completion = app.command_state.completion();
    let style = PromptLineStyle {
      base: Style::default().fg(theme.color(&theme.base.foreground)),
      prefix: Style::default()
        .fg(theme.color(&theme.base.accent))
        .add_modifier(Modifier::BOLD),
      suggestion: Style::default().fg(theme.color(&theme.base.muted)),
    };
    cursor_position = draw_prompt_line(frame, prompt, completion, input_line, &style);
  } else if let Some(message) = app.message_text() {
    frame.render_widget(
      Paragraph::new(Line::from(Span::styled(
        format!(" {message}"),
        Style::default().fg(theme.color(&theme.base.accent_alt)),
      ))),
      input_line,
    );
  }
  // No static hint line: keys follow the user's keymap, discoverable via
  // the f1 help dialog instead.

  // Paint the band last: the theme borrow above must end before handing
  // `app` over as mutable.
  draw_progress_band(frame, app, band_line);
  cursor_position
}

/// Full-width progress band pinned to the bottom of the interface.
/// Clicking or dragging it seeks (hit-tested in `App::handle_mouse`).
fn draw_progress_band(frame: &mut Frame, app: &mut App, area: Rect) {
  app.progress_band_area = (area.width > 0).then_some(area);
  let theme = &app.settings.theme;
  let filled = theme.color(&theme.progress.bar);
  let rest = theme.color(&theme.progress.background);

  let (ratio, label) = match app.duration() {
    Some(duration) if duration > 0.0 => {
      let elapsed = app.elapsed();
      let ratio = (elapsed / duration).clamp(0.0, 1.0);
      let label = format!(
        "{} / {}",
        format_duration_line(Duration::from_secs_f64(elapsed)),
        format_duration_line(Duration::from_secs_f64(duration)),
      );
      (ratio, Some(label))
    }
    _ => (0.0, None::<String>),
  };

  // Split into whole cells plus a half-block at the boundary for sub-cell
  // precision; the rest is a plain background band.
  let filled_cells = ratio * f64::from(area.width);
  let whole = filled_cells.floor() as u16;
  let fraction = filled_cells - f64::from(whole);
  let mut spans: Vec<Span> = Vec::new();
  let remaining = area.width.saturating_sub(whole);
  if whole > 0 {
    spans.push(Span::styled(
      " ".repeat(whole as usize),
      Style::default().bg(filled),
    ));
  }
  if remaining > 0 && fraction >= 0.5 {
    spans.push(Span::styled("▌", Style::default().fg(filled).bg(rest)));
    if remaining > 1 {
      spans.push(Span::styled(
        " ".repeat((remaining - 1) as usize),
        Style::default().bg(rest),
      ));
    }
  } else if remaining > 0 {
    spans.push(Span::styled(
      " ".repeat(remaining as usize),
      Style::default().bg(rest),
    ));
  }
  frame.render_widget(Paragraph::new(Line::from(spans)), area);

  // Overlay the time label; bg is left unset so the band colors show through.
  if let Some(label) = label.filter(|label| area.width as usize >= label.len() + 4) {
    let label = Line::from(Span::styled(
      format!(" {label} "),
      Style::default()
        .fg(theme.color(&theme.base.foreground))
        .add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(Paragraph::new(label).alignment(Alignment::Center), area);
  }
}
