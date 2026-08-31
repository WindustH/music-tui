//! Layout DSL parser: recursive descent over H/V splits and :source panes.

use super::{PaneKind, PaneLayout, PaneSource, SplitDir};

pub(super) fn parse(spec: &str) -> Result<PaneLayout, String> {
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
      let kind = PaneKind::parse(&word).ok_or_else(|| {
        format!("unknown pane {word:?} (expected queue/library/cover/lyrics/metadata/visualizer)")
      })?;
      // Optional `:source` suffix on data panes (cover/lyrics/metadata).
      let source = if tokens.peek() == Some(':') {
        tokens.next();
        let source_word = tokens.read_word();
        let source = PaneSource::parse(&source_word).ok_or_else(|| {
          format!("unknown source {source_word:?} (expected playing/hovered/library)")
        })?;
        if !matches!(
          kind,
          PaneKind::Cover | PaneKind::Lyrics | PaneKind::Metadata
        ) {
          return Err(format!(
            "pane {:?} does not take a data source",
            kind.title()
          ));
        }
        source
      } else {
        PaneSource::Playing
      };
      Ok(PaneLayout::Pane(kind, source))
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
    while matches!(self.peek(), Some(ch) if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-') {
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
