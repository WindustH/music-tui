use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub fn init(cache_dir: &Path) -> Result<PathBuf> {
  let path = cache_dir.join("music-tui.log");
  let file = open_log_file(&path).with_context(|| format!("failed to open {}", path.display()))?;
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

/// Append-mode log handle that refuses symlinks: a hostile or accidental
/// link under the cache dir must not redirect the log stream.
#[cfg(unix)]
fn open_log_file(path: &Path) -> std::io::Result<std::fs::File> {
  use std::os::unix::fs::OpenOptionsExt;
  std::fs::OpenOptions::new()
    .create(true)
    .append(true)
    .custom_flags(libc::O_NOFOLLOW)
    .open(path)
}

#[cfg(windows)]
fn open_log_file(path: &Path) -> std::io::Result<std::fs::File> {
  if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
    return Err(std::io::Error::other(format!(
      "refusing to open {}: the log path is a symlink",
      path.display()
    )));
  }
  std::fs::OpenOptions::new()
    .create(true)
    .append(true)
    .open(path)
}

#[cfg(all(test, unix))]
mod tests {
  use super::*;

  #[test]
  fn refuses_symlinked_log_path() {
    let dir = std::env::temp_dir().join(format!("music-tui-log-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("target.log");
    std::fs::write(&target, b"").unwrap();
    let link = dir.join("music-tui.log");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    assert!(open_log_file(&link).is_err());
    let _ = std::fs::remove_dir_all(&dir);
  }
}
