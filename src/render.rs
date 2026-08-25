//! Cover art rendering: protocol modes via img-tui native images, symbol
//! modes via chafa. Multi-entry store keyed by (path, size).

use std::{
  collections::{HashMap, HashSet, VecDeque},
  path::Path,
};

use ansi_to_tui::IntoText;
use img_tui::{NativeImageConfig, ProtocolPlacement, RenderMode, native_image};
use ratatui::text::Text;
use sha2::{Digest, Sha256};
use tokio::{process::Command, sync::mpsc};
use tracing::{debug, warn};

use crate::{
  config::RenderConfig,
  event::{AsyncEvent, RenderOutcome, RenderedImage},
};

pub struct CoverRenderStore {
  config: RenderConfig,
  native_config: NativeImageConfig,
  modes: Vec<RenderMode>,
  entries: HashMap<String, RenderedImage>,
  order: VecDeque<String>,
  in_flight: HashSet<String>,
}

const MAX_ENTRIES: usize = 8;

impl CoverRenderStore {
  pub fn new(config: RenderConfig, native_config: NativeImageConfig, modes: Vec<RenderMode>) -> Self {
    Self {
      config,
      native_config,
      modes,
      entries: HashMap::new(),
      order: VecDeque::new(),
      in_flight: HashSet::new(),
    }
  }

  /// Request a render for `path` unless it is already shown or in flight.
  /// Returns true when the request was queued.
  pub fn cell_pixels(&self) -> (u16, u16) {
    self.native_config.cell_pixels.unwrap_or((8, 16))
  }

  pub fn request(&mut self, path: &Path, width: u16, height: u16, tx: &mpsc::UnboundedSender<AsyncEvent>) -> bool {
    let cache_key = render_cache_key(path, width, height, &self.native_config);
    if self.entries.contains_key(&cache_key) || self.in_flight.contains(&cache_key) {
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

  /// Cover already rendered for this path and size, if any.
  /// Whether the primary render mode is a terminal image protocol
  /// (kitty/sixel/iterm) — those need pixel-preservation anti-flicker.
  pub(crate) fn draws_with_protocol(&self) -> bool {
    self.modes.first().is_some_and(|mode| mode.is_protocol())
  }

  pub fn get(&self, path: &Path, width: u16, height: u16) -> Option<&RenderedImage> {
    let cache_key = render_cache_key(path, width, height, &self.native_config);
    self.entries.get(&cache_key)
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
        false
      }
    }
  }
}

fn image_mode(image: &RenderedImage) -> &'static str {
  match image {
    RenderedImage::Symbols { mode, .. } => mode.label(),
    RenderedImage::Protocol { mode, .. } => mode.label(),
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
  let image_id = kitty_image_id(path, width, height, mode);
  let placement_id = kitty_placement_id(path, image_id);
  if mode.is_protocol() {
    let prepared = native_image::prepare(path, width, height, native_config.cell_pixels)
      .await
      .map_err(|error| error.to_string())?;
    if mode == RenderMode::Kitty && native_config.kitty_unicode_placeholders {
      // yazi-style U=1: upload once (a=t), then a *virtual* placement
      // (a=p,U=1,c=,r=) that fits the image to the pane rect. Display happens
      // via U+10EEEE placeholder text cells managed by img-tui, so modal
      // dialogs occlude the image per-cell and no re-transmit is needed.
      let image_id = image_id.unwrap_or(1);
      let upload = native_image::render_prepared_kitty_upload(&prepared, native_config, image_id)
        .await
        .map_err(|error| error.to_string())?;
      let virtual_placement = String::from_utf8(native_image::render_kitty_virtual_placement(
        native_config,
        image_id,
        width,
        height,
      ))
      .map_err(|error| error.to_string())?;
      let fingerprint = render_fingerprint(&upload.data);
      let mut data = String::from_utf8(upload.data).map_err(|error| error.to_string())?;
      data.push_str(&virtual_placement);
      return Ok(RenderedImage::Protocol {
        mode,
        data,
        refresh: Some(virtual_placement),
        placement: Some(ProtocolPlacement::KittyUnicode { image_id }),
        fingerprint,
        erase: native_image::erase_sequence(
          mode,
          native_config.passthrough.as_deref(),
          Some(image_id),
        ),
      });
    }
    if mode == RenderMode::Kitty
      && let Some(placement_id) = placement_id
    {
      let viewport = native_image::NativeImageViewport {
        full_width_cells: width,
        full_height_cells: height,
        visible_width_cells: width,
        visible_height_cells: height,
        left_cells: 0,
        top_cells: 0,
      };
      let image_id = image_id.unwrap_or(1);
      let upload = native_image::render_prepared_kitty_upload(&prepared, native_config, image_id)
        .await
        .map_err(|error| error.to_string())?;
      let refresh = native_image::render_kitty_viewport_from_upload(
        &upload,
        viewport,
        native_config,
        placement_id,
      )
      .map_err(|error| error.to_string())?;
      let fingerprint = render_fingerprint(&upload.data);
      let data = String::from_utf8(upload.data).map_err(|error| error.to_string())?;
      return Ok(RenderedImage::Protocol {
        mode,
        data,
        refresh: Some(
          String::from_utf8(refresh).map_err(|error| error.to_string())?,
        ),
        placement: Some(ProtocolPlacement::KittyPlacement {
          image_id,
          placement_id,
        }),
        fingerprint,
        erase: native_image::erase_kitty_placement_sequence(
          native_config.passthrough.as_deref(),
          image_id,
          placement_id,
        ),
      });
    }
    let data = native_image::render_prepared(&prepared, mode, native_config, image_id)
      .await
      .map_err(|error| error.to_string())?;
    let fingerprint = render_fingerprint(&data);
    let data = String::from_utf8(data).map_err(|error| error.to_string())?;
    let placement = None;
    let erase = native_image::erase_sequence(mode, native_config.passthrough.as_deref(), image_id);
    Ok(RenderedImage::Protocol {
      mode,
      data,
      refresh: None,
      placement,
      fingerprint,
      erase,
    })
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
  command.args(args).arg("--size").arg(format!("{width}x{height}"));
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

fn render_cache_key(path: &Path, width: u16, height: u16, native_config: &NativeImageConfig) -> String {
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

fn render_fingerprint(bytes: &[u8]) -> u64 {
  let mut hasher = Sha256::new();
  hasher.update(bytes);
  let digest = hasher.finalize();
  u64::from_le_bytes(digest[..8].try_into().unwrap_or_default())
}
