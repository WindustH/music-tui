//! Application state and input handling.

pub(crate) use std::{
  collections::HashSet,
  path::PathBuf,
  time::{Duration, Instant},
};

pub(crate) use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
pub(crate) use framework_tui::{
  CommandState, KeyBindings, KeyContext, KeyDispatcher, MatchResult, Prompt, PromptInputResult,
  key_event_to_token,
};
pub(crate) use mpd_client::commands::SingleMode;
pub(crate) use mpd_client::responses::{Song, SongInQueue, Status};
pub(crate) use ratatui::{layout::Rect, widgets::ListState};
pub(crate) use tokio::sync::mpsc;
pub(crate) use tracing::debug;

pub(crate) use crate::{
  config::{Settings, expand_home},
  cover, metadata,
  event::{AsyncEvent, CoverOutcome, LyricsOutcome, MpdEvent, MetadataOutcome, MetadataWriteOutcome},
  layout::{PaneKind, PaneLayout, PaneSource, TabLayout, parse_detail, parse_tabs},
  library::{resolve_music_dir, uri_to_path},
  keymap::KeymapConfig,
  lyrics::{self, Lyrics},
  mpd::{InterruptSession, MpdCommand, MpdHandle},
};

pub enum EditorRequest {
  Metadata {
    song_url: String,
    path: PathBuf,
    original: Vec<metadata::MetadataEntry>,
    draft: String,
  },
}

/// Secondary detail view for a queue entry (the `i` key), in the spirit of
pub(crate) mod actions;
pub(crate) mod bindings;
pub(crate) mod editor;
pub(crate) mod labels;
pub(crate) mod loading;
pub(crate) mod outcomes;
pub(crate) mod snapshot;
pub(crate) mod commands;
mod detail;
pub(crate) mod input;
pub(crate) mod mouse;

pub use detail::DetailView;
pub use detail::HoverView;
pub(crate) use labels::{song_artist, song_title};

pub struct App {
  pub settings: Settings,
  pub mpd: MpdHandle,
  pub events: mpsc::UnboundedSender<AsyncEvent>,
  pub music_dir: Option<PathBuf>,

  /// Parsed tab layouts from the config.
  pub tabs: Vec<TabLayout>,
  /// Index of the active tab.
  pub tab: usize,

  pub quit: bool,
  pub message: Option<(String, Instant)>,

  pub connected: Option<String>,
  pub connection_error: Option<String>,
  pub status: Option<Status>,
  pub queue: Vec<SongInQueue>,
  pub queue_state: ListState,
  pub follow_current: bool,

  pub prompt: Option<Prompt>,
  pub command_state: CommandState,
  pub show_help: bool,

  pub lyrics: Option<crate::lyrics::Lyrics>,
  pub lyrics_url: String,
  pub lyrics_error: Option<String>,
  pub lyrics_scroll: usize,
  pub lyrics_follow: bool,
  /// Queued selection restore from the persisted state, applied once the
  /// first non-empty queue snapshot arrives.
  pub pending_restore_selection: Option<usize>,
  /// Active queue filter (case-insensitive substring over title / artist /
  /// album / url), entered via `/`.
  pub queue_filter: Option<String>,
  /// Queue positions matching the filter; selection indexes this list.
  pub queue_filter_matches: Vec<usize>,

  pub metadata_entries: Option<Vec<metadata::MetadataEntry>>,
  pub metadata_url: String,
  pub metadata_error: Option<String>,
  pub metadata_scroll: usize,
  pub editor_request: Option<EditorRequest>,

