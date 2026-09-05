//! Theme configuration: `~/.config/music-tui/theme.toml`.
//!
//! Colors are grouped per interface section (like the keymap file), so
//! every view's colors are configurable independently. Values are color
//! names (`cyan`, `bright black`, `default`) or `#rrggbb` hex strings.

use serde::{Deserialize, Serialize};

macro_rules! color_section {
  ($name:ident { $($field:ident => $default:literal),* $(,)? }) => {
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(default)]
    pub struct $name {
      $(pub $field: String,)*
    }

    impl Default for $name {
      fn default() -> Self {
        Self {
          $($field: $default.to_string(),)*
        }
      }
    }
  };
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BaseSection {
  pub foreground: String,
  pub background: String,
  pub border: String,
  pub muted: String,
  pub accent: String,
  pub accent_alt: String,
  pub render_background: bool,
}

impl Default for BaseSection {
  fn default() -> Self {
    Self {
      foreground: "default".to_string(),
      background: "default".to_string(),
      border: "bright black".to_string(),
      muted: "bright black".to_string(),
      accent: "cyan".to_string(),
      accent_alt: "magenta".to_string(),
      render_background: false,
    }
  }
}

color_section!(TabBarSection {
  active => "cyan",
  inactive => "bright black",
});

color_section!(QueueSection {
  playing => "green",
  paused => "yellow",
  selection => "cyan",
  highlight => "yellow",
});

color_section!(LibrarySection {
  playing => "green",
  paused => "yellow",
  highlight => "yellow",
  selection_foreground => "black",
  selection_background => "cyan",
  field_primary => "default",
  field_secondary => "magenta",
});

color_section!(FooterSection {
  playing => "green",
  paused => "yellow",
  stopped => "bright black",
  message => "magenta",
});

color_section!(ProgressSection {
  bar => "cyan",
  background => "bright black",
});

color_section!(LyricsSection {
  active => "cyan",
  cursor => "cyan",
});

color_section!(MetadataSection {
  label => "cyan",
});

color_section!(VisualizerSection {
  low => "green",
  mid => "yellow",
  high => "red",
});

/// Which-key hint bar colors. `separator` is the text between the key
/// and its description (`" -> "` by default); `columns` wraps the hints
/// into that many columns when the bar gets crowded.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WhichKeySection {
  pub background: String,
  pub foreground: String,
  pub key: String,
  pub description: String,
  pub separator: String,
  pub separator_color: String,
  pub columns: u16,
}

impl Default for WhichKeySection {
  fn default() -> Self {
    Self {
      background: "reset".to_string(),
      foreground: "white".to_string(),
      key: "light_cyan".to_string(),
      description: "light_magenta".to_string(),
      separator: " -> ".to_string(),
      separator_color: "dark_gray".to_string(),
      columns: 3,
    }
  }
}

/// All colors used across the interface, grouped per view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeConfig {
  pub base: BaseSection,
  pub tab_bar: TabBarSection,
  pub queue: QueueSection,
  pub library: LibrarySection,
  pub footer: FooterSection,
  pub progress: ProgressSection,
  pub lyrics: LyricsSection,
  pub metadata: MetadataSection,
  pub visualizer: VisualizerSection,
  pub which_key: WhichKeySection,
}

impl Default for ThemeConfig {
  fn default() -> Self {
    Self::default_sections()
  }
}

impl ThemeConfig {
  fn default_sections() -> Self {
    Self {
      base: BaseSection::default(),
      tab_bar: TabBarSection::default(),
      queue: QueueSection::default(),
      library: LibrarySection::default(),
      footer: FooterSection::default(),
      progress: ProgressSection::default(),
      lyrics: LyricsSection::default(),
      metadata: MetadataSection::default(),
      visualizer: VisualizerSection::default(),
      which_key: WhichKeySection::default(),
    }
  }
}

impl ThemeConfig {
  pub fn base_background(&self) -> ratatui::style::Color {
    if self.base.render_background {
      self.color(&self.base.background)
    } else {
      ratatui::style::Color::Reset
    }
  }

  pub fn overlay_background(&self) -> ratatui::style::Color {
    let base_bg = self.base_background();
    if base_bg != ratatui::style::Color::Reset {
      base_bg
    } else {
      framework_tui::overlay_background()
    }
  }

