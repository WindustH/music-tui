//! Cover art rendering: protocol modes via img-tui native images, symbol
//! modes via chafa. Multi-entry store keyed by (path, size).

use std::{
  collections::{HashMap, HashSet, VecDeque},
  path::Path,
};

use ansi_to_tui::IntoText;
use img_tui::{
  NativeImageConfig, ProtocolImage, ProtocolImageSpec, RenderMode, capability, native_image,
};
use ratatui::text::Text;
use sha2::{Digest, Sha256};
use tokio::{process::Command, sync::mpsc};
use tracing::{debug, info, warn};

use crate::{
  config::RenderConfig,
  event::{AsyncEvent, RenderOutcome, RenderedImage},
};

/// Terminal graphics setup resolved at startup.
pub struct RenderSetup {
  pub native_config: NativeImageConfig,
  /// Render modes to try, best first.
  pub modes: Vec<RenderMode>,
  /// Escape sequence that clears stale Kitty images (startup and exit).
  pub protocol_reset: Option<String>,
}

/// Probe the terminal and pick the render modes: `GALLERY_TUI_RENDER_MODES`
/// (img-tui's override variable) wins, then auto-detection when enabled,
/// else character art only.
pub fn setup(config: &RenderConfig) -> RenderSetup {
  let terminal = capability::detect();
  info!(capability = ?terminal, "detected terminal capability");
  let modes = capability::render_modes_override_from_env()
    .or_else(|| {
      config.auto_detect.then(|| {
        let zellij_sixel = if config.zellij_sixel { "on" } else { "" };
        terminal.preferred_render_modes(zellij_sixel)
      })
    })
    .unwrap_or_else(|| vec![RenderMode::Symbols, RenderMode::Ascii]);
  info!(
    modes = ?modes.iter().map(|mode| mode.label()).collect::<Vec<_>>(),
    "effective render modes"
  );
  // `render.passthrough` overrides the detected multiplexer wrapping for
  // graphics escapes (`tmux`, `screen`, or `none`).
  let passthrough = match config.passthrough.as_deref().map(str::trim) {
    None | Some("") => terminal.passthrough().map(str::to_string),
    Some("none") => None,
    Some(value) => Some(value.to_string()),
  };
  let native_config = NativeImageConfig {
    cell_pixels: terminal.cell_pixels,
    passthrough,
    kitty_unicode_placeholders: terminal.kitty_unicode_placeholders(),
  };
  let protocol_reset = modes
    .contains(&RenderMode::Kitty)
    .then(|| {
      native_image::erase_sequence(
        RenderMode::Kitty,
        native_config.passthrough.as_deref(),
        None,
      )
    })
    .flatten();
  RenderSetup {
    native_config,
    modes,
    protocol_reset,
  }
}

pub struct CoverRenderStore {
  config: RenderConfig,
  native_config: NativeImageConfig,
  modes: Vec<RenderMode>,
  entries: HashMap<String, RenderedImage>,
  order: VecDeque<String>,
  in_flight: HashSet<String>,
  /// Renders that failed in every mode, with the error. Remembered so a
  /// broken cover (or a missing `chafa`) is not re-rendered on every
  /// redraw.
  failed: HashMap<String, String>,
}

const MAX_ENTRIES: usize = 8;
/// Failure memo bound; cleared wholesale when exceeded.
const MAX_FAILURES: usize = 64;

impl CoverRenderStore {
  pub fn new(
    config: RenderConfig,
    native_config: NativeImageConfig,
    modes: Vec<RenderMode>,
  ) -> Self {
    Self {
      config,
      native_config,
      modes,
      entries: HashMap::new(),
      order: VecDeque::new(),
      in_flight: HashSet::new(),
      failed: HashMap::new(),
    }
  }

  /// Terminal cell size in pixels (a common default when unknown).
  pub fn cell_pixels(&self) -> (u16, u16) {
    self.native_config.cell_pixels.unwrap_or((8, 16))
  }