  pub cover_path: Option<(String, PathBuf)>,
  pub cover_dims: Option<(u32, u32)>,
  pub cover_error: Option<String>,
  /// Secondary detail view for the selected queue entry (`i`).
  pub detail: Option<DetailView>,
  /// Data view for the hovered queue row (`:hovered` pane source).
  pub hover: Option<HoverView>,
  /// Lofty re-reads queued for queue songs whose MPD tags are all
  /// `?`-corrupted (e.g. GBK RIFF INFO in WAV files).
  pub(crate) tag_fallbacks_pending: HashSet<String>,
  /// URLs already re-read once; never retried.
  pub(crate) tag_fallbacks_done: HashSet<String>,
  /// Whether any configured pane uses the hovered data source (gates the
  /// lazy loading in `sync_hover_view`).
  pub(crate) has_hover_panes: bool,
  /// Layout tree for the secondary detail view (cover + metadata panes).
  pub(crate) detail_layout: PaneLayout,
  /// Visualizer worker handle (reports pane width for band allocation).
  pub(crate) visualizer: Option<crate::visualizer::VisualizerHandle>,
  /// Scroll position of the f1 key-help dialog.
  pub help_scroll: usize,
  /// Maximum scroll of the key-help dialog, updated at draw time.
  pub max_help_scroll: usize,
  /// Inner screen areas of lyrics panes in the current tab, recorded at
  /// draw time so clicks can be mapped to lyric lines.
  pub lyrics_pane_areas: Vec<Rect>,
  /// Data source of each recorded lyrics pane (parallel to
  /// `lyrics_pane_areas`), so mouse handlers know whether a pane shows the
  /// hovered song (no seek) or the playing song.
  pub lyrics_pane_sources: Vec<PaneSource>,
  /// Screen areas of visible queue panes, recorded at draw time for mouse
  /// hit-testing (click to select, click again to play, wheel to move).
  pub queue_pane_areas: Vec<Rect>,
  /// Tab label rectangles in the tab bar, recorded at draw time so mouse
  /// clicks can switch tabs directly.
  pub tab_hit_areas: Vec<Rect>,
  /// Selected lyric line while in manual scroll mode.
  pub lyrics_cursor: Option<usize>,

  pub spectrum: Vec<u8>,

  /// Screen area of the bottom progress band, recorded at draw time for
  /// mouse hit-testing (click / drag to seek).
  pub progress_band_area: Option<Rect>,
  band_scrubbing: bool,

  pub(crate) dispatcher: KeyDispatcher,
  /// Bindings per pane kind, indexed by `PaneKind::index`.
  view_bindings: Vec<KeyBindings>,
  input_bindings: KeyBindings,
  /// Key-help dialog bindings (scroll keys are user-configurable).
  help_bindings: KeyBindings,
}

impl App {
  pub fn new(
    settings: Settings,
    mpd: MpdHandle,
    events: mpsc::UnboundedSender<AsyncEvent>,
    initial_notice: Option<String>,
    interrupt: Option<InterruptSession>,
  ) -> Self {
    let music_dir = resolve_music_dir(&settings.config.mpd).ok();
    let lyrics_follow = settings.config.lyrics.follow;
    if let Some(session) = interrupt {
      mpd.send(MpdCommand::ArmInterrupt(session));
    }
    let tabs = parse_tabs(&settings.config.layout).unwrap_or_else(|error| {
      eprintln!("invalid layout config ({error}); using default tabs");
      parse_tabs(&crate::config::LayoutConfig::default()).expect("default tabs")
    });
    let detail_layout = parse_detail(&settings.config.layout.detail).unwrap_or_else(|error| {
      eprintln!("invalid detail layout ({error}); using default");
      parse_detail(crate::layout::DEFAULT_DETAIL_LAYOUT).expect("default detail layout")
    });
    let view_bindings = bindings::build_bindings(&settings.keymap);
    let input_bindings = bindings::build_input_bindings(&settings.keymap);
    let help_bindings = settings.keymap.help_bindings();
    let mut app = Self {
      mpd,
      events: events.clone(),
      music_dir,
      tabs,
      detail_layout,
      tab: 0,
      settings,
      quit: false,
      message: initial_notice.map(|notice| (notice, Instant::now())),
      connected: None,
      connection_error: None,
      status: None,
      queue: Vec::new(),
      queue_state: ListState::default(),
      follow_current: true,
      prompt: None,
      command_state: CommandState::default(),
      show_help: false,
      lyrics: None,
      lyrics_url: String::new(),
      lyrics_error: None,
      lyrics_scroll: 0,
      lyrics_follow,
      pending_restore_selection: None,
      queue_filter: None,
      queue_filter_matches: Vec::new(),
      metadata_entries: None,
      metadata_url: String::new(),
      metadata_error: None,
      metadata_scroll: 0,
      editor_request: None,
      cover_path: None,
      cover_dims: None,
      cover_error: None,
      detail: None,
      hover: None,
      tag_fallbacks_pending: HashSet::new(),
      tag_fallbacks_done: HashSet::new(),
      has_hover_panes: false,
      visualizer: None,
      help_scroll: 0,
      max_help_scroll: 0,
      lyrics_pane_areas: Vec::new(),
      lyrics_pane_sources: Vec::new(),
      queue_pane_areas: Vec::new(),
      tab_hit_areas: Vec::new(),
      lyrics_cursor: None,
      spectrum: Vec::new(),
      progress_band_area: None,
      band_scrubbing: false,
      dispatcher: KeyDispatcher::default(),
      view_bindings,
      input_bindings,
      help_bindings,
    };
    app.queue_state.select(Some(0));
    app.has_hover_panes = app
      .tabs
      .iter()
      .any(|tab| tab.layout.has_hovered_pane());
    app.sync_hover_view();
    app
  }

