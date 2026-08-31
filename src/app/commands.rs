//! Command prompt (`:`) and queue-filter prompt handling.

use super::*;

const COMMANDS: &[&str] = &[
  "quit", "q", "help", "play", "pause", "toggle", "stop", "next", "prev", "volume", "vol",
  "repeat", "random", "single", "consume", "clear", "update", "tab", "add", "save", "dedup",
];

impl App {
  pub(crate) fn apply_prompt_result(&mut self, result: PromptInputResult) -> bool {
    if self
      .prompt
      .as_ref()
      .is_some_and(|prompt| !prompt.is_command())
    {
      return self.apply_filter_prompt_result(result);
    }
    match result {
      PromptInputResult::Unhandled => false,
      PromptInputResult::Changed => {
        self.refresh_prompt_completion();
        true
      }
      PromptInputResult::Cancel => {
        self.prompt = None;
        self.command_state.reset_prompt_state();
        self.dispatcher.clear();
        self.set_message("cancelled");
        true
      }
      PromptInputResult::Submit => {
        let Some(prompt) = self.prompt.take() else {
          return false;
        };
        let input = prompt.buffer().input.trim().to_string();
        self.command_state.reset_prompt_state();
        self.dispatcher.clear();
        if input.is_empty() {
          return true;
        }
        self.command_state.push_history(input.clone());
        self.run_command_line(&input);
        true
      }
      PromptInputResult::EditInEditor { input } => {
        self.set_message("editing the command in an editor is not supported yet");
        let _ = input;
        true
      }
      PromptInputResult::UnknownAction(action) if action == "help" => {
        self.show_help = true;
        true
      }
      PromptInputResult::UnknownAction(action) => {
        self.set_message(format!("unknown input action: {action}"));
        true
      }
    }
  }

  /// The `/` filter prompt: typing filters live, enter keeps the filter,
  /// esc exits the filter state entirely. Targets the queue or the
  /// library depending on where `/` was pressed.
  fn apply_filter_prompt_result(&mut self, result: PromptInputResult) -> bool {
    match result {
      PromptInputResult::Unhandled | PromptInputResult::UnknownAction(_) => false,
      PromptInputResult::Changed => {
        if let Some(input) = self
          .prompt
          .as_ref()
          .map(Prompt::buffer)
          .map(|buffer| buffer.input.trim().to_string())
        {
          match self.filter_target {
            FilterTarget::Queue => {
              self.queue_filter = (!input.is_empty()).then_some(input);
              self.recompute_queue_filter();
              self.clamp_queue_selection();
            }
            FilterTarget::Library => {
              self.library_filter = (!input.is_empty()).then_some(input);
              self.recompute_library_filter();
              self.clamp_library_selection();
            }
          }
        }
        true
      }
      PromptInputResult::Cancel => {
        self.prompt = None;
        self.command_state.reset_prompt_state();
        self.dispatcher.clear();
        match self.filter_target {
          FilterTarget::Queue => self.clear_queue_filter(),
          FilterTarget::Library => self.clear_library_filter(),
        }
        true
      }
      PromptInputResult::Submit => {
        let input = self
          .prompt
          .take()
          .map(|prompt| prompt.buffer().input.trim().to_string())
          .unwrap_or_default();
        self.command_state.reset_prompt_state();
        self.dispatcher.clear();
        match self.filter_target {
          FilterTarget::Queue => {
            self.queue_filter = (!input.is_empty()).then_some(input);
            self.recompute_queue_filter();
            self.clamp_queue_selection();
          }
          FilterTarget::Library => {
            self.library_filter = (!input.is_empty()).then_some(input);
            self.recompute_library_filter();
            self.clamp_library_selection();
          }
        }
        true
      }
      PromptInputResult::EditInEditor { .. } => {
        self.set_message("editing the filter in an editor is not supported");
        true
      }
    }
  }