  /// Parse a color name or `#rrggbb` hex string into a ratatui color.
  pub fn color(&self, name: &str) -> ratatui::style::Color {
    parse_color(name).unwrap_or(ratatui::style::Color::Reset)
  }
}

fn parse_color(name: &str) -> Option<ratatui::style::Color> {
  let color = match name.trim().to_ascii_lowercase().as_str() {
    "default" | "reset" => ratatui::style::Color::Reset,
    "black" => ratatui::style::Color::Black,
    "red" => ratatui::style::Color::Red,
    "green" => ratatui::style::Color::Green,
    "yellow" => ratatui::style::Color::Yellow,
    "blue" => ratatui::style::Color::Blue,
    "magenta" => ratatui::style::Color::Magenta,
    "cyan" => ratatui::style::Color::Cyan,
    "gray" | "grey" | "white" => ratatui::style::Color::Gray,
    "dark gray" | "dark grey" | "bright black" => ratatui::style::Color::DarkGray,
    "bright red" => ratatui::style::Color::LightRed,
    "bright green" => ratatui::style::Color::LightGreen,
    "bright yellow" => ratatui::style::Color::LightYellow,
    "bright blue" => ratatui::style::Color::LightBlue,
    "bright magenta" => ratatui::style::Color::LightMagenta,
    "bright cyan" => ratatui::style::Color::LightCyan,
    "bright white" => ratatui::style::Color::White,
    _ => {
      if let Some(hex) = name.trim().strip_prefix('#')
        && hex.len() == 6
        && let Ok(value) = u32::from_str_radix(hex, 16)
      {
        return Some(ratatui::style::Color::Rgb(
          ((value >> 16) & 0xff) as u8,
          ((value >> 8) & 0xff) as u8,
          (value & 0xff) as u8,
        ));
      }
      return None;
    }
  };
  Some(color)
}

const THEME_HEADER: &str = "\
# music-tui theme — every color the interface uses, grouped per view.
# Values are color names (\"cyan\", \"bright black\", \"default\") or
# \"#rrggbb\" hex strings. Edit freely; defaults are restored for any
# key you remove.
";

const SECTION_COMMENTS: &[(&str, &str)] = &[
  (
    "base",
    "# Shared colors: default text, pane borders, dimmed text,\n# accents, and the secondary accent (artist/genre fields, notices).\n",
  ),
  (
    "tab_bar",
    "# Tab bar: the active tab title and the inactive ones.\n",
  ),
  (
    "queue",
    "# Queue pane: the playing/paused row markers and the filter\n# keyword highlight color.\n",
  ),
  (
    "library",
    "# Library pane: playing/paused markers, filter keyword highlight,\n# the selected-row bar, and the per-field text colors\n# (title/album/filename use field_primary, artist/genre/lyrics use\n# field_secondary).\n",
  ),
  (
    "footer",
    "# Footer status line: the play-state icon, the song title while\n# stopped, and transient messages.\n",
  ),
  (
    "progress",
    "# Bottom progress band: the played portion and the remainder.\n",
  ),
  (
    "lyrics",
    "# Lyrics pane: the active line / sung characters and the manual\n# navigation cursor marker.\n",
  ),
  ("metadata", "# Metadata pane: the field label column.\n"),
  (
    "visualizer",
    "# Visualizer bands by frequency range: low / mid / high.\n",
  ),
  (
    "which_key",
    "# Which-key hint bar (pending key sequences). `separator` is the\n# text between key and description; `columns` wraps hints when the\n# bar gets crowded.\n",
  ),
];

/// Serialize the theme into the commented `theme.toml` representation.
pub(crate) fn format_theme_toml(theme: &ThemeConfig) -> String {
  let Ok(body) = toml::to_string_pretty(theme) else {
    return THEME_HEADER.to_string();
  };
  let mut out = String::from(THEME_HEADER);
  for line in body.lines() {
    if let Some(section) = line
      .strip_prefix('[')
      .and_then(|rest| rest.strip_suffix(']'))
      && let Some((_, comment)) = SECTION_COMMENTS.iter().find(|(name, _)| *name == section)
    {
      out.push('\n');
      out.push_str(comment);
    }
    out.push_str(line);
    out.push('\n');
  }
  out
}
