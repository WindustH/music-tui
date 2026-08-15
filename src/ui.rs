//! Terminal UI: tab bar, layout-tree panes, footer, prompt and popups.

use std::time::Duration;

use framework_tui::{
  CompletionListStyle, KeyHelpDialogStyle, KeyHintsStyle, PopupDialogStyle, PromptLineStyle,
  draw_completion_list, draw_key_help_dialog_scrolled, draw_key_hints, draw_prompt_line,
  key_hint_columns, key_hint_rows,
};
use img_tui::ProtocolOverlay;
use mpd_client::commands::SingleMode;
use mpd_client::responses::{PlayState, SongInQueue};
use ratatui::{
  Frame,
  buffer::CellDiffOption,
  layout::{Alignment, Constraint, Layout, Rect},
  style::{Modifier, Style},
  text::{Line, Span},
  widgets::{Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, Wrap},
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

  // Footer height grows with the pending which-key hints (pdf-tui style).
  let hints: Vec<framework_tui::KeyHint> = if app.show_help {
    Vec::new()
  } else {
    app.dispatcher
      .hints()
      .iter()
      .map(|hint| framework_tui::KeyHint {
        key: hint.key.clone(),
        label: hint.label.clone(),
      })
      .collect()
  };
  let hint_rows = if hints.is_empty() {
    0
  } else {
    key_hint_rows(
      hints.len(),
      key_hint_columns(usize::from(app.settings.theme.which_key_columns), area.width),
    ) as u16
  };

  let [tab_bar, content, footer] = Layout::vertical([
    Constraint::Length(1),
    Constraint::Min(0),
    Constraint::Length(3 + hint_rows),
  ])
  .areas(area);

  app.lyrics_pane_areas.clear();
  app.queue_pane_areas.clear();

  draw_tab_bar(frame, app, tab_bar);

  let mut overlays = Vec::new();
  let tab = app.current_tab().clone();
  if let Some(detail) = app.detail.as_ref() {
    // Secondary detail view: replaces the tab content (gallery-tui style).
    draw_detail_view(frame, app, detail, renderer, tx, content, &mut overlays);
  } else {
    draw_layout(frame, app, renderer, tx, content, &tab.layout, &mut overlays);
  }

  let mut cursor_position = draw_footer(frame, app, footer, &hints);

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
    let theme = &app.settings.theme;
    let background = theme.color(&theme.background);
    let base = Style::default()
      .fg(theme.color(&theme.foreground))
      .bg(background);
    let help_style = KeyHelpDialogStyle {
      popup: PopupDialogStyle {
        base,
        border: Style::default()
          .fg(theme.color(&theme.border))
          .bg(background),
        max_height: area.height.saturating_sub(2).clamp(8, 34),
        ..PopupDialogStyle::default()
      },
      key: Style::default()
        .fg(theme.color(&theme.accent))
        .bg(background)
        .add_modifier(Modifier::BOLD),
      description: base,
      muted: Style::default()
        .fg(theme.color(&theme.muted))
        .bg(background),
      ..KeyHelpDialogStyle::default()
    };
    let entries = app
      .bindings()
      .help_entries_filtered(framework_tui::KeyContext::Browser, |_| true);
    if let Some(popup) = draw_key_help_dialog_scrolled(
      frame,
      area,
      &format!("keybindings: {}", app.current_tab().name),
      &entries,
      &help_style,
      app.help_scroll,
    ) {
      // Content = entries + close hint; visible rows sit between borders.
      let visible = popup.height.saturating_sub(2) as usize;
      app.max_help_scroll = (entries.len() + 1).saturating_sub(visible);
      app.help_scroll = app.help_scroll.min(app.max_help_scroll);
    }
    // pdf-tui behavior: modals suppress transient protocol output —
    // otherwise a kitty/sixel cover keeps floating above the dialog.
    overlays.clear();
    cursor_position = None;
  }

  FrameOutput {
    overlays,
    protocol_writes: Vec::new(),
    cursor_position,
    preserve_overlays: false,
    preserve_areas: Vec::new(),
  }
}

fn hint_rows_for(hints: &[framework_tui::KeyHint], width: u16) -> u16 {
  if hints.is_empty() {
    return 0;
  }
  key_hint_rows(hints.len(), key_hint_columns(3, width)) as u16
}

