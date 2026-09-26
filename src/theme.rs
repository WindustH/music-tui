//! Theme configuration: `~/.config/music-tui/theme.toml`.
//!
//! Colors are grouped per interface section (like the keymap file), so
//! every view's colors are configurable independently. Values are color
//! names (`cyan`, `bright black`, `default`) or `#rrggbb` hex strings.

use serde::{Deserialize, Serialize};

macro_rules! color_section {
  ($name:ident { $($field:ident => $default:literal),* $(,)? }) => {
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(default, deny_unknown_fields)]
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
#[serde(default, deny_unknown_fields)]
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
#[serde(default, deny_unknown_fields)]
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

  /// Background of the which-key hint bar: `which_key.background` when set
  /// to a color, else the overlay background.
  pub fn which_key_background(&self) -> ratatui::style::Color {
    match self.color(&self.which_key.background) {
      ratatui::style::Color::Reset => self.overlay_background(),
      color => color,
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

/// music-tui's own names first (`white` is the normal-intensity white,
/// `bright white` the bold one), then ratatui's parser, which also accepts
/// spellings such as `light_cyan`, `dark-gray`, indexed colors (`208`)
/// and `#rrggbb`.
fn parse_color(name: &str) -> Option<ratatui::style::Color> {
  use ratatui::style::Color;
  let color = match name.trim().to_ascii_lowercase().as_str() {
    "default" | "reset" => Color::Reset,
    "black" => Color::Black,
    "red" => Color::Red,
    "green" => Color::Green,
    "yellow" => Color::Yellow,
    "blue" => Color::Blue,
    "magenta" => Color::Magenta,
    "cyan" => Color::Cyan,
    "gray" | "grey" | "white" => Color::Gray,
    "dark gray" | "dark grey" | "bright black" => Color::DarkGray,
    "bright red" => Color::LightRed,
    "bright green" => Color::LightGreen,
    "bright yellow" => Color::LightYellow,
    "bright blue" => Color::LightBlue,
    "bright magenta" => Color::LightMagenta,
    "bright cyan" => Color::LightCyan,
    "bright white" => Color::White,
    other => return other.parse().ok(),
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
    "# Shared colors: default text, background (painted only with\n# render_background = true), pane borders, dimmed text, and the accent\n# for focused titles and prompts (accent_alt is currently unused).\n",
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
    "# Footer status line: the play-state icon (stopped also colors the\n# offline notice) and transient messages.\n",
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
    "# Which-key hint bar (pending key sequences). `background` = \"reset\"\n# uses the overlay background; `separator` is the text between key and\n# description; `columns` wraps hints when the bar gets crowded.\n",
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

#[cfg(test)]
mod tests {
  use super::*;
  use ratatui::style::Color;

  #[test]
  fn default_theme_colors_all_parse() {
    let theme = ThemeConfig::default();
    let which_key = &theme.which_key;
    for name in [
      &which_key.foreground,
      &which_key.key,
      &which_key.description,
      &which_key.separator_color,
      &which_key.background,
    ] {
      assert!(parse_color(name).is_some(), "{name:?} must parse");
    }
    assert_eq!(theme.color(&which_key.key), Color::LightCyan);
    assert_eq!(theme.color(&which_key.separator_color), Color::DarkGray);
  }

  #[test]
  fn color_names_keep_their_meaning() {
    assert_eq!(parse_color("white"), Some(Color::Gray));
    assert_eq!(parse_color("bright white"), Some(Color::White));
    assert_eq!(parse_color("Bright Black"), Some(Color::DarkGray));
    assert_eq!(parse_color("#ff8000"), Some(Color::Rgb(255, 128, 0)));
    assert_eq!(parse_color("208"), Some(Color::Indexed(208)));
    assert_eq!(parse_color("light-magenta"), Some(Color::LightMagenta));
    assert_eq!(parse_color("no such color"), None);
  }
}
