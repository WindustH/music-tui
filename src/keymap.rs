use std::fmt::Write as FmtWrite;

use framework_tui::{KeyBindingConfig, KeyBindings};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KeymapConfig {
  pub queue: KeymapSection,
  pub metadata: KeymapSection,
  pub cover: KeymapSection,
  pub lyrics: KeymapSection,
  pub visualizer: KeymapSection,
  pub input: KeymapSection,
  pub global: KeymapSection,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct KeymapSection {
  pub keymap: Vec<KeymapEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeymapEntry {
  pub on: KeymapOn,
  pub run: String,
  pub desc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum KeymapOn {
  One(String),
  Many(Vec<String>),
}

impl Default for KeymapConfig {
  fn default() -> Self {
    Self {
      queue: KeymapSection {
        keymap: vec![
          key("q", "quit", "Quit music-tui"),
          key("f1", "help", "Show queue key bindings"),
          key("j", "queue_down", "Move selection down"),
          key("down", "queue_down", "Move selection down"),
          key("k", "queue_up", "Move selection up"),
          key("up", "queue_up", "Move selection up"),
          key("pgdn", "queue_page_down", "Move selection one page down"),
          key("pagedown", "queue_page_down", "Move selection one page down"),
          key("pgup", "queue_page_up", "Move selection one page up"),
          key("pageup", "queue_page_up", "Move selection one page up"),
          key(["g", "g"], "queue_top", "Move selection to top"),
          key("home", "queue_top", "Move selection to top"),
          key("G", "queue_end", "Move selection to end"),
          key("end", "queue_end", "Move selection to end"),
          key("enter", "queue_play", "Play selected song"),
          key("d", "queue_delete", "Remove selected song from queue"),
          key("D", "queue_clear", "Clear the queue"),
        ],
      },
      metadata: KeymapSection {
        keymap: vec![
          key("q", "back", "Return to queue view"),
          key("esc", "back", "Return to queue view"),
          key("f1", "help", "Show metadata key bindings"),
          key("e", "edit_metadata", "Edit metadata in $EDITOR"),
          key("j", "scroll_down", "Scroll metadata down"),
          key("down", "scroll_down", "Scroll metadata down"),
          key("k", "scroll_up", "Scroll metadata up"),
          key("up", "scroll_up", "Scroll metadata up"),
          key("pgdn", "page_down", "Scroll metadata page down"),
          key("pagedown", "page_down", "Scroll metadata page down"),
          key("pgup", "page_up", "Scroll metadata page up"),
          key("pageup", "page_up", "Scroll metadata page up"),
        ],
      },
      cover: KeymapSection {
        keymap: vec![
          key("q", "back", "Return to queue view"),
          key("esc", "back", "Return to queue view"),
          key("f1", "help", "Show cover key bindings"),
        ],
      },
      lyrics: KeymapSection {
        keymap: vec![
          key("q", "back", "Return to queue view"),
          key("esc", "back", "Return to queue view"),
          key("f1", "help", "Show lyrics key bindings"),
          key("j", "lyrics_down", "Scroll lyrics down"),
          key("down", "lyrics_down", "Scroll lyrics down"),
          key("k", "lyrics_up", "Scroll lyrics up"),
          key("up", "lyrics_up", "Scroll lyrics up"),
          key("pgdn", "lyrics_page_down", "Scroll lyrics page down"),
          key("pagedown", "lyrics_page_down", "Scroll lyrics page down"),
          key("pgup", "lyrics_page_up", "Scroll lyrics page up"),
          key("pageup", "lyrics_page_up", "Scroll lyrics page up"),
          key("f", "lyrics_follow", "Toggle auto-follow playback"),
        ],
      },
      visualizer: KeymapSection {
        keymap: vec![
          key("q", "back", "Return to queue view"),
          key("esc", "back", "Return to queue view"),
          key("f1", "help", "Show visualizer key bindings"),
        ],
      },
      input: default_input_keymap_section(),
      global: KeymapSection {
        keymap: vec![
          key(":", "command", "Enter command"),
          key("ctrl-c", "quit", "Quit music-tui"),
          key("left", "tab_previous", "Switch to previous tab"),
          key("right", "tab_next", "Switch to next tab"),
          key("[", "tab_previous", "Switch to previous tab"),
          key("]", "tab_next", "Switch to next tab"),
          key("1", "tab_goto_1", "Switch to tab 1"),
          key("2", "tab_goto_2", "Switch to tab 2"),
          key("3", "tab_goto_3", "Switch to tab 3"),
          key("4", "tab_goto_4", "Switch to tab 4"),
          key("5", "tab_goto_5", "Switch to tab 5"),
          key("6", "tab_goto_6", "Switch to tab 6"),
          key("7", "tab_goto_7", "Switch to tab 7"),
          key("8", "tab_goto_8", "Switch to tab 8"),
          key("9", "tab_goto_9", "Switch to tab 9"),
          // Playback controls: active with priority in every view.
          key("space", "play_pause", "Toggle play or pause"),
          key("x", "play_pause", "Toggle play or pause"),
          key("n", "next", "Next song"),
          key("p", "previous", "Previous song"),
          key("s", "stop", "Stop playback"),
          key("+", "volume_up", "Increase volume"),
          key("=", "volume_up", "Increase volume"),
          key("-", "volume_down", "Decrease volume"),
          key("m", "volume_mute", "Toggle mute"),
          key("h", "seek_back", "Seek 5 seconds back"),
          key("l", "seek_forward", "Seek 5 seconds forward"),
          key("H", "seek_back_long", "Seek 30 seconds back"),
          key("L", "seek_forward_long", "Seek 30 seconds forward"),
          key("r", "toggle_repeat", "Toggle repeat"),
          key("t", "toggle_random", "Toggle random"),
          key("y", "cycle_single", "Cycle single mode"),
          key("c", "toggle_consume", "Toggle consume"),
        ],
      },
    }
  }
}

impl KeymapConfig {
  pub fn queue_bindings(&self) -> KeyBindings {
    KeyBindings::from_sections(
      binding_configs(&self.queue.keymap),
      Vec::<KeyBindingConfig>::new(),
      binding_configs(&self.input.keymap),
      binding_configs(&self.global.keymap),
    )
  }

  pub fn metadata_bindings(&self) -> KeyBindings {
    KeyBindings::from_sections(
      binding_configs(&self.metadata.keymap),
      Vec::<KeyBindingConfig>::new(),
      binding_configs(&self.input.keymap),
      binding_configs(&self.global.keymap),
    )
  }

  pub fn cover_bindings(&self) -> KeyBindings {
    KeyBindings::from_sections(
      binding_configs(&self.cover.keymap),
      Vec::<KeyBindingConfig>::new(),
      binding_configs(&self.input.keymap),
      binding_configs(&self.global.keymap),
    )
  }

  pub fn lyrics_bindings(&self) -> KeyBindings {
    KeyBindings::from_sections(
      binding_configs(&self.lyrics.keymap),
      Vec::<KeyBindingConfig>::new(),
      binding_configs(&self.input.keymap),
      binding_configs(&self.global.keymap),
    )
  }

  pub fn visualizer_bindings(&self) -> KeyBindings {
    KeyBindings::from_sections(
      binding_configs(&self.visualizer.keymap),
      Vec::<KeyBindingConfig>::new(),
      binding_configs(&self.input.keymap),
      binding_configs(&self.global.keymap),
    )
  }

  /// Input-context bindings only: the input section without global keys, so
  /// typing never triggers playback shortcuts.
  pub fn input_only_bindings(&self) -> KeyBindings {
    KeyBindings::from_sections(
      Vec::<KeyBindingConfig>::new(),
      Vec::<KeyBindingConfig>::new(),
      binding_configs(&self.input.keymap),
      Vec::<KeyBindingConfig>::new(),
    )
  }

  pub(crate) fn normalize_defaults(&mut self) {
    let default = KeymapConfig::default();
    append_missing_actions(&mut self.queue.keymap, &default.queue.keymap);
    append_missing_actions(&mut self.metadata.keymap, &default.metadata.keymap);
    append_missing_actions(&mut self.cover.keymap, &default.cover.keymap);
    append_missing_actions(&mut self.lyrics.keymap, &default.lyrics.keymap);
    append_missing_actions(&mut self.visualizer.keymap, &default.visualizer.keymap);
    append_missing_actions(&mut self.input.keymap, &default.input.keymap);
    append_missing_actions(&mut self.global.keymap, &default.global.keymap);
  }
}

pub(crate) fn format_keymap_toml(config: &KeymapConfig) -> String {
  let mut out = String::new();
  push_keymap_section(&mut out, "queue", &config.queue);
  push_keymap_section(&mut out, "metadata", &config.metadata);
  push_keymap_section(&mut out, "cover", &config.cover);
  push_keymap_section(&mut out, "lyrics", &config.lyrics);
  push_keymap_section(&mut out, "visualizer", &config.visualizer);
  push_keymap_section(&mut out, "input", &config.input);
  push_keymap_section(&mut out, "global", &config.global);
  out
}

fn binding_configs(entries: &[KeymapEntry]) -> Vec<KeyBindingConfig> {
  entries
    .iter()
    .map(|entry| KeyBindingConfig {
      on: keymap_on_values(&entry.on),
      action: entry.run.clone(),
      desc: entry.desc.clone(),
    })
    .collect()
}

fn keymap_on_values(on: &KeymapOn) -> Vec<String> {
  match on {
    KeymapOn::One(value) => vec![value.clone()],
    KeymapOn::Many(values) => values.clone(),
  }
}

fn append_missing_actions(entries: &mut Vec<KeymapEntry>, defaults: &[KeymapEntry]) {
  for default in defaults {
    if entries.iter().any(|entry| entry.run == default.run) {
      continue;
    }
    entries.push(default.clone());
  }
}

fn default_input_keymap_section() -> KeymapSection {
  KeymapSection {
    keymap: vec![
      key("esc", "cancel", "Cancel input"),
      key("f1", "help", "Show input key bindings"),
      key("enter", "submit", "Submit input"),
      key("backspace", "backspace", "Delete before cursor"),
      key("delete", "delete", "Delete under cursor"),
      key("left", "move_left", "Move cursor left"),
      key("right", "move_right", "Move cursor right"),
      key("home", "move_start", "Move cursor to start"),
      key("ctrl-a", "move_start", "Move cursor to start"),
      key("end", "move_end", "Move cursor to end"),
      key("ctrl-e", "move_end", "Move cursor to end"),
      key("ctrl-u", "kill_before_cursor", "Delete before cursor"),
      key("ctrl-k", "kill_after_cursor", "Delete after cursor"),
      key("tab", "completion_next", "Select next completion"),
      key("backtab", "completion_previous", "Select previous completion"),
      key("up", "history_previous", "Previous command history"),
      key("down", "history_next", "Next command history"),
    ],
  }
}

fn key(on: impl Into<KeymapOn>, run: &str, desc: &str) -> KeymapEntry {
  KeymapEntry {
    on: on.into(),
    run: run.to_string(),
    desc: desc.to_string(),
  }
}

impl From<&str> for KeymapOn {
  fn from(value: &str) -> Self {
    Self::One(value.to_string())
  }
}

impl<const N: usize> From<[&str; N]> for KeymapOn {
  fn from(value: [&str; N]) -> Self {
    Self::Many(value.into_iter().map(str::to_string).collect())
  }
}

fn push_keymap_section(out: &mut String, name: &str, section: &KeymapSection) {
  let _ = writeln!(out, "[{name}]");
  out.push_str("keymap = [\n");
  for entry in &section.keymap {
    let _ = writeln!(
      out,
      "  {{ on = {}, run = {}, desc = {} }},",
      format_keymap_on(&entry.on),
      toml_basic_string(&entry.run),
      toml_basic_string(&entry.desc)
    );
  }
  out.push_str("]\n\n");
}

fn format_keymap_on(on: &KeymapOn) -> String {
  match on {
    KeymapOn::One(value) => toml_basic_string(value),
    KeymapOn::Many(values) => {
      let keys = values
        .iter()
        .map(|value| toml_basic_string(value))
        .collect::<Vec<_>>()
        .join(", ");
      format!("[{keys}]")
    }
  }
}

fn toml_basic_string(value: &str) -> String {
  let mut out = String::with_capacity(value.len() + 2);
  out.push('"');
  for ch in value.chars() {
    match ch {
      '\\' => out.push_str("\\\\"),
      '"' => out.push_str("\\\""),
      '\n' => out.push_str("\\n"),
      '\r' => out.push_str("\\r"),
      '\t' => out.push_str("\\t"),
      '\u{08}' => out.push_str("\\b"),
      '\u{0c}' => out.push_str("\\f"),
      ch if ch.is_control() => {
        let _ = write!(out, "\\u{:04X}", ch as u32);
      }
      ch => out.push(ch),
    }
  }
  out.push('"');
  out
}