fn draw_tab_bar(frame: &mut Frame, app: &mut App, area: Rect) {
  let theme = &app.settings.theme;
  let border = Style::default().fg(theme.color(&theme.border));
  app.tab_hit_areas.clear();
  let mut spans = Vec::new();
  let mut column = area.x;
  for (index, tab) in app.tabs.iter().enumerate() {
    if index > 0 {
      spans.push(Span::styled(" │ ", border));
      column += 3;
    }
    let active = index == app.tab;
    let label = format!(" {} ", tab.name);
    let style = if active {
      Style::default()
        .fg(theme.color(&theme.accent))
        .add_modifier(Modifier::BOLD)
    } else {
      Style::default().fg(theme.color(&theme.muted))
    };
    // Record the hit rectangle for mouse-based tab switching.
    if app.tab_hit_areas.len() < 9 {
      app.tab_hit_areas.push(Rect {
        x: column,
        y: area.y,
        width: label.chars().count() as u16,
        height: 1,
      });
    }
    column += label.chars().count() as u16;
    spans.push(Span::styled(label, style));
  }
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

  // Center the artwork: fit the intrinsic aspect ratio inside the pane,
  // converting through cell pixels (same math as gallery-tui's
  // `fit_image_rect` — cells are taller than they are wide).
  let (cell_width, cell_height) = renderer.cell_pixels();
  let image_area = match app.cover_dims {
    Some((image_width, image_height)) if image_width > 0 && image_height > 0 => {
      let max_pixel_width = f64::from(inner.width) * f64::from(cell_width.max(1));
      let max_pixel_height = f64::from(inner.height) * f64::from(cell_height.max(1));
      let scale = (max_pixel_width / f64::from(image_width))
        .min(max_pixel_height / f64::from(image_height))
        .max(0.0);
      let fitted_width = ((f64::from(image_width) * scale) / f64::from(cell_width.max(1)))
        .round()
        .clamp(1.0, f64::from(inner.width)) as u16;
      let fitted_height = ((f64::from(image_height) * scale) / f64::from(cell_height.max(1)))
        .round()
        .clamp(1.0, f64::from(inner.height)) as u16;
      Rect {
        x: inner.x + inner.width.saturating_sub(fitted_width) / 2,
        y: inner.y + inner.height.saturating_sub(fitted_height) / 2,
        width: fitted_width,
        height: fitted_height,
      }
    }
    _ => inner,
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
          // Keep the TUI from touching cells under the protocol image.
          reserve_protocol_area(frame, image_area);
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

fn reserve_protocol_area(frame: &mut Frame, area: Rect) {
  let buffer = frame.buffer_mut();
  for y in area.y..area.y.saturating_add(area.height) {
    for x in area.x..area.x.saturating_add(area.width) {
      if let Some(cell) = buffer.cell_mut((x, y)) {
        cell.set_diff_option(CellDiffOption::Skip);
      }
    }
  }
}

fn draw_lyrics_pane(frame: &mut Frame, app: &mut App, area: Rect) {
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
      .unwrap_or(app.lyrics_scroll as usize)
  } else {
    let mut scroll = app.lyrics_scroll as usize;
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
  app.lyrics_scroll = scroll as u16;
  app.lyrics_pane_areas.push(inner);

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

/// Secondary detail surface for a queue entry (`i`): cover on top,
/// metadata below — the sidebar data stays untouched.
fn draw_detail_view(
  frame: &mut Frame,
  app: &App,
  detail: &crate::app::DetailView,
  renderer: &mut CoverRenderStore,
  tx: &mpsc::UnboundedSender<AsyncEvent>,
  area: Rect,
  overlays: &mut Vec<ProtocolOverlay>,
) {
  let theme = &app.settings.theme;
  let block = Block::default()
    .borders(Borders::ALL)
    .border_style(Style::default().fg(theme.color(&theme.accent)))
    .title(format!(" detail: {} ", detail.title))
    .title_alignment(Alignment::Center);
  let inner = block.inner(area);
  frame.render_widget(block, area);
  if inner.width < 2 || inner.height < 2 {
    return;
  }
  let [cover_area, metadata_area] =
    Layout::vertical([Constraint::Ratio(2, 3), Constraint::Ratio(1, 3)]).areas(inner);

  // --- cover (same fitting math as the cover pane) ---
  let (cell_width, cell_height) = renderer.cell_pixels();
  let image_area = match detail.cover_dims {
    Some((image_width, image_height)) if image_width > 0 && image_height > 0 => {
      let max_pixel_width = f64::from(cover_area.width) * f64::from(cell_width.max(1));
      let max_pixel_height = f64::from(cover_area.height) * f64::from(cell_height.max(1));
      let scale = (max_pixel_width / f64::from(image_width))
        .min(max_pixel_height / f64::from(image_height))
        .max(0.0);
      let fitted_width = ((f64::from(image_width) * scale) / f64::from(cell_width.max(1)))
        .round()
        .clamp(1.0, f64::from(cover_area.width)) as u16;
      let fitted_height = ((f64::from(image_height) * scale) / f64::from(cell_height.max(1)))
        .round()
        .clamp(1.0, f64::from(cover_area.height)) as u16;
      Rect {
        x: cover_area.x + cover_area.width.saturating_sub(fitted_width) / 2,
        y: cover_area.y + cover_area.height.saturating_sub(fitted_height) / 2,
        width: fitted_width,
        height: fitted_height,
      }
    }
    _ => cover_area,
  };
  match detail.cover.as_ref() {
    Some(path) => {
      renderer.request(path, image_area.width, image_area.height, tx);
      match renderer.get(path, image_area.width, image_area.height) {
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
          reserve_protocol_area(frame, image_area);
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
              .style(Style::default().fg(theme.color(&theme.muted)))
              .alignment(Alignment::Center),
            cover_area,
          );
        }
      }
    }
    None => {
      let hint = detail
        .cover_error
        .clone()
        .unwrap_or_else(|| "no cover".to_string());
      frame.render_widget(
        Paragraph::new(hint)
          .style(Style::default().fg(theme.color(&theme.muted)))
          .alignment(Alignment::Center),
        cover_area,
      );
    }
  }

  // --- metadata ---
  let metadata_block = Block::default()
    .borders(Borders::ALL)
    .border_style(Style::default().fg(theme.color(&theme.border)))
    .title(" metadata (e edit · i close) ");
  let metadata_inner = metadata_block.inner(metadata_area);
  frame.render_widget(metadata_block, metadata_area);
  if metadata_inner.height == 0 {
    return;
  }
  match detail.metadata.as_ref() {
    Some(entries) => {
      let lines: Vec<Line> = entries
        .iter()
        .skip(detail.metadata_scroll as usize)
        .map(|entry| metadata_line(app, &entry.name, &entry.value))
        .collect();
      frame.render_widget(Paragraph::new(lines), metadata_inner);
    }
    None => {
      let hint = detail
        .metadata_error
        .clone()
        .unwrap_or_else(|| "reading metadata…".to_string());
      frame.render_widget(
        Paragraph::new(hint).style(Style::default().fg(theme.color(&theme.muted))),
        metadata_inner,
      );
    }
  }
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

  // Resample the configured bar count to the pane width so the visualization
  // always spans the full available area (max of each source range).
  let bars = &app.spectrum;
  let columns = inner.width as usize;
  let values: Vec<u8> = (0..columns)
    .map(|column| {
      let start = column * bars.len() / columns;
      let end = ((column + 1) * bars.len() / columns).max(start + 1);
      bars[start..end.min(bars.len())]
        .iter()
        .map(|value| *value)
        .max()
        .unwrap_or(0)
    })
    .collect();

  // Full-height vertical bars: '█' for fully filled rows, a partial block
  // at the top edge, empty cells above — ncmpcpp style, bottom-aligned.
  let height = inner.height as usize;
  let fraction_chars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇'];
  let mut lines: Vec<Line> = Vec::with_capacity(height);
  for row in 0..height {
    let from_bottom = height - 1 - row;
    let mut spans = Vec::with_capacity(columns);
    for value in &values {
      let value = (*value).min(100) as usize;
      let full = value * height / 100; // fully filled rows below the tip
      let remainder = value * height % 100; // fraction of the tip row
      let (ch, lit) = if from_bottom < full {
        ('█', true)
      } else if from_bottom == full && value > 0 {
        let index = (remainder * fraction_chars.len() / 100).max(1);
        (
          fraction_chars[(index - 1).min(fraction_chars.len() - 1)],
          true,
        )
      } else {
        (' ', false)
      };
      let color = if value < 34 {
        theme.color(&theme.visualizer_low)
      } else if value < 67 {
        theme.color(&theme.visualizer_mid)
      } else {
        theme.color(&theme.visualizer_high)
      };
      let style = if lit {
        Style::default().fg(color)
      } else {
        Style::default()
      };
      spans.push(Span::styled(ch.to_string(), style));
    }
    lines.push(Line::from(spans));
  }
  frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_footer(
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
    draw_key_hints(
      frame,
      hints,
      hints_area,
      &KeyHintsStyle {
        base: Style::default()
          .fg(theme.color(&theme.which_key_foreground))
          .bg(theme.color(&theme.which_key_background)),
        key: Style::default()
          .fg(theme.color(&theme.which_key_key))
          .bg(theme.color(&theme.which_key_background))
          .add_modifier(Modifier::BOLD),
        separator: Style::default()
          .fg(theme.color(&theme.which_key_separator_color))
          .bg(theme.color(&theme.which_key_background)),
        description: Style::default()
          .fg(theme.color(&theme.which_key_description))
          .bg(theme.color(&theme.which_key_background)),
        separator_text: theme.which_key_separator.clone(),
        columns: key_hint_columns(usize::from(theme.which_key_columns), area.width),
      },
    );
  }

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
