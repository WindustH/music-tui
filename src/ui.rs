//! Terminal UI: tab bar, layout-tree panes, footer, prompt and popups.

use std::time::Duration;

use framework_tui::{
  CompletionListStyle, KeyHelpDialogStyle, KeyHint, KeyHintsStyle, PromptLineStyle,
  draw_completion_list, draw_key_help_dialog, draw_key_hints, draw_prompt_line,
};
use img_tui::ProtocolOverlay;
use mpd_client::commands::SingleMode;
use mpd_client::responses::{PlayState, SongInQueue};
use ratatui::{
  Frame,
  layout::{Alignment, Constraint, Layout, Rect},
  style::{Modifier, Style},
  text::{Line, Span},
  widgets::{Block, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, Wrap},
};
use tokio::sync::mpsc;

use crate::{
  app::App,
  event::{AsyncEvent, RenderedImage},
  layout::{PaneKind, PaneLayout, SplitDir},
  render::CoverRenderStore,
  terminal::FrameOutput,
};

pub fn draw(
  frame: &mut Frame,
  app: &mut App,
  renderer: &mut CoverRenderStore,
  tx: &mpsc::UnboundedSender<AsyncEvent>,
) -> FrameOutput {
  let area = frame.area();

  let [tab_bar, content, footer] = Layout::vertical([
    Constraint::Length(1),
    Constraint::Min(0),
    Constraint::Length(3),
  ])
  .areas(area);

  draw_tab_bar(frame, app, tab_bar);

  let mut overlays = Vec::new();
  let tab = app.current_tab().clone();
  draw_layout(frame, app, renderer, tx, content, &tab.layout, &mut overlays);

  let mut cursor_position = draw_footer(frame, app, footer);

  if let Some(completion) = app.command_state.completion()
    && app.prompt.is_some()
  {
    let rows = framework_tui::completion_rows(Some(completion), 6).min(6);
    if rows > 0 && footer.y >= rows {
      let theme = &app.settings.theme;
      let popup = Rect {
        x: footer.x,
        y: footer.y - rows,
        width: footer.width.min(40),
        height: rows,
      };
      draw_completion_list(
        frame,
        completion,
        popup,
        &CompletionListStyle {
          base: Style::default().fg(theme.color(&theme.foreground)),
          selected: Style::default()
            .fg(theme.color(&theme.accent))
            .add_modifier(Modifier::BOLD),
        },
      );
    }
  }

  if app.show_help {
    let entries = app
      .bindings()
      .help_entries_filtered(framework_tui::KeyContext::Browser, |_| true);
    draw_key_help_dialog(
      frame,
      area,
      &format!("keybindings: {}", app.current_tab().name),
      &entries,
      &KeyHelpDialogStyle::default(),
    );
  }

  let _ = cursor_position.take();

  FrameOutput {
    overlays,
    protocol_writes: Vec::new(),
    cursor_position,
    preserve_overlays: false,
    preserve_areas: Vec::new(),
  }
}

