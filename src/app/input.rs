//! Keyboard input dispatch: prompt mode, help dialog, main-pane bindings.

use super::*;

impl App {
  pub fn handle_input(&mut self, event: Event) -> bool {
    if let Event::Paste(value) = event {
      if let Some(prompt) = self.prompt.as_mut() {
        let result =
          framework_tui::input::handle_prompt_paste(prompt, &mut self.command_state, &value);
        return self.apply_prompt_result(result);
      }
      return false;
    }
    if let Event::Mouse(mouse) = event {
      return self.handle_mouse(mouse);
    }
    let Event::Key(key) = event else {
      return false;
    };

    if let Some(prompt) = self.prompt.as_mut() {
      let result = framework_tui::input::handle_prompt_key(
        prompt,
        &mut self.command_state,
        &self.input_bindings,
        key,
      );
      return self.apply_prompt_result(result);
    }

    if self.show_help {
      return match framework_tui::handle_help_dialog_key(
        &mut self.help_scroll,
        self.max_help_scroll,
        &self.help_bindings,
        key,
      ) {
        framework_tui::HelpDialogInput::Scrolled => true,
        framework_tui::HelpDialogInput::Closed => {
          self.show_help = false;
          true
        }
        framework_tui::HelpDialogInput::Unhandled => false,
      };
    }

    let Some(token) = key_event_to_token(key) else {
      return false;
    };
    let bindings = self
      .view_bindings
      .get(self.main_pane().index())
      .expect("bindings for main pane");
    match self.dispatcher.dispatch(bindings, KeyContext::Browser, token) {
      MatchResult::Action(action) => {
        self.dispatcher.clear();
        debug!(%action, "action dispatched");
        self.run_action(&action)
      }
      MatchResult::Prefix(_) => true,
      MatchResult::None => false,
    }
  }
}
