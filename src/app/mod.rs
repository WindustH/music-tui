//! Application state and input handling.

use std::{
  path::{Path, PathBuf},
  sync::Arc,
  time::{Duration, Instant},
};

use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use framework_tui::{
  CommandState, KeyBindings, KeyContext, KeyDispatcher, MatchResult, Prompt, PromptInputResult,
  key_event_to_token,
};
use mpd_client::commands::SingleMode;
use mpd_client::responses::{Song, SongInQueue, Status};
use ratatui::{
  layout::Rect,
  widgets::{ListState, TableState},
};
use tokio::sync::mpsc;
use tracing::debug;

use crate::{
  config::{Settings, expand_home},
  cover,
  event::{
    AsyncEvent, CoverOutcome, LyricsOutcome, MetadataOutcome, MetadataWriteOutcome, MpdEvent,
  },
  layout::{PaneKind, PaneLayout, PaneSource, TabLayout, parse_detail, parse_tabs},
  library::{resolve_music_dir, uri_to_path},
  lyrics::{self, Lyrics},
  metadata,
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

mod actions;
mod commands;
mod detail;
mod editor;
mod input;
mod labels;
mod library;
mod loading;
mod lyrics_view;
mod mouse;
mod outcomes;
mod queue;
mod snapshot;
mod viewport;

pub use detail::SongView;
pub(crate) use labels::{song_album, song_artist, song_title};
pub(crate) use library::FilterTarget;

/// Screen geometry recorded while drawing, for mouse hit-testing. Pane
/// lists are rebuilt every frame (only panes visible on the active tab
/// are hit-testable).
#[derive(Debug, Default)]
pub(crate) struct HitAreas {
  /// Tab labels in the tab bar (click to switch).
  pub tabs: Vec<Rect>,
  /// Inner areas of queue panes (click to select, again to play).
  pub queue_panes: Vec<Rect>,
  /// Scrollbar tracks of queue panes (click / drag the viewport).
  pub queue_bars: Vec<Rect>,
  /// Data viewports of library panes (below the header row).
  pub library_panes: Vec<Rect>,
  pub library_bars: Vec<Rect>,
  /// Inner areas of lyrics panes with the data source each one shows, so
  /// clicks know whether a pane shows the hovered song (no seek) or the
  /// playing song.
  pub lyrics_panes: Vec<(Rect, PaneSource)>,
  /// The bottom progress band (click / drag to seek).
  pub progress_band: Option<Rect>,
}

impl HitAreas {
  pub(crate) fn clear_panes(&mut self) {
    self.queue_panes.clear();
    self.queue_bars.clear();
    self.library_panes.clear();
    self.library_bars.clear();
    self.lyrics_panes.clear();
  }
}

/// What a held left button is dragging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drag {
  QueueBar,
  LibraryBar,
  /// Scrubbing the progress band.
  Band,
}

pub struct App {
  pub(crate) settings: Settings,
  mpd: MpdHandle,
  events: mpsc::UnboundedSender<AsyncEvent>,
  pub(crate) music_dir: Option<PathBuf>,

  /// Parsed tab layouts from the config.
  pub(crate) tabs: Vec<TabLayout>,
  /// Index of the active tab.
  pub(crate) tab: usize,
  /// Layout tree for the secondary detail view (cover + metadata panes).
  pub(crate) detail_layout: PaneLayout,

  quit: bool,
  message: Option<(String, Instant)>,

  pub(crate) connected: Option<String>,
  pub(crate) connection_error: Option<String>,
  pub(crate) status: Option<Status>,
  pub(crate) queue: Arc<[SongInQueue]>,
  pub(crate) queue_state: ListState,
  pub(crate) follow_current: bool,
  /// Queued selection restore from the persisted state, applied once the
  /// first queue snapshot arrives.
  pending_restore_selection: Option<usize>,
  /// Active queue filter (case-insensitive substring over title / artist /
  /// album / url), entered via `/`.
  pub(crate) queue_filter: Option<String>,
  /// Hide duplicate queue entries (same URL keeps its first occurrence
  /// visible; the playing copy stays visible) — from [behavior] config.
  pub(crate) queue_dedup: bool,
  /// Queue positions matching the filter; selection indexes this list.
  pub(crate) queue_filter_matches: Vec<usize>,
  /// Volume to restore when unmuting.
  pre_mute_volume: Option<u8>,

  pub(crate) prompt: Option<Prompt>,
  pub(crate) command_state: CommandState,
  /// Which list `/` filters (queue or library).
  filter_target: FilterTarget,
  pub(crate) show_help: bool,
  /// Scroll position of the f1 key-help dialog.
  pub(crate) help_scroll: usize,
  /// Maximum scroll of the key-help dialog, updated at draw time.
  pub(crate) max_help_scroll: usize,

