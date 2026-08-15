//! KeyBindings construction from the keymap config.

use super::*;

pub(crate) fn build_bindings(keymap: &KeymapConfig) -> Vec<KeyBindings> {
  vec![
    keymap.queue_bindings(),
    keymap.library_bindings(),
    keymap.cover_bindings(),
    keymap.lyrics_bindings(),
    keymap.metadata_bindings(),
    keymap.visualizer_bindings(),
  ]
}

pub(crate) fn build_input_bindings(keymap: &KeymapConfig) -> KeyBindings {
  keymap.input_only_bindings()
}