  /// Request a render for `path` unless it is already shown or in flight.
  /// Returns true when the request was queued.
  pub fn request(
    &mut self,
    path: &Path,
    width: u16,
    height: u16,
    tx: &mpsc::UnboundedSender<AsyncEvent>,
  ) -> bool {
    let cache_key = render_cache_key(path, width, height, &self.native_config);
    if self.entries.contains_key(&cache_key)
      || self.in_flight.contains(&cache_key)
      || self.failed.contains_key(&cache_key)
    {
      return false;
    }
    self.in_flight.insert(cache_key.clone());
    let config = self.config.clone();
    let native_config = self.native_config.clone();
    let modes = self.modes.clone();
    let path = path.to_path_buf();
    let tx = tx.clone();
    tokio::spawn(async move {
      let result = render_cover(&path, width, height, &config, &native_config, &modes).await;
      let _ = tx.send(AsyncEvent::Render(RenderOutcome { cache_key, result }));
    });
    true
  }

  /// Whether the primary render mode is a terminal image protocol
  /// (kitty/sixel/iterm) — those need pixel-preservation anti-flicker.
  pub(crate) fn draws_with_protocol(&self) -> bool {
    self.modes.first().is_some_and(|mode| mode.is_protocol())
  }

  /// Cover already rendered for this path and size, if any.
  pub fn get(&self, path: &Path, width: u16, height: u16) -> Option<&RenderedImage> {
    let cache_key = render_cache_key(path, width, height, &self.native_config);
    self.entries.get(&cache_key)
  }

  /// Why rendering this cover at this size failed, if it did.
  pub fn error(&self, path: &Path, width: u16, height: u16) -> Option<&str> {
    let cache_key = render_cache_key(path, width, height, &self.native_config);
    self.failed.get(&cache_key).map(String::as_str)
  }

  pub fn finish(&mut self, outcome: RenderOutcome) -> bool {
    if !self.in_flight.remove(outcome.cache_key.as_str()) {
      return false;
    }
    match outcome.result {
      Ok(image) => {
        debug!(cache_key = %outcome.cache_key, mode = image_mode(&image), "cover rendered");
        self.order.push_back(outcome.cache_key.clone());
        self.entries.insert(outcome.cache_key.clone(), image);
        while self.order.len() > MAX_ENTRIES {
          if let Some(oldest) = self.order.pop_front() {
            self.entries.remove(&oldest);
          }
        }
        true
      }
      Err(error) => {
        warn!(%error, cache_key = %outcome.cache_key, "cover render failed");
        if self.failed.len() >= MAX_FAILURES {
          self.failed.clear();
        }
        self.failed.insert(outcome.cache_key, error);
        true
      }
    }
  }
}

fn image_mode(image: &RenderedImage) -> &'static str {
  match image {
    RenderedImage::Symbols { mode, .. } => mode.label(),
    RenderedImage::Protocol(image) => image.mode.label(),
  }
}

async fn render_cover(
  path: &Path,
  width: u16,
  height: u16,
  config: &RenderConfig,
  native_config: &NativeImageConfig,
  modes: &[RenderMode],
) -> Result<RenderedImage, String> {
  let mut errors = Vec::new();
  for mode in modes {
    match render_once(path, width, height, config, native_config, *mode).await {
      Ok(image) => return Ok(image),
      Err(error) => errors.push(format!("{}: {error}", mode.label())),
    }
  }
  Err(errors.join("; "))
}

async fn render_once(
  path: &Path,
  width: u16,
  height: u16,
  config: &RenderConfig,
  native_config: &NativeImageConfig,
  mode: RenderMode,
) -> Result<RenderedImage, String> {
  if mode.is_protocol() {
    let prepared = native_image::prepare(path, width, height, native_config.cell_pixels)
      .await
      .map_err(|error| error.to_string())?;
    // With kitty Unicode placeholders (yazi-style U=1) the image is uploaded
    // once and shown through placeholder text cells managed by img-tui, so
    // modal dialogs occlude it per cell and no re-transmit is needed.
    let image_id = kitty_image_id(path, width, height, mode);
    let spec = ProtocolImageSpec {
      image_id,
      placement_id: kitty_placement_id(path, image_id),
      ..ProtocolImageSpec::new(mode, width, height)
    };
    ProtocolImage::render(&prepared, &spec, native_config)
      .await
      .map(RenderedImage::Protocol)
      .map_err(|error| error.to_string())
  } else {
    let bytes = run_chafa(path, width, height, config, mode).await?;
    let text: Text<'static> = bytes.into_text().map_err(|error| error.to_string())?;
    Ok(RenderedImage::Symbols { mode, text })
  }
}

