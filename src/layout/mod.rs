//! Configurable tab/pane layout.
//!
//! Each tab is described by a small layout DSL:
//!
//! ```text
//! H(2:1, queue, V(2:1, cover, metadata))
//! ```
//!
//! - `H(a:b, left, right)` splits horizontally (side by side) with the width
//!   shared `a:b`.
//! - `V(a:b, top, bottom)` splits vertically with the height shared `a:b`.
//! - leaf panes: `queue`, `cover`, `lyrics`, `metadata`, `visualizer`.
//!
//! `cover`, `lyrics` and `metadata` panes take an optional **data source**
//! suffix: `cover:hovered` shows the cover of the song hovered in the queue
//! instead of the playing song (`lyrics:hovered` has no playback state, so
//! it renders as a plain scrollable list). The default source is `playing`.
//!
//! A tab's keymap is decided by its **main pane** (see `TabLayout::main`):
//! keys always dispatch to the main pane's bindings, which take priority over
//! global bindings. Other panes in the same tab are display-only.

use crate::config::{LayoutConfig, TabConfig};

mod parser;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneKind {
  Queue,
  Library,
  Cover,
  Lyrics,
  Metadata,
  Visualizer,
}

impl PaneKind {
  pub fn parse(value: &str) -> Option<Self> {
    match value.trim() {
      "queue" => Some(Self::Queue),
      "library" => Some(Self::Library),
      "cover" => Some(Self::Cover),
      "lyrics" => Some(Self::Lyrics),
      "metadata" => Some(Self::Metadata),
      "visualizer" => Some(Self::Visualizer),
      _ => None,
    }
  }

  pub fn title(self) -> &'static str {
    match self {
      PaneKind::Queue => "queue",
      PaneKind::Library => "library",
      PaneKind::Cover => "cover",
      PaneKind::Lyrics => "lyrics",
      PaneKind::Metadata => "metadata",
      PaneKind::Visualizer => "visualizer",
    }
  }

  pub fn index(self) -> usize {
    self as usize
  }
}

/// Where a display pane gets its song data from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PaneSource {
  /// The song currently playing (the default).
  #[default]
  Playing,
  /// The song hovered (selected) in the queue pane. No playback state:
  /// lyrics render without sync highlight and seeking is disabled.
  QueueHovered,
  /// The track hovered (selected) in the library pane. No playback state
  /// either.
  LibraryHovered,
}

