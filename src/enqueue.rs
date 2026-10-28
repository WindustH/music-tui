//! MPD URI resolution and idle playback helpers shared by CLI and live commands.
use crate::{
  config::MpdConfig,
  music_files::{ensure_link, file_uri, is_socket_host, links_dir, path_to_uri},
};
use anyhow::{Result, bail};
use mpd_client::{
  Client,
  commands::{Play, Status, Update},
  responses::PlayState,
};
use std::path::{Path, PathBuf};
/// Resolve one file to a playable MPD uri: in-library paths keep their
/// relative uri; outside paths become `file://` on socket connections or
/// a bridged symlink (plus a db update) on TCP connections.
pub(crate) async fn resolve_open_uri(
  client: &Client,
  path: &Path,
  mpd_config: &MpdConfig,
  music_dir: Option<&Path>,
) -> Result<String> {
  if let Some(uri) = direct_open_uri(path, mpd_config, music_dir) {
    return Ok(uri);
  }
  let owned = path.to_path_buf();
  resolve_outside_uris(client, &[owned], mpd_config, music_dir)
    .await
    .map(|mut uris| uris.remove(0))
}

/// Resolve a path without touching MPD. Relative library URIs are preferred
/// when a root is known; otherwise Unix socket connections can use `file://`.
pub(crate) fn direct_open_uri(
  path: &Path,
  mpd_config: &MpdConfig,
  music_dir: Option<&Path>,
) -> Option<String> {
  if let Some(music_dir) = music_dir
    && let Ok(uri) = path_to_uri(music_dir, path)
  {
    return Some(uri);
  }
  is_socket_host(&mpd_config.host).then(|| file_uri(path))
}

/// Resolve a batch of files (mixed in/outside paths allowed), preserving
/// the caller's order.
pub(crate) async fn resolve_open_uris(
  client: &Client,
  files: &[PathBuf],
  mpd_config: &MpdConfig,
  music_dir: Option<&Path>,
) -> Result<Vec<String>> {
  let inside: Vec<(usize, String)> = files
    .iter()
    .enumerate()
    .filter_map(|(index, file)| {
      direct_open_uri(file, mpd_config, music_dir).map(|uri| (index, uri))
    })
    .collect();
  let outside: Vec<(usize, PathBuf)> = files
    .iter()
    .enumerate()
    .filter(|(_, file)| direct_open_uri(file, mpd_config, music_dir).is_none())
    .map(|(index, file)| (index, file.clone()))
    .collect();
  if outside.is_empty() {
    return Ok(inside.into_iter().map(|(_, uri)| uri).collect());
  }
  let outside_paths: Vec<PathBuf> = outside.iter().map(|(_, path)| path.clone()).collect();
  let resolved = resolve_outside_uris(client, &outside_paths, mpd_config, music_dir).await?;
  let mut mixed: Vec<(usize, String)> = inside;
  mixed.extend(outside.iter().map(|(index, _)| *index).zip(resolved));
  mixed.sort_by_key(|(index, _)| *index);
  Ok(mixed.into_iter().map(|(_, uri)| uri).collect())
}

/// Files outside the library: `file://` when connected via socket, else a
/// symlink bridge under `[mpd].link_dir` (default `<music_dir>/.music-tui-links`)
/// plus a scoped database update.
async fn resolve_outside_uris(
  client: &Client,
  outside: &[PathBuf],
  mpd_config: &MpdConfig,
  music_dir: Option<&Path>,
) -> Result<Vec<String>> {
  if is_socket_host(&mpd_config.host) {
    return Ok(outside.iter().map(|path| file_uri(path)).collect());
  }
  let Some(music_dir) = music_dir else {
    bail!(
      "cannot open local files over TCP without a music directory; configure mpd.music_dir or connect through a Unix socket"
    );
  };
  let dir = links_dir(music_dir, &mpd_config.link_dir);
  let mut links = Vec::with_capacity(outside.len());
  for path in outside {
    links.push(ensure_link(&dir, path).map_err(anyhow::Error::msg)?);
  }
  let dir_uri = path_to_uri(music_dir, &dir)?;
  update_and_wait(client, &dir_uri).await?;
  links
    .iter()
    .map(|link| path_to_uri(music_dir, link))
    .collect::<std::result::Result<Vec<_>, _>>()
}

/// Send a scoped `update` and wait until MPD finishes (max ~15s).
async fn update_and_wait(client: &Client, uri: &str) -> Result<()> {
  client.command(Update::new().uri(uri)).await?;
  let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
  while tokio::time::Instant::now() < deadline {
    let status = client.command(Status).await?;
    if status.update_job.is_none() {
      return Ok(());
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
  }
  bail!("timed out waiting for the database update")
}

pub(crate) async fn maybe_start_if_idle(client: &Client) -> Result<()> {
  let status = client.command(Status).await?;
  if status.state == PlayState::Stopped {
    client.command(Play::current()).await?;
  }
  Ok(())
}
