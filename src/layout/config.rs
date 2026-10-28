//! Serialized layouts and application defaults.
use serde::{Deserialize, Serialize};
/// Top-level tab configuration. Each `[[layout.tabs]]` entry describes one
/// tab shown in the tab bar; `detail` describes the secondary detail view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LayoutConfig {
  pub detail: String,
  pub tabs: Vec<TabConfig>,
}

impl Default for LayoutConfig {
  fn default() -> Self {
    Self::with_default_tabs()
  }
}

impl LayoutConfig {
  pub(crate) fn normalize_defaults(&mut self) {
    if self.detail.trim().is_empty() {
      self.detail = super::DEFAULT_DETAIL_LAYOUT.to_string();
    }
    if self.tabs.is_empty() {
      *self = LayoutConfig::with_default_tabs();
    }
  }

  fn with_default_tabs() -> Self {
    let tabs = vec![
      TabConfig::playlist(),
      TabConfig::library(),
      TabConfig::playing(),
      TabConfig::metadata(),
      TabConfig::lyrics(),
    ];
    #[cfg(unix)]
    let tabs = {
      let mut tabs = tabs;
      tabs.push(TabConfig::visualizer());
      tabs
    };
    Self {
      detail: super::DEFAULT_DETAIL_LAYOUT.to_string(),
      tabs,
    }
  }
}

/// One tab: a layout tree plus the pane whose keymap is active.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabConfig {
  /// Name shown in the tab bar.
  pub name: String,
  /// Layout tree, e.g. `H(2:1, queue, V(2:1, cover, metadata))`.
  pub layout: String,
  /// Pane that receives this tab's keys (`queue`, `library`, `cover`,
  /// `lyrics`, `metadata`, `visualizer`). Defaults to the first pane in the
  /// tree.
  pub main: Option<String>,
}

impl TabConfig {
  fn playlist() -> Self {
    Self {
      name: "playlist".to_string(),
      layout: "H(2:1, queue, V(2:1, cover:hovered, metadata:hovered))".to_string(),
      main: Some("queue".to_string()),
    }
  }

  fn library() -> Self {
    Self {
      name: "library".to_string(),
      layout: "H(2:1, library, V(2:1, cover:library-hovered, metadata:library-hovered))"
        .to_string(),
      main: Some("library".to_string()),
    }
  }

  fn playing() -> Self {
    Self {
      name: "playing".to_string(),
      layout: "H(1:2, cover, lyrics)".to_string(),
      main: Some("cover".to_string()),
    }
  }

  fn metadata() -> Self {
    Self {
      name: "metadata".to_string(),
      layout: "metadata".to_string(),
      main: Some("metadata".to_string()),
    }
  }

  fn lyrics() -> Self {
    Self {
      name: "lyrics".to_string(),
      layout: "lyrics".to_string(),
      main: Some("lyrics".to_string()),
    }
  }

  #[cfg(unix)]
  fn visualizer() -> Self {
    Self {
      name: "visualizer".to_string(),
      layout: "visualizer".to_string(),
      main: Some("visualizer".to_string()),
    }
  }
}
