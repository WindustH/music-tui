use std::path::PathBuf;

use crossterm::event::Event;
use img_tui::{ProtocolPlacement, RenderMode};
use mpd_client::responses::{SongInQueue, Status};
use ratatui::text::Text;

#[derive(Debug)]
pub enum AsyncEvent {
  Input { event: Event, generation: u64 },
  Mpd(MpdEvent),
  Tick,
  Lyrics(LyricsOutcome),
  Metadata(MetadataOutcome),
  MetadataWrite(MetadataWriteOutcome),
  Cover(CoverOutcome),
  Render(RenderOutcome),
  #[cfg(unix)]
  Spectrum(Vec<u8>),
  /// Precomputed visualizer pane lines from the band-render worker.
  VisualizerFrame(Vec<ratatui::text::Line<'static>>),
  /// Library database events from the scanner thread.
  Library(LibraryEvent),
}

#[derive(Debug)]
pub enum LibraryEvent {
  /// Periodic progress while scanning (scanned files, changed files).
  Scanning { scanned: usize, changed: usize },
  /// Scan finished; carries the full track list.
  Loaded(Vec<crate::library_db::LibraryTrack>),
  Error(String),
}

#[derive(Debug)]
pub enum MpdEvent {
  Connected(String),
  Snapshot { status: Status, queue: Vec<SongInQueue> },
  ConnectionLost(String),
  Notice(String),
}

#[derive(Debug)]
pub struct LyricsOutcome {
  pub song_url: String,
  pub result: Result<crate::lyrics::Lyrics, String>,
}

#[derive(Debug)]
pub struct MetadataOutcome {
  pub song_url: String,
  pub result: Result<Vec<crate::metadata::MetadataEntry>, String>,
}

#[derive(Debug)]
pub struct MetadataWriteOutcome {
  pub song_url: String,
  pub changed_tags: usize,
  pub result: Result<(), String>,
}

#[derive(Debug)]
pub struct CoverOutcome {
  pub song_url: String,
  pub result: Result<PathBuf, String>,
  /// Intrinsic pixel dimensions of the cover, read next to `find_cover`.
  pub dims: Option<(u32, u32)>,
}

#[derive(Debug)]
pub struct RenderOutcome {
  pub cache_key: String,
  pub result: Result<RenderedImage, String>,
}

#[derive(Debug, Clone)]
pub enum RenderedImage {
  Symbols { mode: RenderMode, text: Text<'static> },
  Protocol {
    mode: RenderMode,
    data: String,
    refresh: Option<String>,
    placement: Option<ProtocolPlacement>,
    fingerprint: u64,
    erase: Option<String>,
  },
}