  /// Mirrors pdf-tui's refresh_command_completion: no prompt / non-command
  /// prompt clears the completion; command prompts recompute it from the
  /// buffer before the cursor.
  pub(crate) fn refresh_prompt_completion(&mut self) {
    let Some(prompt) = self.prompt.as_ref() else {
      self.command_state.clear_completion();
      return;
    };
    if !prompt.is_command() {
      self.command_state.clear_completion();
      return;
    }
    let buffer = prompt.buffer();
    let completion = self.command_completion_for(&buffer.input, buffer.cursor);
    self
      .command_state
      .set_completion_preserving_selection(completion);
  }

  /// Command-name completion for the first token, per-command candidates for
  /// subcommands — same shape as pdf-tui's command_completion_for.
  fn command_completion_for(
    &self,
    input: &str,
    cursor: usize,
  ) -> Option<framework_tui::CommandCompletion> {
    let cursor = cursor.min(input.len());
    let before_cursor = input.get(..cursor)?;
    let normalized = before_cursor.trim_start_matches(':');
    let tokens = normalized.split_whitespace().collect::<Vec<_>>();
    let ends_with_space = normalized.chars().last().is_some_and(char::is_whitespace);
    let word_start = framework_tui::current_word_start(input, cursor);
    let prefix = if ends_with_space {
      ""
    } else {
      input.get(word_start..cursor).unwrap_or_default()
    };

    if tokens.is_empty() || (tokens.len() == 1 && !ends_with_space) {
      return Some(framework_tui::CommandCompletion::new(
        word_start,
        cursor,
        prefix,
        framework_tui::filter_completion_candidates(COMMANDS.iter().copied(), prefix),
        true,
        0,
      ));
    }

    match tokens[0] {
      "tab" => {
        if tokens.len() > 2 || (tokens.len() == 2 && ends_with_space) {
          return None;
        }
        let replace_start = if ends_with_space { cursor } else { word_start };
        let prefix = if ends_with_space { "" } else { prefix };
        Some(framework_tui::CommandCompletion::new(
          replace_start,
          cursor,
          prefix,
          framework_tui::filter_completion_candidates(
            self.tabs.iter().map(|tab| tab.name.as_str()),
            prefix,
          ),
          true,
          0,
        ))
      }
      _ => None,
    }
  }

  fn run_command_line(&mut self, input: &str) {
    let mut parts = input.split_whitespace();
    let Some(command) = parts.next() else {
      return;
    };
    let args: Vec<&str> = parts.collect();
    match command {
      "quit" | "q" => self.quit = true,
      "help" => self.show_help = true,
      "play" => self.mpdc(MpdCommand::PlayPauseToggle),
      "pause" => self.mpdc(MpdCommand::Pause(true)),
      "toggle" => self.mpdc(MpdCommand::PlayPauseToggle),
      "stop" => self.mpdc(MpdCommand::Stop),
      "next" => self.mpdc(MpdCommand::Next),
      "prev" => self.mpdc(MpdCommand::Previous),
      "volume" | "vol" => match args.first() {
        Some(value) => {
          if let Some(delta) = value.strip_prefix(['+', '-']) {
            let magnitude: i16 = delta.parse().unwrap_or(0);
            let signed = if value.starts_with('-') {
              -magnitude
            } else {
              magnitude
            };
            self.mpdc(MpdCommand::NudgeVolume(signed));
          } else if let Ok(volume) = value.parse::<u8>() {
            self.mpdc(MpdCommand::SetVolume(volume.min(100)));
          } else {
            self.set_message(format!("invalid volume: {value}"));
          }
        }
        None => {
          let volume = self.status.as_ref().map(|status| status.volume);
          self.set_message(format!("volume: {}%", volume.unwrap_or(0)));
        }
      },
      "repeat" => self.mpdc(MpdCommand::SetRepeat(self.toggle_flag("repeat"))),
      "random" => self.mpdc(MpdCommand::SetRandom(self.toggle_flag("random"))),
      "single" => self.mpdc(MpdCommand::SetSingle(self.toggle_single())),
      "consume" => self.mpdc(MpdCommand::SetConsume(self.toggle_flag("consume"))),
      "dedup" => {
        self.run_action("queue_dedup");
      }
      "clear" => self.mpdc(MpdCommand::ClearQueue),
      "update" => self.mpdc(MpdCommand::Rescan),
      "tab" => self.command_tab(&args),
      "add" => self.command_add(&args),
      "save" => self.command_save(args.first().copied()),
      other => self.set_message(format!("unknown command: {other}")),
    }
  }