fn draw_tab_bar(frame: &mut Frame, app: &App, area: Rect) {
  let theme = &app.settings.theme;
  let border = Style::default().fg(theme.color(&theme.border));
  let mut spans = Vec::new();
  for (index, tab) in app.tabs.iter().enumerate() {
    if index > 0 {
      spans.push(Span::styled(" │ ", border));
    }
    let active = index == app.tab;
    let label = format!(" {} {} ", index + 1, tab.name);
    let style = if active {
      Style::default()
        .fg(theme.color(&theme.accent))
        .add_modifier(Modifier::BOLD)
    } else {
      Style::default().fg(theme.color(&theme.muted))
    };
    spans.push(Span::styled(label, style));
  }
  spans.push(Span::styled(
    "  ←/→ switch tabs",
    Style::default().fg(theme.color(&theme.muted)),
  ));
  frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_layout(
  frame: &mut Frame,
  app: &mut App,
  renderer: &mut CoverRenderStore,
  tx: &mpsc::UnboundedSender<AsyncEvent>,
  area: Rect,
  layout: &PaneLayout,
  overlays: &mut Vec<ProtocolOverlay>,
) {
  match layout {
    PaneLayout::Pane(kind) => draw_pane(frame, app, renderer, tx, area, *kind, overlays),
    PaneLayout::Split {
      dir,
      ratio,
      first,
      second,
    } => {
      let constraints = [
        Constraint::Ratio(ratio.0 as u32, ratio.0 as u32 + ratio.1 as u32),
        Constraint::Ratio(ratio.1 as u32, ratio.0 as u32 + ratio.1 as u32),
      ];
      let areas: [Rect; 2] = match dir {
        SplitDir::Horizontal => Layout::horizontal(constraints).areas(area),
        SplitDir::Vertical => Layout::vertical(constraints).areas(area),
      };
      draw_layout(frame, app, renderer, tx, areas[0], first, overlays);
      draw_layout(frame, app, renderer, tx, areas[1], second, overlays);
    }
  }
}

fn draw_pane(
  frame: &mut Frame,
  app: &mut App,
  renderer: &mut CoverRenderStore,
  tx: &mpsc::UnboundedSender<AsyncEvent>,
  area: Rect,
  kind: PaneKind,
  overlays: &mut Vec<ProtocolOverlay>,
) {
  match kind {
    PaneKind::Queue => draw_queue_pane(frame, app, area),
    PaneKind::Cover => draw_cover_pane(frame, app, renderer, tx, area, overlays),
    PaneKind::Lyrics => draw_lyrics_pane(frame, app, area),
    PaneKind::Metadata => draw_metadata_pane(frame, app, area),
    PaneKind::Visualizer => draw_visualizer_pane(frame, app, area),
  }
}

fn pane_block(app: &App, title: &str, is_main: bool) -> Block<'static> {
  let theme = &app.settings.theme;
  let title_span = if is_main {
    Span::styled(
      format!(" {title} "),
      Style::default()
        .fg(theme.color(&theme.accent))
        .add_modifier(Modifier::BOLD),
    )
  } else {
    Span::styled(format!(" {title} "), Style::default().fg(theme.color(&theme.muted)))
  };
  Block::bordered()
    .title(title_span)
    .border_style(Style::default().fg(theme.color(&theme.border)))
}

fn draw_queue_pane(frame: &mut Frame, app: &mut App, area: Rect) {
  let theme = &app.settings.theme;
  let is_main = app.main_pane() == PaneKind::Queue;
  let block = pane_block(app, &format!("queue ({})", app.queue.len()), is_main);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.height == 0 || inner.width == 0 {
    return;
  }

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

  let (position, _) = app
    .status
    .as_ref()
    .and_then(|status| status.current_song)
    .unzip();
  let playing = position.map(|pos| pos.0);

  let items: Vec<ListItem> = app
    .queue
    .iter()
    .enumerate()
    .map(|(index, song)| ListItem::new(queue_line(app, index, song, playing)))
    .collect();

  let list = List::new(items).highlight_style(
    Style::default()
      .fg(theme.color(&theme.accent))
      .add_modifier(Modifier::BOLD),
  );
  frame.render_stateful_widget(list, inner, &mut app.queue_state);

  let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
    .style(Style::default().fg(theme.color(&theme.border)));
  let mut state = ratatui::widgets::ScrollbarState::new(app.queue.len())
    .position(app.queue_state.selected().unwrap_or(0));
  frame.render_stateful_widget(scrollbar, area, &mut state);
}

fn queue_line(app: &App, index: usize, song: &SongInQueue, playing: Option<usize>) -> Line<'static> {
  let theme = &app.settings.theme;
  let title = song
    .song
    .title()
    .map(str::to_string)
    .unwrap_or_else(|| song.song.url.clone());
  let artist = song.song.artists().first().cloned().unwrap_or_default();
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
    .map(|duration| format_duration_line(duration))
    .unwrap_or_default();
  Line::from(vec![marker, main, Span::raw(" "), Span::styled(duration, Style::default().fg(theme.color(&theme.muted)))])
}

