use std::collections::HashSet;

use framework_tui::keymap::{
  InputKeymapOptions, KeyBindingConfig, KeyBindings, KeymapEntry, KeymapOn, KeymapSection,
  default_input_keymap, format_keymap_sections, key,
};

use crate::layout::PaneKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KeymapConfig {
  pub queue: KeymapSection,
  pub library: KeymapSection,
  pub metadata: KeymapSection,
  pub cover: KeymapSection,
  pub lyrics: KeymapSection,
  pub visualizer: KeymapSection,
  pub input: KeymapSection,
  pub help: KeymapSection,
  pub global: KeymapSection,
}

impl Default for KeymapConfig {
  fn default() -> Self {
    Self {
      queue: KeymapSection {
        keymap: vec![
          key("esc", "back", "Clear filter or return to first tab"),
          key("f1", "help", "Show queue key bindings"),
          key("j", "queue_down", "Move selection down"),
          key("down", "queue_down", "Move selection down"),
          key("k", "queue_up", "Move selection up"),
          key("up", "queue_up", "Move selection up"),
          key("pgdn", "queue_page_down", "Move selection one page down"),
          key(
            "pagedown",
            "queue_page_down",
            "Move selection one page down",
          ),
          key("pgup", "queue_page_up", "Move selection one page up"),
          key("pageup", "queue_page_up", "Move selection one page up"),
          key(
            ["g", "c"],
            "queue_goto_playing",
            "Jump to the currently playing song",
          ),
          key(["g", "g"], "queue_top", "Move selection to top"),
          key("home", "queue_top", "Move selection to top"),
          key("G", "queue_end", "Move selection to end"),
          key("end", "queue_end", "Move selection to end"),
          key("enter", "queue_play", "Play selected song"),
          key("d", "queue_delete", "Remove selected song from queue"),
          key("D", "queue_clear", "Clear the queue"),
          key("?", "queue_shuffle", "Shuffle the queue"),
          key(
            [",", "d"],
            "queue_dedup",
            "Toggle hiding duplicate queue entries",
          ),
          key("i", "queue_detail", "Open details of the selected song"),
          key("e", "edit_metadata", "Edit the selected song's metadata"),
          key("/", "queue_filter", "Filter the queue (esc clears)"),
        ],
      },
      library: KeymapSection {
        keymap: vec![
          key("esc", "back", "Clear filter or return to first tab"),
          key("f1", "help", "Show library key bindings"),
          key("j", "library_down", "Move selection down"),
          key("down", "library_down", "Move selection down"),
          key("k", "library_up", "Move selection up"),
          key("up", "library_up", "Move selection up"),
          key("pgdn", "library_page_down", "Move selection one page down"),
          key(
            "pagedown",
            "library_page_down",
            "Move selection one page down",
          ),
          key("pgup", "library_page_up", "Move selection one page up"),
          key("pageup", "library_page_up", "Move selection one page up"),
          key(["g", "g"], "library_top", "Move selection to top"),
          key("home", "library_top", "Move selection to top"),
          key("G", "library_end", "Move selection to end"),
          key("end", "library_end", "Move selection to end"),
          key(
            "enter",
            "library_play",
            "Play selected track (replaces play position)",
          ),
          key("a", "library_append", "Append selected track to the queue"),
          key("i", "library_detail", "Open details of the selected track"),
          key("u", "library_rescan", "Rescan the library database"),
          key("/", "library_filter", "Filter the library (esc clears)"),
        ],
      },
      metadata: KeymapSection {
        keymap: vec![
          // `q` is not bound here: the global `quit` takes priority in
          // every view (opt-in global-priority matching).
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
          // `q` is not bound here: the global `quit` takes priority in
          // every view (opt-in global-priority matching).
          key("esc", "back", "Return to queue view"),
          key("f1", "help", "Show cover key bindings"),
        ],
      },
      lyrics: KeymapSection {
        keymap: vec![
          // `q` is not bound here: the global `quit` takes priority in
          // every view (opt-in global-priority matching).
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
          key("F", "lyrics_follow", "Toggle auto-follow playback"),
          key("enter", "lyrics_jump", "Seek to the selected lyric line"),
        ],
      },
      visualizer: KeymapSection {
        keymap: vec![
          // `q` is not bound here: the global `quit` takes priority in
          // every view (opt-in global-priority matching).
          key("esc", "back", "Return to queue view"),
          key("f1", "help", "Show visualizer key bindings"),
        ],
      },
      input: default_input_keymap(&InputKeymapOptions {
        help: Some("Show input key bindings".to_string()),
        help_after_cancel: true,
        ..InputKeymapOptions::default()
      }),
      help: KeymapSection {
        keymap: vec![
          key("pgdn", "page_down", "Scroll help one page down"),
          key("pagedown", "page_down", "Scroll help one page down"),
          key("pgup", "page_up", "Scroll help one page up"),
          key("pageup", "page_up", "Scroll help one page up"),
          key("j", "scroll_down", "Scroll help down"),
          key("down", "scroll_down", "Scroll help down"),
          key("k", "scroll_up", "Scroll help up"),
          key("up", "scroll_up", "Scroll help up"),
        ],
      },
      global: KeymapSection {
        keymap: vec![
          key(":", "command", "Enter command"),
          key("q", "quit", "Quit music-tui"),
          key("ctrl-c", "quit", "Quit music-tui"),
          // Tab switching: letter-zone left/right plus arrows, cycling.
          key("a", "tab_previous", "Switch to previous tab"),
          key("f", "tab_next", "Switch to next tab"),
          key("h", "tab_previous", "Switch to previous tab"),
          key("l", "tab_next", "Switch to next tab"),
          key("left", "tab_previous", "Switch to previous tab"),
          key("right", "tab_next", "Switch to next tab"),
          key("tab", "tab_next", "Switch to next tab"),
          key("backtab", "tab_previous", "Switch to previous tab"),
          // Playback controls: active with priority in every view.
          key("[", "previous", "Previous song"),
          key("]", "next", "Next song"),
          key("\\", "play_pause", "Toggle play or pause"),
          key("x", "stop", "Stop playback"),
          key("-", "seek_back", "Seek 5 seconds back"),
          key("=", "seek_forward", "Seek 5 seconds forward"),
          key("_", "seek_back_long", "Seek 30 seconds back"),
          key("+", "seek_forward_long", "Seek 30 seconds forward"),
          key("{", "volume_down", "Decrease volume"),
          key("}", "volume_up", "Increase volume"),
          key("m", "volume_mute", "Toggle mute"),
          key([",", "r"], "toggle_repeat", "Toggle repeat"),
          key([",", "t"], "toggle_random", "Toggle random"),
          key([",", "y"], "cycle_single", "Cycle single mode"),
          key([",", "c"], "toggle_consume", "Toggle consume"),
        ],
      },
    }
  }
}

