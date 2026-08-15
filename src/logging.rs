use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub fn init(cache_dir: &Path) -> Result<PathBuf> {
  let path = cache_dir.join("music-tui.log");
  let file = std::fs::OpenOptions::new()
    .create(true)
    .append(true)
    .open(&path)
    .with_context(|| format!("failed to open {}", path.display()))?;
  tracing_subscriber::fmt()
    .with_ansi(false)
    .with_target(false)
    .with_writer(std::sync::Mutex::new(file))
    .with_env_filter(
      tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
    )
    .init();
  Ok(path)
}
