//! Overlay surfaces: command completion popup and the key-help dialog.

use super::*;

/// Completion list floating above the footer while the command prompt is
/// active.
pub(super) fn draw_completion_popup(frame: &mut Frame, app: &App, footer: Rect) -> Option<Rect> {
  let completion = app.command_state.completion()?;
  app.prompt.as_ref()?;
  let rows = framework_tui::completion_rows(Some(completion), 5).min(5);
  if rows == 0 || footer.y < rows {
    return None;
  }
  let theme = &app.settings.theme;
  let popup = Rect {
    x: footer.x,
    y: footer.y - rows,
    width: footer.width,
    height: rows,
  };
  draw_completion_list(
    frame,
    completion,
    popup,
    &completion_list_style(theme.color(&theme.which_key.foreground)),
  );
  Some(popup)
}

/// Centered, scrollable key-binding dialog (`f1`). Reports its rect so kitty
/// U=1 placeholder cells yield to the dialog while it is open.
pub(super) fn draw_help_dialog(
  frame: &mut Frame,
  app: &mut App,
  area: Rect,
) -> (Option<Rect>, Option<(u16, u16)>) {
  let theme = &app.settings.theme;
  // Give the dialog a stable opaque surface even when the theme inherits
  // the terminal's default background.
  let background = overlay_background();
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
      .fg(theme.color(&theme.which_key.key))
      .bg(background)
      .add_modifier(Modifier::BOLD),
    description: base.fg(theme.color(&theme.which_key.description)),
    muted: Style::default()
      .fg(theme.color(&theme.base.muted))
      .bg(background),
    ..KeyHelpDialogStyle::default()
  };
  let entries =
    framework_tui::merge_help_entries(app.pane_bindings().iter().flat_map(|bindings| {
      bindings.help_entries_filtered(framework_tui::KeyContext::Browser, |_| true)
    }));
  let popup = draw_key_help_dialog_scrolled(
    frame,
    area,
    &format!("keybindings: {}", app.current_tab().name),
    &entries,
    &help_style,
    app.help_scroll,
  );
  if let Some(popup) = popup {
    // Content = entries + close hint; visible rows sit between borders.
    let visible = popup.height.saturating_sub(2) as usize;
    app.max_help_scroll = (entries.len() + 1).saturating_sub(visible);
    app.help_scroll = app.help_scroll.min(app.max_help_scroll);
    // Modals own the interaction: no cursor while the dialog is open.
    return (Some(popup), None);
  }
  (None, None)
}