impl KeymapConfig {
  /// View bindings share one shape: the section's keys + input + global,
  /// with global keys taking priority over view-local ones — except where
  /// the section binds the exact same key sequence itself: that explicit
  /// pane binding wins inside the pane (the library's `a` appends while `a`
  /// switches tabs everywhere else).
  fn view_bindings(&self, section: &KeymapSection) -> KeyBindings {
    let claimed: HashSet<Vec<String>> = section
      .keymap
      .iter()
      .filter_map(|entry| normalized_sequence(&entry.on))
      .collect();
    let global: Vec<KeymapEntry> = self
      .global
      .keymap
      .iter()
      .filter(|entry| normalized_sequence(&entry.on).is_none_or(|keys| !claimed.contains(&keys)))
      .cloned()
      .collect();
    KeyBindings::from_sections(
      section.binding_configs(),
      Vec::new(),
      self.input.binding_configs(),
      global.iter().map(KeyBindingConfig::from),
    )
    .with_global_priority()
  }

  /// Bindings of one pane kind.
  pub fn pane_bindings(&self, pane: PaneKind) -> KeyBindings {
    self.view_bindings(match pane {
      PaneKind::Queue => &self.queue,
      PaneKind::Library => &self.library,
      PaneKind::Cover => &self.cover,
      PaneKind::Lyrics => &self.lyrics,
      PaneKind::Metadata => &self.metadata,
      PaneKind::Visualizer => &self.visualizer,
    })
  }