impl PaneSource {
  pub fn parse(value: &str) -> Option<Self> {
    match value.trim() {
      "playing" => Some(Self::Playing),
      "queue-hovered" | "queue" | "hovered" | "hover" => Some(Self::QueueHovered),
      "library-hovered" | "library" | "lib" => Some(Self::LibraryHovered),
      _ => None,
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDir {
  /// Side-by-side split; children share the width.
  Horizontal,
  /// Stacked split; children share the height.
  Vertical,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaneLayout {
  Pane(PaneKind, PaneSource),
  Split {
    dir: SplitDir,
    ratio: (u32, u32),
    first: Box<PaneLayout>,
    second: Box<PaneLayout>,
  },
}

impl PaneLayout {
  pub fn contains(&self, kind: PaneKind) -> bool {
    match self {
      PaneLayout::Pane(pane, _) => *pane == kind,
      PaneLayout::Split { first, second, .. } => first.contains(kind) || second.contains(kind),
    }
  }

  /// Panes in layout (left-to-right, top-to-bottom) order, with duplicates
  /// if a pane kind appears more than once.
  pub fn pane_kinds(&self) -> Vec<PaneKind> {
    let mut panes = Vec::new();
    collect_panes(self, &mut panes);
    panes
  }

  pub fn first_pane(&self) -> PaneKind {
    match self {
      PaneLayout::Pane(kind, _) => *kind,
      PaneLayout::Split { first, .. } => first.first_pane(),
    }
  }

  /// Source of the first pane matching `kind` (leftmost / topmost wins).
  pub fn source_of(&self, kind: PaneKind) -> Option<PaneSource> {
    match self {
      PaneLayout::Pane(pane, source) => (*pane == kind).then_some(*source),
      PaneLayout::Split { first, second, .. } => {
        first.source_of(kind).or_else(|| second.source_of(kind))
      }
    }
  }

  /// Whether any pane uses the given data source.
  pub fn has_source(&self, source: PaneSource) -> bool {
    match self {
      PaneLayout::Pane(_, pane_source) => *pane_source == source,
      PaneLayout::Split { first, second, .. } => {
        first.has_source(source) || second.has_source(source)
      }
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabLayout {
  pub name: String,
  pub layout: PaneLayout,
  pub main: PaneKind,
}

pub fn parse_layout(spec: &str) -> Result<PaneLayout, String> {
  parser::parse(spec)
}

/// Parse a `[[layout.tabs]]` config into runtime layouts.
pub fn parse_tabs(config: &LayoutConfig) -> Result<Vec<TabLayout>, String> {
  if config.tabs.is_empty() {
    return Err("layout needs at least one tab".to_string());
  }
  config
    .tabs
    .iter()
    .map(|tab| parse_tab(tab).map_err(|error| format!("tab {:?}: {error}", tab.name)))
    .collect()
}

/// Default secondary detail-view layout: cover left, metadata right.
pub const DEFAULT_DETAIL_LAYOUT: &str = "H(2:1, cover, metadata)";

/// Parse the `[layout].detail` spec. Only the `cover` and `metadata` panes
/// are allowed, each exactly once.
pub fn parse_detail(spec: &str) -> Result<PaneLayout, String> {
  let layout = parse_layout(spec)?;
  let mut panes = Vec::new();
  collect_panes(&layout, &mut panes);
  if panes.len() != 2 || !panes.contains(&PaneKind::Cover) || !panes.contains(&PaneKind::Metadata) {
    return Err(format!(
      "detail layout must contain exactly one cover and one metadata pane, got {spec:?}"
    ));
  }
  Ok(layout)
}

fn collect_panes(layout: &PaneLayout, panes: &mut Vec<PaneKind>) {
  match layout {
    PaneLayout::Pane(kind, _) => panes.push(*kind),
    PaneLayout::Split { first, second, .. } => {
      collect_panes(first, panes);
      collect_panes(second, panes);
    }
  }
}

fn parse_tab(tab: &TabConfig) -> Result<TabLayout, String> {
  let layout = parse_layout(&tab.layout)?;
  let main = match tab.main.as_deref() {
    Some(name) => PaneKind::parse(name).ok_or_else(|| format!("unknown main pane {name:?}"))?,
    None => layout.first_pane(),
  };
  if !layout.contains(main) {
    return Err(format!(
      "main pane {:?} is not part of the layout",
      main.title()
    ));
  }
  Ok(TabLayout {
    name: tab.name.clone(),
    layout,
    main,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_nested_splits() {
    let layout = parse_layout("H(2:1, queue, V(2:1, cover, metadata))").expect("valid layout");
    assert!(matches!(layout, PaneLayout::Split { .. }));
    assert!(layout.contains(PaneKind::Queue));
    assert!(layout.contains(PaneKind::Metadata));
    assert!(!layout.contains(PaneKind::Visualizer));
    assert_eq!(layout.first_pane(), PaneKind::Queue);
  }

  #[test]
  fn parses_single_pane() {
    let layout = parse_layout(" visualizer ").expect("valid layout");
    assert_eq!(
      layout,
      PaneLayout::Pane(PaneKind::Visualizer, PaneSource::Playing)
    );
  }

  #[test]
  fn rejects_unknown_panes_and_bad_syntax() {
    assert!(parse_layout("nope").is_err());
    assert!(parse_layout("H(queue)").is_err());
    assert!(parse_layout("H(0:1, queue, cover)").is_err());
    assert!(parse_layout("queue extra").is_err());
  }

  #[test]
  fn default_config_parses() {
    let tabs = parse_tabs(&LayoutConfig::default()).expect("default layouts");
    #[cfg(unix)]
    assert_eq!(tabs.len(), 6);
    #[cfg(windows)]
    assert_eq!(tabs.len(), 5);
    assert_eq!(tabs[0].main, PaneKind::Queue);
    assert!(tabs[0].layout.contains(PaneKind::Cover));
    assert!(tabs[0].layout.contains(PaneKind::Metadata));
    assert_eq!(tabs[1].main, PaneKind::Library);
    assert!(tabs[1].layout.contains(PaneKind::Cover));
    assert!(tabs[1].layout.contains(PaneKind::Metadata));
    assert_eq!(tabs[2].main, PaneKind::Cover);
    assert!(tabs[2].layout.contains(PaneKind::Lyrics));
  }

  #[test]
  fn parses_hovered_sources() {
    let layout = parse_layout("V(2:1, cover:hovered, lyrics:hover)").expect("valid layout");
    assert_eq!(
      layout.source_of(PaneKind::Cover),
      Some(PaneSource::QueueHovered)
    );
    assert_eq!(
      layout.source_of(PaneKind::Lyrics),
      Some(PaneSource::QueueHovered)
    );
    assert!(layout.has_source(PaneSource::QueueHovered));
    let plain = parse_layout("cover").expect("valid layout");
    assert_eq!(plain.source_of(PaneKind::Cover), Some(PaneSource::Playing));
    assert!(!plain.has_source(PaneSource::QueueHovered));
  }

  #[test]
  fn rejects_bad_sources() {
    assert!(parse_layout("cover:nonsense").is_err());
    assert!(parse_layout("queue:hovered").is_err());
    assert!(parse_layout("visualizer:hovered").is_err());
  }

  #[test]
  fn main_must_be_in_layout() {
    let tab = TabConfig {
      name: "bad".to_string(),
      layout: "queue".to_string(),
      main: Some("lyrics".to_string()),
    };
    let config = LayoutConfig {
      detail: DEFAULT_DETAIL_LAYOUT.to_string(),
      tabs: vec![tab],
    };
    assert!(parse_tabs(&config).is_err());
  }
}