  /// Data view for the playing song (`:playing` panes); `None` while
  /// nothing plays or the song has no local file.
  pub(crate) playing: Option<SongView>,
  pub(crate) lyrics_follow: bool,
  /// Selected lyric line while in manual scroll mode.
  pub(crate) lyrics_cursor: Option<usize>,
  editor_request: Option<EditorRequest>,
  /// Secondary detail view for the selected queue/library entry (`i`).
  pub(crate) detail: Option<SongView>,
  /// Data view for the hovered queue row (`:hovered` pane source).
  hover: Option<SongView>,
  /// Hover view fed by the library pane selection (`:library-hovered`).
  library_hover: Option<SongView>,
  /// Whether any configured pane uses the hovered data source (gates the
  /// lazy loading in `sync_hover_view`).
  has_hover_panes: bool,
  /// Some pane uses `:library-hovered`; enables the library hover view.
  has_library_hover_panes: bool,

  /// Library database state (see `library.rs`): every track, and the
  /// visible rows (indices into `library`, filtered and ranked).
  pub(crate) library: Vec<crate::library_db::LibraryTrack>,
  pub(crate) library_rows: Vec<crate::library_db::TrackMatch>,
  pub(crate) library_filter: Option<String>,
  /// Filter the current `library_rows` were computed for; a query that
  /// extends it only needs to re-check those rows.
  library_rows_query: String,
  pub(crate) library_state: TableState,
  /// Scan progress while the scanner thread is running.
  pub(crate) library_scanning: Option<(usize, usize)>,
  /// Parsed `[library] columns` config.
  pub(crate) library_columns: Vec<crate::config::LibraryColumn>,
  /// Sender to the library scanner thread (rescan requests).
  pub(crate) library_scan_tx: Option<std::sync::mpsc::Sender<()>>,
  /// A rescan was requested with `u` (announce when it finishes).
  library_rescan_requested: bool,

  /// Visualizer worker handle (reports pane width for band allocation).
  pub(crate) visualizer: Option<crate::visualizer::VisualizerHandle>,
  /// Off-thread band renderer; layout + styled lines happen on a worker.
  visualizer_renderer: Option<crate::visualizer::BandRendererHandle>,
  /// Last visualizer pane size seen while drawing.
  visualizer_geometry: Option<(u16, u16)>,
  /// Latest precomputed visualizer lines ready to blit.
  pub(crate) visualizer_lines: Option<Vec<ratatui::text::Line<'static>>>,
  spectrum: Vec<u8>,