  fn command_tab(&mut self, args: &[&str]) {
    let Some(target) = args.first() else {
      let names: Vec<String> = self
        .tabs
        .iter()
        .enumerate()
        .map(|(index, tab)| format!("{}) {}", index + 1, tab.name))
        .collect();
      self.set_message(format!("tabs: {}", names.join("  ")));
      return;
    };
    if let Ok(index) = target.parse::<usize>()
      && self.goto_tab(index - 1)
    {
      return;
    }
    if let Some(index) = self.tabs.iter().position(|tab| tab.name == *target) {
      self.goto_tab(index);
      return;
    }
    self.set_message(format!("no such tab: {target}"));
  }

  fn command_add(&mut self, args: &[&str]) {
    let Some(target) = args.first() else {
      self.set_message("usage: add <path>");
      return;
    };
    let path = expand_home(target);
    let resolved = if path.is_absolute() {
      path
    } else {
      let Some(music_dir) = self.music_dir.as_ref() else {
        self.set_message("relative paths require a configured music directory");
        return;
      };
      music_dir.join(path)
    };
    let canonical = match resolved.canonicalize() {
      Ok(canonical) => canonical,
      Err(_) => {
        self.set_message(format!("path not found: {}", resolved.display()));
        return;
      }
    };
    if canonical.is_dir() {
      let recursive = args.iter().any(|arg| *arg == "--recursive" || *arg == "-r");
      let files = match crate::library::collect_audio_files(&canonical, recursive) {
        Ok(files) => files,
        Err(error) => {
          self.set_message(format!("scan failed: {error}"));
          return;
        }
      };
      let mut count = 0;
      for file in files {
        if let Some(uri) =
          crate::open::direct_open_uri(&file, &self.settings.config.mpd, self.music_dir.as_deref())
        {
          self.mpdc(MpdCommand::AddUri(uri));
          count += 1;
        }
      }
      if count == 0 {
        self.set_message("local files over TCP require a configured music directory");
      } else {
        self.set_message(format!("queued {count} song(s)"));
      }
    } else if let Some(uri) = crate::open::direct_open_uri(
      &canonical,
      &self.settings.config.mpd,
      self.music_dir.as_deref(),
    ) {
      self.mpdc(MpdCommand::AddUri(uri));
      self.set_message(format!("queued {}", canonical.display()));
    } else {
      self.set_message("local files over TCP require a configured music directory");
    }
  }
  /// `:save [path]` — export the current queue as an m3u8 file. Bare file
  /// names resolve under `playlist.save_dir` (default
  /// `~/.local/state/music-tui/playlists`); relative paths are rejected.
  fn command_save(&mut self, arg: Option<&str>) {
    let save_dir = self.settings.config.playlist.effective_save_dir();
    let target = match crate::playlist::resolve_save_path(arg, &save_dir) {
      Ok(target) => target,
      Err(error) => {
        self.set_message(error);
        return;
      }
    };
    let mut body = String::from("#EXTM3U\n");
    let mut written = 0;
    for song in &self.queue {
      let song = &song.song;
      let artist = song_artist(song).map(str::to_string).unwrap_or_default();
      let label = match (artist.is_empty(), song_title(song)) {
        (false, Some(title)) => format!("{artist} - {title}"),
        (true, Some(title)) => title.to_string(),
        _ => song.url.clone(),
      };
      let seconds = song
        .duration
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
      let path = crate::library::uri_to_path(self.music_dir.as_deref(), &song.url);
      if let Some(path) = path {
        body.push_str(&format!("#EXTINF:{seconds},{label}\n{}\n", path.display()));
        written += 1;
      }
    }
    if let Some(parent) = target.parent()
      && let Err(error) = std::fs::create_dir_all(parent)
    {
      self.set_message(format!("save failed: {error}"));
      return;
    }
    match std::fs::write(&target, body) {
      Ok(()) => self.set_message(format!("saved {written} song(s) to {}", target.display())),
      Err(error) => self.set_message(format!("save failed: {error}")),
    }
  }
}