  /// Input-context bindings only: the input section without global keys, so
  /// typing never triggers playback shortcuts.
  pub fn input_only_bindings(&self) -> KeyBindings {
    KeyBindings::from_sections(
      Vec::<KeyBindingConfig>::new(),
      Vec::<KeyBindingConfig>::new(),
      self.input.binding_configs(),
      Vec::<KeyBindingConfig>::new(),
    )
  }

  /// Key-help dialog bindings: an isolated section so help scroll keys are
  /// user-configurable without leaking into normal views. Any key not bound
  /// here closes the dialog.
  pub fn help_bindings(&self) -> KeyBindings {
    KeyBindings::from_sections(
      self.help.binding_configs(),
      Vec::<KeyBindingConfig>::new(),
      Vec::<KeyBindingConfig>::new(),
      Vec::<KeyBindingConfig>::new(),
    )
  }

  /// Every section with its name in the TOML file, in write order.
  fn sections(&self) -> [(&'static str, &KeymapSection); 9] {
    [
      ("queue", &self.queue),
      ("library", &self.library),
      ("metadata", &self.metadata),
      ("cover", &self.cover),
      ("lyrics", &self.lyrics),
      ("visualizer", &self.visualizer),
      ("input", &self.input),
      ("help", &self.help),
      ("global", &self.global),
    ]
  }

  pub(crate) fn normalize_defaults(&mut self) {
    let default = KeymapConfig::default();
    let sections = [
      (&mut self.queue, &default.queue),
      (&mut self.library, &default.library),
      (&mut self.metadata, &default.metadata),
      (&mut self.cover, &default.cover),
      (&mut self.lyrics, &default.lyrics),
      (&mut self.visualizer, &default.visualizer),
      (&mut self.input, &default.input),
      (&mut self.help, &default.help),
      (&mut self.global, &default.global),
    ];
    for (section, default_section) in sections {
      section.append_missing_actions(default_section);
    }
  }
}

pub(crate) fn format_keymap_toml(config: &KeymapConfig) -> String {
  format_keymap_sections(config.sections())
}

/// The key sequence of `on` as canonical tokens (so aliases such as
/// `<C-c>` / `ctrl-c` compare equal); `None` when no key parses.
fn normalized_sequence(on: &KeymapOn) -> Option<Vec<String>> {
  let tokens = on.tokens();
  (!tokens.is_empty()).then_some(tokens)
}

#[cfg(test)]
mod tests {
  use super::*;
  use framework_tui::{KeyContext, KeyDispatcher, MatchResult};

  fn dispatch(bindings: &KeyBindings, keys: &[&str]) -> MatchResult {
    let mut dispatcher = KeyDispatcher::default();
    let mut result = MatchResult::None;
    for key in keys {
      result = dispatcher.dispatch(bindings, KeyContext::Browser, *key);
    }
    result
  }

  #[test]
  fn pane_binding_overrides_the_same_global_key() {
    let keymap = KeymapConfig::default();
    let library = keymap.pane_bindings(PaneKind::Library);
    assert_eq!(
      dispatch(&library, &["a"]),
      MatchResult::Action("library_append".to_string())
    );
    // Other global keys still win in the library pane.
    assert_eq!(
      dispatch(&library, &["q"]),
      MatchResult::Action("quit".to_string())
    );
    // Elsewhere `a` keeps switching tabs.
    assert_eq!(
      dispatch(&keymap.pane_bindings(PaneKind::Queue), &["a"]),
      MatchResult::Action("tab_previous".to_string())
    );
  }

  #[test]
  fn aliases_normalize_to_one_sequence() {
    assert_eq!(
      normalized_sequence(&KeymapOn::One("<C-c>".to_string())),
      normalized_sequence(&KeymapOn::One("ctrl-c".to_string()))
    );
    assert_eq!(normalized_sequence(&KeymapOn::Many(Vec::new())), None);
  }

  #[test]
  fn default_keymap_round_trips_through_toml() {
    let keymap = KeymapConfig::default();
    let parsed: KeymapConfig = toml::from_str(&format_keymap_toml(&keymap)).unwrap();
    assert_eq!(parsed.global.keymap.len(), keymap.global.keymap.len());
    assert_eq!(parsed.library.keymap.len(), keymap.library.keymap.len());
  }
}