  pub fn should_quit(&self) -> bool {
    self.quit
  }

  pub fn set_message(&mut self, message: impl Into<String>) {
    self.message = Some((message.into(), Instant::now()));
  }

  pub fn message_text(&self) -> Option<&str> {
    self.message.as_ref().map(|(text, _)| text.as_str())
  }

  pub fn take_editor_request(&mut self) -> Option<EditorRequest> {
    self.editor_request.take()
  }

  // --- tab helpers ----------------------------------------------------------

  pub fn current_tab(&self) -> &TabLayout {
    self.tabs.get(self.tab).unwrap_or(&self.tabs[0])
  }

  /// The pane whose keymap receives keys on the active tab.
  pub fn main_pane(&self) -> PaneKind {
    self.current_tab().main
  }

  /// Data source of the main pane on the active tab (first pane matching
  /// the main kind wins).
  pub fn main_pane_source(&self) -> PaneSource {
    self
      .current_tab()
      .layout
      .source_of(self.main_pane())
      .unwrap_or(PaneSource::Playing)
  }

  /// Does the active tab contain a pane of this kind?
  pub fn tab_contains(&self, kind: PaneKind) -> bool {
    self.current_tab().layout.contains(kind)
  }

  fn cycle_tab(&mut self, delta: i32) -> bool {
    if self.tabs.len() < 2 {
      return false;
    }
    let len = self.tabs.len() as i32;
    let next = ((self.tab as i32 + delta).rem_euclid(len)) as usize;
    self.goto_tab(next)
  }

  fn goto_tab(&mut self, index: usize) -> bool {
    if index < self.tabs.len() && index != self.tab {
      self.tab = index;
      true
    } else {
      false
    }
  }

  // --- current song helpers ---------------------------------------------------

  pub fn current_song(&self) -> Option<&SongInQueue> {
    let status = self.status.as_ref()?;
    let (position, _) = status.current_song?;
    self.queue.get(position.0)
  }

  pub fn current_song_url(&self) -> Option<String> {
    self.current_song().map(|song| song.song.url.to_string())
  }

  pub fn current_song_path(&self) -> Option<PathBuf> {
    let url = self.current_song_url()?;
    self.music_dir.as_ref().map(|dir| uri_to_path(dir, &url))
  }

  pub fn elapsed(&self) -> f64 {
    let Some(status) = &self.status else { return 0.0 };
    status.elapsed.map(|elapsed| elapsed.as_secs_f64()).unwrap_or(0.0)
  }

  pub fn duration(&self) -> Option<f64> {
    self.status.as_ref().and_then(|status| status.duration).map(|d| d.as_secs_f64())
  }

  // --- event application -------------------------------------------------

