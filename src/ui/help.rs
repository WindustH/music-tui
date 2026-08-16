//! Overlay surfaces: command completion popup and the key-help dialog.

use super::*;

/// Completion list floating above the footer while the command prompt is
/// active.
pub(super) fn draw_completion_popup(
  frame: &mut Frame,
  app: &App,
  footer: Rect,
) -> Option<Rect> {
  let Some(completion) = app.command_state.completion() else {
    return None;
  };
  if app.prompt.is_none() {
    return None;
  }
  let rows = framework_tui::completion_rows(Some(completion), 6).min(6);
  if rows == 0 || footer.y < rows {
    return None;
  }
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
      base: Style::default().fg(theme.color(&theme.base.foreground)),
      selected: Style::default()
        .fg(theme.color(&theme.base.accent))
        .add_modifier(Modifier::BOLD),
    },
  );
  Some(popup)
}

/// Centered, scrollable key-binding dialog (`f1`). Clears overlays and the
/// cursor while open so protocol images do not float above the modal.
pub(super) fn draw_help_dialog(
  frame: &mut Frame,
  app: &mut App,
  area: Rect,
) -> (Vec<ProtocolOverlay>, Option<(u16, u16)>) {
  let theme = &app.settings.theme;
  let background = theme.color(&theme.base.background);
  let base = Style::default()
    .fg(theme.color(&theme.base.foreground))
    .bg(background);
  let help_style = KeyHelpDialogStyle {
    popup: PopupDialogStyle {
      base,
      border: Style::default()
        .fg(theme.color(&theme.base.border))
        .bg(background),
      max_height: area.height.saturating_sub(2).clamp(8, 34),
      ..PopupDialogStyle::default()
    },
    key: Style::default()
      .fg(theme.color(&theme.base.accent))
      .bg(background)
      .add_modifier(Modifier::BOLD),
    description: base,
    muted: Style::default()
      .fg(theme.color(&theme.base.muted))
      .bg(background),
    ..KeyHelpDialogStyle::default()
  };
  let entries = framework_tui::merge_help_entries(
    app.pane_bindings()
      .iter()
      .flat_map(|bindings| {
        bindings.help_entries_filtered(framework_tui::KeyContext::Browser, |_| true)
      }),
  );
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
  (Vec::new(), None)
}