fn draw_cover_pane(
  frame: &mut Frame,
  app: &mut App,
  renderer: &mut CoverRenderStore,
  tx: &mpsc::UnboundedSender<AsyncEvent>,
  area: Rect,
  overlays: &mut Vec<ProtocolOverlay>,
) {
  let theme = &app.settings.theme;
  let is_main = app.main_pane() == PaneKind::Cover;
  let block = pane_block(app, "cover", is_main);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.width < 2 || inner.height < 2 {
    return;
  }

  // Center the artwork as a square that fits the pane.
  let side = inner.width.min(inner.height);
  let image_area = Rect {
    x: inner.x + (inner.width - side) / 2,
    y: inner.y + (inner.height - side) / 2,
    width: side,
    height: side,
  };

  let current_url = app.current_song_url().unwrap_or_default();
  let cover = app
    .cover_path
    .as_ref()
    .filter(|(url, _)| url == &current_url)
    .map(|(_, path)| path.clone());

  match cover {
    Some(path) => {
      renderer.request(&path, image_area.width, image_area.height, tx);
      match renderer.get(&path, image_area.width, image_area.height) {
        Some(RenderedImage::Symbols { text, .. }) => {
          frame.render_widget(
            Paragraph::new(text.clone()).wrap(Wrap { trim: false }),
            image_area,
          );
        }
        Some(RenderedImage::Protocol {
          mode,
          data,
          refresh,
          placement,
          fingerprint,
          erase,
        }) => {
          overlays.push(ProtocolOverlay {
            area: image_area,
            mode: *mode,
            data: data.clone(),
            refresh: refresh.clone(),
            placement: placement.clone(),
            fingerprint: *fingerprint,
            erase: erase.clone(),
          });
        }
        None => {
          frame.render_widget(
            Paragraph::new("rendering cover…")
              .style(Style::default().fg(theme.color(&theme.muted))),
            inner,
          );
        }
      }
    }
    None => {
      let hint = app
        .cover_error
        .clone()
        .unwrap_or_else(|| "no cover".to_string());
      frame.render_widget(
        Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.muted))),
        inner,
      );
    }
  }
}