  /// Geometry recorded at draw time for mouse hit tests.
  pub(crate) hit: HitAreas,
  drag: Option<Drag>,

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
    let queue_dedup = settings.config.behavior.queue_dedup;
    let library_columns = settings.config.library.columns.clone();
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
    let view_bindings = PaneKind::ALL
      .iter()
      .map(|pane| settings.keymap.pane_bindings(*pane))
      .collect();
    let input_bindings = settings.keymap.input_only_bindings();
    let help_bindings = settings.keymap.help_bindings();
    let has_hover_panes = tabs
      .iter()
      .any(|tab| tab.layout.has_source(PaneSource::QueueHovered));
    let has_library_hover_panes = tabs
      .iter()
      .any(|tab| tab.layout.has_source(PaneSource::LibraryHovered));
    let mut app = Self {
      mpd,
      events,
      music_dir,
      tabs,
      tab: 0,
      detail_layout,
      settings,
      quit: false,
      message: initial_notice.map(|notice| (notice, Instant::now())),
      connected: None,
      connection_error: None,
      status: None,
      queue: Arc::default(),
      queue_state: ListState::default(),
      follow_current: true,
      pending_restore_selection: None,
      queue_filter: None,
      queue_dedup,
      queue_filter_matches: Vec::new(),
      pre_mute_volume: None,
      prompt: None,
      command_state: CommandState::default(),
      filter_target: FilterTarget::Queue,
      show_help: false,
      help_scroll: 0,
      max_help_scroll: 0,
      playing: None,
      lyrics_follow,
      lyrics_cursor: None,
      editor_request: None,
      detail: None,
      hover: None,
      library_hover: None,
      has_hover_panes,
      has_library_hover_panes,
      library: Vec::new(),
      library_rows: Vec::new(),
      library_filter: None,
      library_rows_query: String::new(),
      library_state: TableState::default(),
      library_scanning: None,
      library_columns,
      library_scan_tx: None,
      library_rescan_requested: false,
      visualizer: None,
      visualizer_renderer: None,
      visualizer_geometry: None,
      visualizer_lines: None,
      spectrum: Vec::new(),
      hit: HitAreas::default(),
      drag: None,
      dispatcher: KeyDispatcher::default(),
      view_bindings,
      input_bindings,
      help_bindings,
    };
    app.queue_state.select(Some(0));
    app.library_state.select(Some(0));
    app.sync_hover_view();
    app
  }

  /// Attach the optional background workers spawned next to the app.
  pub fn attach_workers(
    &mut self,
    visualizer: Option<crate::visualizer::VisualizerHandle>,
    visualizer_renderer: Option<crate::visualizer::BandRendererHandle>,
    library_scan_tx: Option<std::sync::mpsc::Sender<()>>,
  ) {
    self.visualizer = visualizer;
    self.visualizer_renderer = visualizer_renderer;
    self.library_scan_tx = library_scan_tx;
  }

  pub fn should_quit(&self) -> bool {
    self.quit
  }

  pub fn set_message(&mut self, message: impl Into<String>) {
    let message = crate::sanitize::sanitize_text(&message.into());
    self.message = Some((message, Instant::now()));
  }

  pub fn message_text(&self) -> Option<&str> {
    self.message.as_ref().map(|(text, _)| text.as_str())
  }

  /// When the footer message expires (drives the main loop's timer).
  pub fn message_deadline(&self) -> Option<Instant> {
    self.message.as_ref().map(|(_, at)| *at + MESSAGE_TTL)
  }

  /// Drop the footer message once it has been shown long enough; returns
  /// whether the screen changed.
  pub fn expire_message(&mut self, now: Instant) -> bool {
    if self
      .message_deadline()
      .is_some_and(|deadline| deadline <= now)
    {
      self.message = None;
      return true;
    }
    false
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

  /// Is a pane of this kind on screen right now? The detail view replaces
  /// the tab content while it is open.
  pub(crate) fn pane_visible(&self, kind: PaneKind) -> bool {
    self.detail.is_none() && self.tab_contains(kind)
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
    self.queue.get(self.playing_position()?)
  }

  /// The data view backing a pane source.
  pub(crate) fn song_view(&self, source: PaneSource) -> Option<&SongView> {
    match source {
      PaneSource::Playing => self.playing.as_ref(),
      PaneSource::QueueHovered => self.hover.as_ref(),
      PaneSource::LibraryHovered => self.library_hover.as_ref(),
    }
  }

  fn song_view_mut(&mut self, source: PaneSource) -> Option<&mut SongView> {
    match source {
      PaneSource::Playing => self.playing.as_mut(),
      PaneSource::QueueHovered => self.hover.as_mut(),
      PaneSource::LibraryHovered => self.library_hover.as_mut(),
    }
  }

  pub fn current_song_url(&self) -> Option<String> {
    self.current_song().map(|song| song.song.url.to_string())
  }

  pub fn elapsed(&self) -> f64 {
    let Some(status) = &self.status else {
      return 0.0;
    };
    status
      .elapsed
      .map(|elapsed| elapsed.as_secs_f64())
      .unwrap_or(0.0)
  }

  pub fn duration(&self) -> Option<f64> {
    self
      .status
      .as_ref()
      .and_then(|status| status.duration)
      .map(|d| d.as_secs_f64())
  }

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

  /// Mute, or restore the volume from before muting.
  fn toggle_mute(&mut self) {
    let volume = self.status.as_ref().map(|status| status.volume);
    if volume == Some(0) {
      let restore = self.pre_mute_volume.take().filter(|volume| *volume > 0);
      self.mpdc(MpdCommand::SetVolume(restore.unwrap_or(50)));
    } else {
      self.pre_mute_volume = volume;
      self.mpdc(MpdCommand::SetVolume(0));
    }
  }

  /// Scroll whichever metadata surface is active: the detail view when
  /// open, otherwise the main pane's source view.
  fn scroll_metadata_by(&mut self, delta: i32) {
    let view = if self.detail.is_some() {
      self.detail.as_mut()
    } else {
      let source = if self.main_pane() == PaneKind::Metadata {
        self.main_pane_source()
      } else {
        PaneSource::Playing
      };
      self.song_view_mut(source)
    };
    if let Some(view) = view {
      view.metadata_scroll = view.metadata_scroll.saturating_add_signed(delta as isize);
    }
  }

  /// Binding tables for the current tab as a priority queue: the main
  /// pane first, then the tab's other panes in layout order (dedup).
  /// Key dispatch walks this queue, so keys the main pane does not claim
  /// fall through to neighboring panes in the same tab.
  pub(crate) fn pane_bindings(&self) -> Vec<&KeyBindings> {
    let main = self.main_pane();
    let mut panes = self.current_tab().layout.pane_kinds();
    panes.sort_by_key(|pane| (*pane != main) as u8);
    panes.dedup();
    panes
      .into_iter()
      .filter_map(|pane| self.view_bindings.get(pane.index()))
      .collect()
  }
}

impl App {
  /// Capture the current UI state for persistence.
  pub fn snapshot_state(&self) -> crate::state::PersistedState {
    crate::state::PersistedState {
      tab: self.tab,
      lyrics_follow: Some(self.lyrics_follow),
      queue_selected: self.queue_state.selected(),
    }
  }

  /// Apply a previously persisted state (called once at startup).
  pub fn restore_state(&mut self, state: crate::state::PersistedState) {
    if !self.tabs.is_empty() {
      self.tab = state.tab.min(self.tabs.len() - 1);
    }
    if let Some(follow) = state.lyrics_follow {
      self.lyrics_follow = follow;
    }
    self.pending_restore_selection = state.queue_selected;
  }
}

/// How long a footer message stays visible.
const MESSAGE_TTL: Duration = Duration::from_secs(4);

/// `mm:ss` for footer/seek messages.
pub(crate) fn format_time(secs: f64) -> String {
  let total = secs.max(0.0) as u64;
  format!("{}:{:02}", total / 60, total % 60)
}
