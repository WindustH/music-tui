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
//! A tab's keymap is decided by its **main pane** (see `TabLayout::main`):
//! keys always dispatch to the main pane's bindings, which take priority over
//! global bindings. Other panes in the same tab are display-only.

use crate::config::{LayoutConfig, TabConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneKind {
  Queue,
  Cover,
  Lyrics,
  Metadata,
  Visualizer,
}

impl PaneKind {
  pub const ALL: [PaneKind; 5] = [
    PaneKind::Queue,
    PaneKind::Cover,
    PaneKind::Lyrics,
    PaneKind::Metadata,
    PaneKind::Visualizer,
  ];

  pub fn parse(value: &str) -> Option<Self> {
    match value.trim() {
      "queue" => Some(Self::Queue),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDir {
  /// Side-by-side split; children share the width.
  Horizontal,
  /// Stacked split; children share the height.
  Vertical,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaneLayout {
  Pane(PaneKind),
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
      PaneLayout::Pane(pane) => *pane == kind,
      PaneLayout::Split { first, second, .. } => first.contains(kind) || second.contains(kind),
    }
  }

  pub fn first_pane(&self) -> PaneKind {
    match self {
      PaneLayout::Pane(kind) => *kind,
      PaneLayout::Split { first, .. } => first.first_pane(),
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabLayout {
  pub name: String,
  pub layout: PaneLayout,
  pub main: PaneKind,
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

fn parse_tab(tab: &TabConfig) -> Result<TabLayout, String> {
  let layout = parse_layout(&tab.layout)?;
  let main = match tab.main.as_deref() {
    Some(name) => PaneKind::parse(name)
      .ok_or_else(|| format!("unknown main pane {name:?}"))?,
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

pub fn parse_layout(spec: &str) -> Result<PaneLayout, String> {
  let mut tokens = Tokenizer::new(spec);
  let node = parse_node(&mut tokens)?;
  tokens.expect_end()?;
  Ok(node)
}

fn parse_node(tokens: &mut Tokenizer) -> Result<PaneLayout, String> {
  tokens.skip_whitespace();
  match tokens.peek() {
    Some('H') | Some('V') => parse_split(tokens),
    Some(_) => {
      let word = tokens.read_word();
      let kind = PaneKind::parse(&word)
        .ok_or_else(|| format!("unknown pane {word:?} (expected queue/cover/lyrics/metadata/visualizer)"))?;
      Ok(PaneLayout::Pane(kind))
    }
    None => Err("unexpected end of layout".to_string()),
  }
}

fn parse_split(tokens: &mut Tokenizer) -> Result<PaneLayout, String> {
  let dir = match tokens.next() {
    Some('H') => SplitDir::Horizontal,
    Some('V') => SplitDir::Vertical,
    _ => unreachable!(),
  };
  tokens.skip_whitespace();
  tokens.expect_char('(')?;
  let ratio = parse_ratio(tokens)?;
  tokens.expect_char(',')?;
  let first = parse_node(tokens)?;
  tokens.expect_char(',')?;
  let second = parse_node(tokens)?;
  tokens.expect_char(')')?;
  Ok(PaneLayout::Split {
    dir,
    ratio,
    first: Box::new(first),
    second: Box::new(second),
  })
}

fn parse_ratio(tokens: &mut Tokenizer) -> Result<(u32, u32), String> {
  tokens.skip_whitespace();
  let first = tokens.read_number()?;
  tokens.expect_char(':')?;
  let second = tokens.read_number()?;
  if first == 0 || second == 0 {
    return Err(format!("ratio {first}:{second} must be positive"));
  }
  Ok((first, second))
}

struct Tokenizer {
  chars: Vec<char>,
  pos: usize,
}

impl Tokenizer {
  fn new(spec: &str) -> Self {
    Self {
      chars: spec.chars().collect(),
      pos: 0,
    }
  }

  fn peek(&self) -> Option<char> {
    self.chars.get(self.pos).copied()
  }

  fn next(&mut self) -> Option<char> {
    let ch = self.peek()?;
    self.pos += 1;
    Some(ch)
  }

  fn skip_whitespace(&mut self) {
    while matches!(self.peek(), Some(ch) if ch.is_whitespace()) {
      self.pos += 1;
    }
  }

  fn expect_char(&mut self, expected: char) -> Result<(), String> {
    self.skip_whitespace();
    match self.next() {
      Some(ch) if ch == expected => Ok(()),
      Some(other) => Err(format!("expected {expected:?}, found {other:?}")),
      None => Err(format!("expected {expected:?}, found end of layout")),
    }
  }

  fn expect_end(&mut self) -> Result<(), String> {
    self.skip_whitespace();
    match self.peek() {
      None => Ok(()),
      Some(ch) => Err(format!("unexpected trailing {ch:?} in layout")),
    }
  }

  fn read_word(&mut self) -> String {
    let mut out = String::new();
    while matches!(self.peek(), Some(ch) if ch.is_ascii_alphanumeric() || ch == '_') {
      out.push(self.next().expect("peeked"));
    }
    out
  }

  fn read_number(&mut self) -> Result<u32, String> {
    let mut digits = String::new();
    while matches!(self.peek(), Some(ch) if ch.is_ascii_digit()) {
      digits.push(self.next().expect("peeked"));
    }
    digits
      .parse()
      .map_err(|_| format!("expected a number, found {digits:?}"))
  }
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
    assert_eq!(layout, PaneLayout::Pane(PaneKind::Visualizer));
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
    assert_eq!(tabs.len(), 5);
    assert_eq!(tabs[0].main, PaneKind::Queue);
    assert!(tabs[0].layout.contains(PaneKind::Cover));
    assert!(tabs[0].layout.contains(PaneKind::Metadata));
    assert_eq!(tabs[1].main, PaneKind::Cover);
    assert!(tabs[1].layout.contains(PaneKind::Lyrics));
  }

  #[test]
  fn main_must_be_in_layout() {
    let tab = TabConfig {
      name: "bad".to_string(),
      layout: "queue".to_string(),
      main: Some("lyrics".to_string()),
    };
    let config = LayoutConfig { tabs: vec![tab] };
    assert!(parse_tabs(&config).is_err());
  }
}
