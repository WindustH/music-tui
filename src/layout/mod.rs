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
//! - leaf panes: `queue`, `library`, `cover`, `lyrics`, `metadata`,
//!   `visualizer`.
//!
//! `cover`, `lyrics` and `metadata` panes take an optional **data source**
//! suffix: `cover:hovered` shows the cover of the song hovered in the queue
//! (`:library-hovered`: the library selection) instead of the playing song
//! (hovered lyrics have no playback state, so they render as a plain
//! scrollable list). The default source is `playing`.
//!
//! Keys dispatch to the **main pane** first (see `TabLayout::main`), then
//! fall through to the tab's other panes; `global` bindings win in every
//! pane unless the pane binds the very same key itself.

mod config;
pub use config::{LayoutConfig, TabConfig};

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
  /// Every kind, in `index` order.
  pub const ALL: [PaneKind; 6] = [
    Self::Queue,
    Self::Library,
    Self::Cover,
    Self::Lyrics,
    Self::Metadata,
    Self::Visualizer,
  ];

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
