//! "Interrupt" preview sessions: snapshot the current queue into a stored
//! playlist, play the preview song, then restore the previous queue and
//! playback state when it finishes.

use super::*;
/// Best-effort helper used by `open --mode interrupt` to snapshot state.
pub async fn capture_interrupt_session(client: &Client) -> anyhow::Result<InterruptSession> {
  let status = client.command(commands::Status).await?;
  let playlist = if status.playlist_length > 0 {
    let stamp = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs();
    let name = format!("music-tui-preview-{stamp}");
    client.command(SaveQueueAsPlaylist(name.as_str())).await?;
    Some(name)
  } else {
    None
  };
  Ok(InterruptSession {
    playlist,
    was_playing: status.state == PlayState::Playing,
    position: status.current_song.map(|(pos, _)| pos.0 as u32),
    elapsed_secs: status.elapsed.map(|elapsed| elapsed.as_secs_f64()),
    single: status.single,
  })
}
