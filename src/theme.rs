use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
  pub foreground: String,
  pub background: String,
  pub border: String,
  pub muted: String,
  pub accent: String,
  pub accent_alt: String,
  pub playing: String,
  pub paused: String,
  pub stopped: String,
  pub progress: String,
  pub progress_background: String,
  pub lyrics_active: String,
  pub visualizer_low: String,
  pub visualizer_mid: String,
  pub visualizer_high: String,
  pub which_key_background: String,
  pub which_key_foreground: String,
  pub which_key_key: String,
  pub which_key_description: String,
  pub which_key_separator: String,
  pub which_key_separator_color: String,
  pub which_key_columns: u16,
}

impl Default for ThemeConfig {
  fn default() -> Self {
    Self {
      foreground: "default".to_string(),
      background: "default".to_string(),
      border: "bright black".to_string(),
      muted: "bright black".to_string(),
      accent: "cyan".to_string(),
      accent_alt: "magenta".to_string(),
      playing: "green".to_string(),
      paused: "yellow".to_string(),
      stopped: "bright black".to_string(),
      progress: "cyan".to_string(),
      progress_background: "bright black".to_string(),
      lyrics_active: "cyan".to_string(),
      visualizer_low: "green".to_string(),
      visualizer_mid: "yellow".to_string(),
      visualizer_high: "red".to_string(),
      which_key_background: "black".to_string(),
      which_key_foreground: "white".to_string(),
      which_key_key: "light_cyan".to_string(),
      which_key_description: "light_magenta".to_string(),
      which_key_separator: " -> ".to_string(),
      which_key_separator_color: "dark_gray".to_string(),
      which_key_columns: 3,
    }
  }
}

impl ThemeConfig {
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
