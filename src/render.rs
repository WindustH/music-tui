//! Cover art rendering: protocol modes via img-tui native images, symbol
//! modes via chafa. Multi-entry store keyed by (path, size).

use std::{
  collections::{HashMap, HashSet},
  path::Path,
};

use img_tui::{
  NativeImageConfig, ProtocolImage, ProtocolImageSpec, RenderMode, capability, native_image,
};
use ratatui::{
  text::Text,
  widgets::{Paragraph, Wrap},
};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
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

/// Environment variable that forces the render modes, e.g. `sixel,symbols`.
const RENDER_MODES_ENV: &str = "MUSIC_TUI_RENDER_MODES";

/// Probe the terminal and pick the render modes: `MUSIC_TUI_RENDER_MODES`
/// wins, then auto-detection when enabled, else character art only.
pub fn setup(config: &RenderConfig) -> RenderSetup {
  let terminal = capability::detect();
  info!(capability = ?terminal, "detected terminal capability");
  let modes = capability::render_modes_override_from_env(RENDER_MODES_ENV)
    .or_else(|| {
      config.auto_detect.then(|| {
        let zellij_sixel = config.zellij_sixel.as_str();
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
  cache: std::sync::Arc<img_tui::cache::paths::CachePaths>,
  config: RenderConfig,
  native_config: NativeImageConfig,
  modes: Vec<RenderMode>,
  entries: img_tui::cache::MemoryCache<String, RenderedImage>,
  current: Option<std::sync::Arc<RenderedImage>>,
  in_flight: HashSet<String>,
  /// Renders that failed in every mode, with the error. Remembered so a
  /// broken cover (or a missing `chafa`) is not re-rendered on every
  /// redraw.
  failed: HashMap<String, String>,
}

/// Failure memo bound; cleared wholesale when exceeded.
const MAX_FAILURES: usize = 64;

impl CoverRenderStore {
  pub fn new(
    cache: std::sync::Arc<img_tui::cache::paths::CachePaths>,
    config: RenderConfig,
    native_config: NativeImageConfig,
    modes: Vec<RenderMode>,
  ) -> Self {
    let memory_budget = config.raw_memory_cache_max_bytes;
    Self {
      cache,
      config,
      native_config,
      modes,
      entries: img_tui::cache::MemoryCache::new(memory_budget),
      current: None,
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
    let cache = self.cache.clone();
    let config = self.config.clone();
    let native_config = self.native_config.clone();
    let modes = self.modes.clone();
    let path = path.to_path_buf();
    let tx = tx.clone();
    tokio::spawn(async move {
      let result = render_cover(
        &cache,
        &path,
        width,
        height,
        &config,
        &native_config,
        &modes,
      )
      .await;
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
  pub fn get(&mut self, path: &Path, width: u16, height: u16) -> Option<&RenderedImage> {
    let cache_key = render_cache_key(path, width, height, &self.native_config);
    self.current = self.entries.get(&cache_key);
    self.current.as_deref()
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
        let bytes = match &image {
          RenderedImage::Protocol(image) => image.payload_len() as u64,
          // Symbol output is bounded by the cell dimensions and span overhead.
          RenderedImage::Symbols { bytes, .. } => *bytes,
        };
        self.entries.insert(outcome.cache_key.clone(), image, bytes);
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
  cache: &img_tui::cache::paths::CachePaths,
  path: &Path,
  width: u16,
  height: u16,
  config: &RenderConfig,
  native_config: &NativeImageConfig,
  modes: &[RenderMode],
) -> Result<RenderedImage, String> {
  let mut errors = Vec::new();
  for mode in modes {
    match render_once(cache, path, width, height, config, native_config, *mode).await {
      Ok(image) => return Ok(image),
      Err(error) => errors.push(format!("{}: {error}", mode.label())),
    }
  }
  Err(errors.join("; "))
}

async fn render_once(
  cache: &img_tui::cache::paths::CachePaths,
  path: &Path,
  width: u16,
  height: u16,
  config: &RenderConfig,
  native_config: &NativeImageConfig,
  mode: RenderMode,
) -> Result<RenderedImage, String> {
  use img_tui::cache::payload::{CacheHeader, RenderedBytes, decode_cache_file, encode_cache_file};
  let image_id = kitty_image_id(path, width, height, mode);
  let spec = ProtocolImageSpec {
    image_id,
    placement_id: kitty_placement_id(path, image_id),
    ..ProtocolImageSpec::new(mode, width, height)
  };
  let header = CacheHeader {
    width,
    height,
    cell_pixels: native_config.cell_pixels,
    mode,
    image_id,
    placement_id: spec.placement_id,
  };
  let key = format!(
    "{}:{mode:?}:{config:?}:{native_config:?}",
    render_cache_key(path, width, height, native_config)
  );
  let db = cache.database.clone();
  let lookup_key = key.clone();
  let cached = tokio::task::spawn_blocking(move || db.get("render", &lookup_key))
    .await
    .ok()
    .and_then(Result::ok)
    .flatten();
  let cached = if let Some(bytes) = cached {
    decode_cache_file(&bytes, &header)
      .await
      .ok()
      .map(|v| v.payload)
  } else {
    None
  };
  let bytes = if let Some(bytes) = cached {
    bytes
  } else {
    let bytes = if mode.is_protocol() {
      let prepared = native_image::prepare(path, width, height, native_config.cell_pixels)
        .await
        .map_err(|e| e.to_string())?;
      let encoded = native_image::encode_protocol(&prepared, &spec, native_config)
        .await
        .map_err(|e| e.to_string())?;
      RenderedBytes {
        data: encoded.data,
        refresh: encoded.refresh,
      }
    } else {
      RenderedBytes {
        data: run_chafa(path, width, height, config, mode).await?,
        refresh: None,
      }
    };
    if let Ok(encoded) = encode_cache_file(&bytes, &header, 1, 0).await {
      let db = cache.database.clone();
      let budget = config.disk_cache_max_bytes;
      let _ =
        tokio::task::spawn_blocking(move || db.put_bounded("render", &key, &encoded, budget)).await;
    }
    bytes
  };
  if mode.is_protocol() {
    ProtocolImage::from_encoded(
      img_tui::EncodedProtocolImage {
        data: bytes.data,
        refresh: bytes.refresh,
      },
      &spec,
      native_config,
    )
    .map(RenderedImage::Protocol)
    .map_err(|e| e.to_string())
  } else {
    let text: Text<'static> = img_tui::chafa::decode(&bytes.data)?;
    let size = text
      .lines
      .iter()
      .map(|line| {
        32 + line
          .spans
          .iter()
          .map(|span| 32 + span.content.len() as u64)
          .sum::<u64>()
      })
      .sum::<u64>();
    Ok(RenderedImage::Symbols {
      mode,
      bytes: size,
      paragraph: Box::new(Paragraph::new(text).wrap(Wrap { trim: false })),
    })
  }
}

async fn run_chafa(
  image_path: &Path,
  width: u16,
  height: u16,
  config: &RenderConfig,
  mode: RenderMode,
) -> Result<Vec<u8>, String> {
  img_tui::chafa::run(
    image_path,
    width,
    height,
    &img_tui::chafa::ChafaConfig {
      chafa_bin: &config.chafa_bin,
      chafa_args: &config.chafa_args,
      chafa_threads: config.chafa_threads,
    },
    mode,
  )
  .await
}

fn render_cache_key(
  path: &Path,
  width: u16,
  height: u16,
  native_config: &NativeImageConfig,
) -> String {
  let mut hasher = Sha256::new();
  hasher.update(b"music-tui-cover-render-v1");
  hasher.update(
    img_tui::cache::paths::identity(path)
      .as_os_str()
      .as_encoded_bytes(),
  );
  if let Ok(metadata) = std::fs::metadata(path) {
    hasher.update(metadata.len().to_le_bytes());
    if let Ok(time) = metadata.modified().and_then(|t| {
      t.duration_since(std::time::UNIX_EPOCH)
        .map_err(std::io::Error::other)
    }) {
      hasher.update(time.as_nanos().to_le_bytes());
    }
  }
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
  hasher.update(
    img_tui::cache::paths::identity(path)
      .as_os_str()
      .as_encoded_bytes(),
  );
  if let Ok(metadata) = std::fs::metadata(path) {
    hasher.update(metadata.len().to_le_bytes());
    if let Ok(time) = metadata.modified().and_then(|t| {
      t.duration_since(std::time::UNIX_EPOCH)
        .map_err(std::io::Error::other)
    }) {
      hasher.update(time.as_nanos().to_le_bytes());
    }
  }
  hasher.update(width.to_le_bytes());
  hasher.update(height.to_le_bytes());
  let digest = hasher.finalize();
  Some(native_image::kitty_image_id(&digest))
}

fn kitty_placement_id(path: &Path, image_id: Option<u32>) -> Option<u32> {
  let mut hasher = Sha256::new();
  hasher.update(b"music-tui-kitty-placement-v1");
  hasher.update(
    img_tui::cache::paths::identity(path)
      .as_os_str()
      .as_encoded_bytes(),
  );
  if let Ok(metadata) = std::fs::metadata(path) {
    hasher.update(metadata.len().to_le_bytes());
    if let Ok(time) = metadata.modified().and_then(|t| {
      t.duration_since(std::time::UNIX_EPOCH)
        .map_err(std::io::Error::other)
    }) {
      hasher.update(time.as_nanos().to_le_bytes());
    }
  }
  hasher.update(image_id.unwrap_or_default().to_le_bytes());
  let digest = hasher.finalize();
  let placement_id = u32::from_le_bytes(digest[..4].try_into().unwrap_or_default()) & 0x7fff_ffff;
  Some(placement_id.max(1))
}