  pub(crate) fn active_lyrics_index(&self) -> Option<usize> {
    self.lyrics
      .as_ref()
      .and_then(|lyrics| lyrics.active_index(Duration::from_secs_f64(self.elapsed())))
  }

  /// Seek to the playback position under a screen column of the progress band.
  fn toggle_flag(&self, flag: &str) -> bool {
    let status = self.status.as_ref();
    let current = match flag {
      "repeat" => status.map(|status| status.repeat).unwrap_or(false),
      "random" => status.map(|status| status.random).unwrap_or(false),
      "consume" => status.map(|status| status.consume).unwrap_or(false),
      _ => false,
    };
    !current
  }

  fn toggle_single(&self) -> SingleMode {
    match self.status.as_ref().map(|status| status.single) {
      Some(SingleMode::Disabled) => SingleMode::Enabled,
      _ => SingleMode::Disabled,
    }
  }

  fn mpdc(&self, command: MpdCommand) {
    self.mpd.send(command);
  }

  fn move_selection(&mut self, delta: i32) -> bool {
    let len = self.visible_len();
    if len == 0 {
      return false;
    }
    let next = (self.queue_state.selected().unwrap_or(0) as i32 + delta)
      .clamp(0, len as i32 - 1) as usize;
    self.queue_state.select(Some(next));
    true
  }

  fn move_selection_page(&mut self, direction: i32) -> bool {
    self.move_selection(direction * 10)
  }

  /// Scroll whichever metadata surface is active: the detail view when
  /// open, otherwise the playing-song metadata pane.
  fn scroll_metadata_by(&mut self, delta: i32) {
    if let Some(detail) = self.detail.as_mut() {
      detail.metadata_scroll = if delta < 0 {
        detail.metadata_scroll.saturating_sub(delta.unsigned_abs() as usize)
      } else {
        detail.metadata_scroll.saturating_add(delta as usize)
      };
    } else if self.main_pane() == PaneKind::Metadata
      && self.main_pane_source() == PaneSource::Hovered
      && let Some(hover) = self.hover.as_mut()
    {
      hover.metadata_scroll = if delta < 0 {
        hover.metadata_scroll.saturating_sub(delta.unsigned_abs() as usize)
      } else {
        hover.metadata_scroll.saturating_add(delta as usize)
      };
    } else if delta < 0 {
      self.metadata_scroll = self.metadata_scroll.saturating_sub(delta.unsigned_abs() as usize);
    } else {
      self.metadata_scroll = self.metadata_scroll.saturating_add(delta as usize);
    }
  }

  /// Whether the active tab's main lyrics pane reads the hovered song.
  fn hover_lyrics_active(&self) -> bool {
    self.main_pane() == PaneKind::Lyrics && self.main_pane_source() == PaneSource::Hovered
  }

  /// Scroll the hovered song's lyrics (plain list — no playback state).
  fn scroll_hover_lyrics(&mut self, delta: i32) {
    let Some(hover) = self.hover.as_mut() else { return };
    let line_count = hover.lyrics.as_ref().map(Lyrics::line_count).unwrap_or(0);
    if line_count == 0 {
      return;
    }
    let height = usize::from(
      self
        .lyrics_pane_areas
        .iter()
        .map(|area| area.height)
        .max()
        .unwrap_or(1)
        .max(1),
    );
    let max_scroll = line_count.saturating_sub(height);
    hover.lyrics_scroll = if delta < 0 {
      hover.lyrics_scroll.saturating_sub(delta.unsigned_abs() as usize)
    } else {
      (hover.lyrics_scroll + delta as usize).min(max_scroll)
    };
  }


  pub fn bindings(&self) -> &KeyBindings {
    self
      .view_bindings
      .get(self.main_pane().index())
      .expect("bindings for main pane")
  }
}
/// `mm:ss` for footer/seek messages.
pub(crate) fn format_time(secs: f64) -> String {
  let total = secs.max(0.0) as u64;
  format!("{}:{:02}", total / 60, total % 60)
}