fn draw_lyrics_pane(frame: &mut Frame, app: &mut App, area: Rect) {
  let theme = &app.settings.theme;
  let is_main = app.main_pane() == PaneKind::Lyrics;
  let block = pane_block(app, "lyrics", is_main);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.height == 0 {
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

  let elapsed = app.elapsed();
  let active = lyrics.active_index(Duration::from_secs_f64(elapsed));

  if app.lyrics_follow
    && let Some(active) = active
  {
    let target = active.saturating_sub(inner.height as usize / 2);
    app.lyrics_scroll = target as u16;
  }
  let scroll = app.lyrics_scroll as usize;

  let lines: Vec<Line> = match lyrics {
    crate::lyrics::Lyrics::Synced(lines) => lines
      .iter()
      .skip(scroll)
      .enumerate()
      .map(|(row, line)| {
        let is_active = Some(row + scroll) == active;
        let style = if is_active {
          Style::default()
            .fg(theme.color(&theme.lyrics_active))
            .add_modifier(Modifier::BOLD)
        } else {
          Style::default().fg(theme.color(&theme.foreground))
        };
        Line::styled(line.text.clone(), style)
      })
      .collect(),
    crate::lyrics::Lyrics::Plain(lines) => lines
      .iter()
      .skip(scroll)
      .map(|line| Line::styled(line.clone(), Style::default().fg(theme.color(&theme.foreground))))
      .collect(),
  };
  frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn draw_metadata_pane(frame: &mut Frame, app: &mut App, area: Rect) {
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
    .skip(app.metadata_scroll as usize)
    .map(|entry| metadata_line(app, &entry.name, &entry.value))
    .collect();
  frame.render_widget(Paragraph::new(lines), inner);
}

fn metadata_line(app: &App, name: &str, value: &str) -> Line<'static> {
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

fn draw_visualizer_pane(frame: &mut Frame, app: &mut App, area: Rect) {
  let theme = &app.settings.theme;
  let is_main = app.main_pane() == PaneKind::Visualizer;
  let block = pane_block(app, "visualizer", is_main);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.height == 0 || inner.width == 0 {
    return;
  }

  if app.spectrum.is_empty() {
    frame.render_widget(
      Paragraph::new("waiting for audio on the mpd fifo…")
        .style(Style::default().fg(theme.color(&theme.muted))),
      inner,
    );
    return;
  }

  let chars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
  let mut spans = Vec::with_capacity(app.spectrum.len());
  for value in &app.spectrum {
    let value = (*value).min(100) as usize;
    let index = if value == 0 { 0 } else { (value * chars.len() / 100).min(chars.len() - 1) };
    let color = if value < 34 {
      theme.color(&theme.visualizer_low)
    } else if value < 67 {
      theme.color(&theme.visualizer_mid)
    } else {
      theme.color(&theme.visualizer_high)
    };
    spans.push(Span::styled(chars[index].to_string(), Style::default().fg(color)));
  }
  let line = Line::from(spans);
  let text_height = 1u16;
  let centered = Rect {
    x: inner.x,
    y: inner.y + inner.height.saturating_sub(text_height) / 2,
    width: inner.width,
    height: text_height,
  };
  frame.render_widget(Paragraph::new(line), centered);
}

fn draw_footer(frame: &mut Frame, app: &mut App, area: Rect) -> Option<(u16, u16)> {
  let theme = &app.settings.theme;
  let [status_line, input_line, band_line] =
    Layout::vertical([Constraint::Length(1); 3]).areas(area);

  // --- status line ---
  let mut spans = Vec::new();
  let state_style = |color: &str| Style::default().fg(theme.color(color));
  match app.status.as_ref().map(|status| status.state) {
    Some(PlayState::Playing) => spans.push(Span::styled("▶ ", state_style(&theme.playing))),
    Some(PlayState::Paused) => spans.push(Span::styled("⏸ ", state_style(&theme.paused))),
    Some(PlayState::Stopped) | None => spans.push(Span::styled("■ ", state_style(&theme.stopped))),
  }
  if let Some(song) = app.current_song() {
    let title = song
      .song
      .title()
      .map(str::to_string)
      .unwrap_or_else(|| song.song.url.clone());
    let artist = song.song.artists().first().cloned().unwrap_or_default();
    let label = if artist.is_empty() { title } else { format!("{title} — {artist}") };
    spans.push(Span::styled(label, Style::default().fg(theme.color(&theme.foreground))));
  } else if let Some(error) = app.connection_error.as_ref() {
    spans.push(Span::styled(
      format!("mpd offline: {error}"),
      Style::default().fg(theme.color(&theme.stopped)),
    ));
  } else {
    spans.push(Span::styled("idle", Style::default().fg(theme.color(&theme.muted))));
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
  spans.push(Span::styled(right, Style::default().fg(theme.color(&theme.muted))));
  frame.render_widget(
    Paragraph::new(Line::from(spans)).wrap(Wrap { trim: true }),
    status_line,
  );

  let mut cursor_position = None;

  // --- prompt / message / hints line ---
  if let Some(prompt) = app.prompt.as_ref() {
    let completion = app.command_state.completion();
    let style = PromptLineStyle {
      base: Style::default().fg(theme.color(&theme.foreground)),
      prefix: Style::default()
        .fg(theme.color(&theme.accent))
        .add_modifier(Modifier::BOLD),
      suggestion: Style::default().fg(theme.color(&theme.muted)),
    };
    cursor_position = draw_prompt_line(frame, prompt, completion, input_line, &style);
  } else if let Some(message) = app.message_text() {
    frame.render_widget(
      Paragraph::new(Line::from(Span::styled(
        format!(" {message}"),
        Style::default().fg(theme.color(&theme.accent_alt)),
      ))),
      input_line,
    );
  } else {
  let hints = vec![
    KeyHint { key: "←/→".to_string(), label: "switch tab".to_string() },
    KeyHint { key: "1-9".to_string(), label: "go to tab".to_string() },
    KeyHint { key: "space".to_string(), label: "play/pause".to_string() },
    KeyHint { key: "n/p".to_string(), label: "next/prev".to_string() },
    KeyHint { key: "+-".to_string(), label: "volume".to_string() },
    KeyHint { key: ":".to_string(), label: "command".to_string() },
    KeyHint { key: "f1".to_string(), label: "help".to_string() },
    KeyHint { key: "q".to_string(), label: "quit".to_string() },
  ];
  draw_key_hints(
    frame,
    &hints,
    input_line,
    &KeyHintsStyle {
      base: Style::default().fg(theme.color(&theme.foreground)),
      key: Style::default()
        .fg(theme.color(&theme.accent))
        .add_modifier(Modifier::BOLD),
      separator: Style::default().fg(theme.color(&theme.muted)),
      description: Style::default().fg(theme.color(&theme.muted)),
      separator_text: " · ".to_string(),
      columns: 4,
    },
  );
  }

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
  let filled = theme.color(&theme.progress);
  let rest = theme.color(&theme.progress_background);

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
        .fg(theme.color(&theme.foreground))
        .add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(Paragraph::new(label).alignment(Alignment::Center), area);
  }
}

fn format_duration_line(duration: Duration) -> String {
  let total = duration.as_secs();
  let hours = total / 3600;
  let minutes = (total % 3600) / 60;
  let seconds = total % 60;
  if hours > 0 {
    format!("{hours}:{minutes:02}:{seconds:02}")
  } else {
    format!("{minutes}:{seconds:02}")
  }
}