async fn run_chafa(
  image_path: &Path,
  width: u16,
  height: u16,
  config: &RenderConfig,
  mode: RenderMode,
) -> Result<Vec<u8>, String> {
  let mut command = Command::new(&config.chafa_bin);
  let mut args: Vec<String> = config
    .chafa_args
    .iter()
    .filter(|arg| {
      !arg.starts_with("--format=")
        && !arg.starts_with("--colors=")
        && !arg.starts_with("--symbols=")
        && !arg.starts_with("--passthrough=")
        && !arg.starts_with("--probe=")
        && !arg.starts_with("--relative=")
    })
    .cloned()
    .collect();
  args.push(format!("--format={}", mode.chafa_format()));
  args.push("--probe=off".to_string());
  args.push("--relative=off".to_string());
  args.push("--passthrough=none".to_string());
  if !args.iter().any(|arg| arg.starts_with("--scale=")) {
    args.push("--scale=max".to_string());
  }
  if config.chafa_threads > 0
    && !config
      .chafa_args
      .iter()
      .any(|arg| arg.starts_with("--threads="))
  {
    args.push(format!("--threads={}", config.chafa_threads));
  }
  match mode {
    RenderMode::Symbols => {
      for arg in &config.chafa_args {
        if arg.starts_with("--colors=") || arg.starts_with("--symbols=") {
          args.push(arg.clone());
        }
      }
    }
    RenderMode::Ascii => {
      args.push("--colors=none".to_string());
      args.push("--symbols=ascii".to_string());
    }
    _ => {}
  }
  command
    .args(args)
    .arg("--size")
    .arg(format!("{width}x{height}"));
  command.arg(image_path);

  let chafa_bin = config.chafa_bin.clone();
  let output = command
    .output()
    .await
    .map_err(|error| format!("failed to run {chafa_bin}: {error}"))?;
  if !output.status.success() {
    return Err(format!(
      "{chafa_bin} exited with {}: {}",
      output.status,
      String::from_utf8_lossy(&output.stderr).trim()
    ));
  }
  Ok(output.stdout)
}

fn render_cache_key(
  path: &Path,
  width: u16,
  height: u16,
  native_config: &NativeImageConfig,
) -> String {
  let mut hasher = Sha256::new();
  hasher.update(b"music-tui-cover-render-v1");
  hasher.update(path.to_string_lossy().as_bytes());
  hasher.update(width.to_le_bytes());
  hasher.update(height.to_le_bytes());
  let (cell_w, cell_h) = native_config.cell_pixels.unwrap_or((0, 0));
  hasher.update(cell_w.to_le_bytes());
  hasher.update(cell_h.to_le_bytes());
  hex::encode(hasher.finalize())
}

fn kitty_image_id(path: &Path, width: u16, height: u16, mode: RenderMode) -> Option<u32> {
  if mode != RenderMode::Kitty {
    return None;
  }
  let mut hasher = Sha256::new();
  hasher.update(b"music-tui-kitty-image-v1");
  hasher.update(path.to_string_lossy().as_bytes());
  hasher.update(width.to_le_bytes());
  hasher.update(height.to_le_bytes());
  let digest = hasher.finalize();
  Some(native_image::kitty_image_id(&digest))
}

fn kitty_placement_id(path: &Path, image_id: Option<u32>) -> Option<u32> {
  let mut hasher = Sha256::new();
  hasher.update(b"music-tui-kitty-placement-v1");
  hasher.update(path.to_string_lossy().as_bytes());
  hasher.update(image_id.unwrap_or_default().to_le_bytes());
  let digest = hasher.finalize();
  let placement_id = u32::from_le_bytes(digest[..4].try_into().unwrap_or_default()) & 0x7fff_ffff;
  Some(placement_id.max(1))
}
