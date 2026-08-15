//! Application state and input handling.

use std::{
  path::PathBuf,
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
  widgets::ListState,
};
use tokio::sync::mpsc;
use tracing::debug;

use crate::{
  config::{Settings, expand_home},
  cover,
  event::{AsyncEvent, CoverOutcome, LyricsOutcome, MpdEvent, MetadataOutcome, MetadataWriteOutcome},
  keymap::KeymapConfig,
  layout::{PaneKind, TabLayout, parse_tabs},
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

/// Secondary detail view for a queue entry (the `i` key), in the spirit of
/// gallery-tui's image detail view: the sidebar always shows the playing
/// song, details open as their own full-screen surface.
pub struct DetailView {
  pub url: String,
  pub path: PathBuf,
  pub title: String,
  pub metadata: Option<Vec<metadata::MetadataEntry>>,
  pub metadata_error: Option<String>,
  pub metadata_scroll: u16,
  pub cover: Option<PathBuf>,
  pub cover_dims: Option<(u32, u32)>,
  pub cover_error: Option<String>,
}

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
  pub lyrics_scroll: u16,
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
  pub metadata_scroll: u16,
  pub editor_request: Option<EditorRequest>,

  pub cover_path: Option<(String, PathBuf)>,
  pub cover_dims: Option<(u32, u32)>,
  pub cover_error: Option<String>,
  /// Secondary detail view for the selected queue entry (`i`).
  pub detail: Option<DetailView>,
  /// Scroll position of the f1 key-help dialog.
  pub help_scroll: usize,
  /// Maximum scroll of the key-help dialog, updated at draw time.
  pub max_help_scroll: usize,
  /// Inner screen areas of lyrics panes in the current tab, recorded at
  /// draw time so clicks can be mapped to lyric lines.
  pub lyrics_pane_areas: Vec<Rect>,
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

const COMMANDS: &[&str] = &[
  "quit", "q", "help", "play", "pause", "toggle", "stop", "next", "prev", "volume", "vol",
  "repeat", "random", "single", "consume", "clear", "update", "tab", "add",
];

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
    let view_bindings = build_bindings(&settings.keymap);
    let input_bindings = build_input_bindings(&settings.keymap);
    let help_bindings = settings.keymap.help_bindings();
    let mut app = Self {
      mpd,
      events: events.clone(),
      music_dir,
      tabs,
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
      help_scroll: 0,
      max_help_scroll: 0,
      lyrics_pane_areas: Vec::new(),
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

  pub fn handle_mpd_event(&mut self, event: MpdEvent) -> bool {
    match event {
      MpdEvent::Connected(address) => {
        self.connected = Some(address);
        self.connection_error = None;
        true
      }
      MpdEvent::ConnectionLost(reason) => {
        self.connected = None;
        self.connection_error = Some(reason);
        self.status = None;
        true
      }
      MpdEvent::Notice(notice) => {
        self.set_message(notice);
        true
      }
      MpdEvent::Snapshot { status, queue } => {
        let song_changed = Self::snapshot_song_url(&status, &queue).as_deref()
          != self.current_song_url().as_deref();
        self.status = Some(status);
        self.queue = queue;
        self.recompute_queue_filter();
        self.clamp_queue_selection();
        if let Some(position) = self
          .pending_restore_selection
          .take()
          .filter(|position| *position < self.queue.len())
          && self.queue_state.selected().is_none_or(|current| current == 0)
        {
          self.queue_state.select(Some(position));
        }
        if song_changed {
          self.on_song_changed();
        }
        true
      }
    }
  }

  fn snapshot_song_url(status: &Status, queue: &[SongInQueue]) -> Option<String> {
    let (position, _) = status.current_song?;
    queue.get(position.0).map(|song| song.song.url.to_string())
  }

  fn clamp_queue_selection(&mut self) {
    if self.queue_filter_matches.is_empty() {
      self.queue_state.select(None);
      return;
    }
    let len = self.queue_filter_matches.len();
    let current = self.queue_state.selected().unwrap_or(0).min(len - 1);
    self.queue_state.select(Some(current));
  }

  /// Number of rows visible in the queue pane (filtered or not).
  fn visible_len(&self) -> usize {
    self.queue_filter_matches.len()
  }

  /// Map the selection (an index into the visible rows) to a queue position.
  fn filtered_position(&self, selected: usize) -> Option<usize> {
    self.queue_filter_matches.get(selected).copied()
  }

  fn song_matches_filter(song: &Song, needle: &str) -> bool {
    let needle = needle.to_lowercase();
    if song.title().is_some_and(|title| title.to_lowercase().contains(&needle)) {
      return true;
    }
    if song
      .artists()
      .iter()
      .any(|artist| artist.to_lowercase().contains(&needle))
    {
      return true;
    }
    if song
      .album()
      .is_some_and(|album| album.to_lowercase().contains(&needle))
    {
      return true;
    }
    song.url.to_lowercase().contains(&needle)
  }

  fn recompute_queue_filter(&mut self) {
    self.queue_filter_matches = match self.queue_filter.as_deref() {
      None | Some("") => (0..self.queue.len()).collect(),
      Some(needle) => self
        .queue
        .iter()
        .enumerate()
        .filter(|(_, song)| Self::song_matches_filter(&song.song, needle))
        .map(|(position, _)| position)
        .collect(),
    };
  }

  fn clear_queue_filter(&mut self) {
    self.queue_filter = None;
    self.recompute_queue_filter();
    self.clamp_queue_selection();
  }

  fn follow_playing_position(&mut self) {
    if let Some(status) = &self.status
      && let Some((position, _)) = status.current_song
    {
      let row = self
        .queue_filter_matches
        .iter()
        .position(|candidate| *candidate == position.0)
        .or(if self.queue_filter.is_none() { Some(position.0) } else { None });
      if let Some(row) = row {
        self.queue_state.select(Some(row));
      }
    }
  }

  fn on_song_changed(&mut self) {
    self.lyrics = None;
    self.lyrics_error = None;
    self.lyrics_scroll = 0;
    self.lyrics_cursor = None;
    self.metadata_entries = None;
    self.metadata_error = None;
    self.metadata_scroll = 0;
    self.cover_path = None;
    self.cover_dims = None;
    self.cover_error = None;
    if self.follow_current {
      self.follow_playing_position();
    }
    if let (Some(url), Some(path)) = (self.current_song_url(), self.current_song_path()) {
      self.request_lyrics(url.clone(), path.clone());
      self.request_metadata(url.clone(), path.clone());
      self.request_cover(url, path);
    }
  }

  fn request_lyrics(&mut self, url: String, path: PathBuf) {
    self.lyrics_url = url.clone();
    let extra_dirs: Vec<PathBuf> = self
      .settings
      .config
      .lyrics
      .extra_dirs
      .iter()
      .map(|dir| expand_home(dir))
      .collect();
    let (artist, title) = self.current_song_tags();
    let tx = self.events.clone();
    tokio::task::spawn_blocking(move || {
      let result = lyrics::load(&path, &extra_dirs, artist.as_deref(), title.as_deref());
      let _ = tx.send(AsyncEvent::Lyrics(LyricsOutcome { song_url: url, result }));
    });
  }

  fn current_song_tags(&self) -> (Option<String>, Option<String>) {
    let song = self.current_song();
    (
      song.and_then(|song| song.song.artists().first().cloned()),
      song.map(|song| {
        song
          .song
          .title()
          .map(str::to_string)
          .unwrap_or_else(|| song.song.url.clone())
      }),
    )
  }

  fn request_metadata(&mut self, url: String, path: PathBuf) {
    self.metadata_url = url.clone();
    self.spawn_metadata_read(url, path);
  }

  fn spawn_metadata_read(&self, url: String, path: PathBuf) {
    let tx = self.events.clone();
    tokio::task::spawn_blocking(move || {
      let result = metadata::read_metadata(&path);
      let _ = tx.send(AsyncEvent::Metadata(MetadataOutcome { song_url: url, result }));
    });
  }

  fn spawn_cover_read(&self, url: String, path: PathBuf) {
    let cache_dir = self.settings.cache_dir.join("covers");
    let tx = self.events.clone();
    tokio::task::spawn_blocking(move || {
      let result = cover::find_cover(&path, &cache_dir);
      let dims = result
        .as_ref()
        .ok()
        .and_then(|path| image::image_dimensions(path).ok());
      let _ = tx.send(AsyncEvent::Cover(CoverOutcome { song_url: url, result, dims }));
    });
  }

  fn request_cover(&mut self, url: String, path: PathBuf) {
    self.spawn_cover_read(url, path);
  }

  fn song_path(&self, url: &str) -> Option<PathBuf> {
    self.music_dir.as_ref().map(|dir| uri_to_path(dir, url))
  }

  /// Open the secondary detail view for the selected queue entry (`i`), in
  /// the spirit of gallery-tui's image view: the sidebar keeps showing the
  /// playing song; details live on their own surface until closed.
  fn open_detail(&mut self) -> bool {
    let Some(index) = self.queue_state.selected() else {
      return false;
    };
    let Some(index) = self.filtered_position(index) else {
      return false;
    };
    let Some(song) = self.queue.get(index) else {
      return false;
    };
    let url = song.song.url.to_string();
    if self.detail.as_ref().is_some_and(|detail| detail.url == url) {
      self.close_detail();
      return true;
    }
    let Some(path) = self.song_path(&url) else {
      self.set_message("song is not under music_dir");
      return true;
    };
    let title = song
      .song
      .title()
      .map(str::to_string)
      .unwrap_or_else(|| url.clone());
    self.detail = Some(DetailView {
      url: url.clone(),
      path: path.clone(),
      title,
      metadata: None,
      metadata_error: None,
      metadata_scroll: 0,
      cover: None,
      cover_dims: None,
      cover_error: None,
    });
    self.spawn_metadata_read(url.clone(), path.clone());
    self.spawn_cover_read(url, path);
    true
  }

  fn close_detail(&mut self) {
    self.detail = None;
  }

  /// `g` / `c` in the queue: jump the selection (and view) to the song that
  /// is currently playing.
  fn goto_playing(&mut self) -> bool {
    let Some(position) = self.status.as_ref().and_then(|status| status.current_song) else {
      self.set_message("nothing is playing");
      return true;
    };
    let row = self
      .queue_filter_matches
      .iter()
      .position(|candidate| *candidate == position.0 .0)
      .unwrap_or(position.0 .0);
    self.select_queue_row(row);
    true
  }

  pub fn handle_lyrics_outcome(&mut self, outcome: LyricsOutcome) -> bool {
    if outcome.song_url != self.lyrics_url {
      return false;
    }
    match outcome.result {
      Ok(lyrics) => {
        self.lyrics = Some(lyrics);
        self.lyrics_error = None;
      }
      Err(error) => {
        self.lyrics = None;
        self.lyrics_error = Some(error);
      }
    }
    true
  }

  pub fn handle_metadata_outcome(&mut self, outcome: MetadataOutcome) -> bool {
    let mut handled = false;
    if let Some(detail) = self.detail.as_mut()
      && detail.url == outcome.song_url
    {
      match &outcome.result {
        Ok(entries) => {
          detail.metadata = Some(entries.clone());
          detail.metadata_error = None;
        }
        Err(error) => {
          detail.metadata = None;
          detail.metadata_error = Some(error.clone());
        }
      }
      handled = true;
    }
    if outcome.song_url == self.metadata_url {
      match outcome.result {
        Ok(entries) => {
          self.metadata_entries = Some(entries);
          self.metadata_error = None;
        }
        Err(error) => {
          self.metadata_entries = None;
          self.metadata_error = Some(error);
        }
      }
      handled = true;
    }
    handled
  }

  pub fn handle_metadata_write_outcome(&mut self, outcome: MetadataWriteOutcome) -> bool {
    match outcome.result {
      Ok(()) => {
        self.set_message(format!("metadata updated: {} tag(s)", outcome.changed_tags));
        if outcome.song_url == self.metadata_url {
          self.metadata_entries = None;
          if let Some(path) = self.current_song_path() {
            self.request_metadata(outcome.song_url, path);
          }
        }
        true
      }
      Err(error) => {
        self.set_message(format!("metadata write failed: {error}"));
        true
      }
    }
  }

  pub fn handle_cover_outcome(&mut self, outcome: CoverOutcome) -> bool {
    let mut handled = false;
    if let Some(detail) = self.detail.as_mut()
      && detail.url == outcome.song_url
    {
      match &outcome.result {
        Ok(path) => {
          detail.cover_dims = outcome.dims;
          detail.cover = Some(path.clone());
          detail.cover_error = None;
        }
        Err(error) => {
          detail.cover = None;
          detail.cover_error = Some(error.clone());
        }
      }
      handled = true;
    }
    match outcome.result {
      Ok(path) => {
        self.cover_dims = outcome.dims;
        self.cover_path = Some((outcome.song_url.clone(), path));
        self.cover_error = None;
      }
      Err(error) => {
        if outcome.song_url == self.current_song_url().unwrap_or_default() {
          self.cover_path = None;
          self.cover_error = Some(error);
        }
      }
    }
    handled || self.tab_contains(PaneKind::Cover)
  }

  pub fn handle_spectrum(&mut self, bars: Vec<u8>) -> bool {
    self.spectrum = bars;
    self.tab_contains(PaneKind::Visualizer)
  }

  pub fn handle_tick(&mut self) -> bool {
    if let Some((_, at)) = self.message
      && at.elapsed() > Duration::from_secs(4)
    {
      self.message = None;
      return true;
    }
    false
  }

  // --- input --------------------------------------------------------------

  pub fn handle_input(&mut self, event: Event) -> bool {
    if let Event::Paste(value) = event {
      if let Some(prompt) = self.prompt.as_mut() {
        let result =
          framework_tui::input::handle_prompt_paste(prompt, &mut self.command_state, &value);
        return self.apply_prompt_result(result);
      }
      return false;
    }
    if let Event::Mouse(mouse) = event {
      return self.handle_mouse(mouse);
    }
    let Event::Key(key) = event else {
      return false;
    };

    if let Some(prompt) = self.prompt.as_mut() {
      let result = framework_tui::input::handle_prompt_key(
        prompt,
        &mut self.command_state,
        &self.input_bindings,
        key,
      );
      return self.apply_prompt_result(result);
    }

    if self.show_help {
      return match framework_tui::handle_help_dialog_key(
        &mut self.help_scroll,
        self.max_help_scroll,
        &self.help_bindings,
        key,
      ) {
        framework_tui::HelpDialogInput::Scrolled => true,
        framework_tui::HelpDialogInput::Closed => {
          self.show_help = false;
          true
        }
        framework_tui::HelpDialogInput::Unhandled => false,
      };
    }

    let Some(token) = key_event_to_token(key) else {
      return false;
    };
    let bindings = self
      .view_bindings
      .get(self.main_pane().index())
      .expect("bindings for main pane");
    match self.dispatcher.dispatch(bindings, KeyContext::Browser, token) {
      MatchResult::Action(action) => {
        self.dispatcher.clear();
        debug!(%action, "action dispatched");
        self.run_action(&action)
      }
      MatchResult::Prefix(_) => true,
      MatchResult::None => false,
    }
  }

  fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
    if self.show_help {
      match mouse.kind {
        MouseEventKind::Down(_) => {
          self.show_help = false;
          true
        }
        MouseEventKind::ScrollUp => self.scroll_help(-3),
        MouseEventKind::ScrollDown => self.scroll_help(3),
        _ => false,
      }
    } else {
      self.handle_mouse_on_interface(mouse)
    }
  }

  fn handle_mouse_on_interface(&mut self, mouse: MouseEvent) -> bool {
    // Clicking a tab label in the tab bar switches to that tab.
    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
      && let Some(index) = self
        .tab_hit_areas
        .iter()
        .position(|area| {
          mouse.row == area.y
            && mouse.column >= area.x
            && mouse.column < area.x + area.width
        })
      && index != self.tab
    {
      self.goto_tab(index);
      return true;
    }
    // Clicking a synced lyric line seeks to its timestamp (only when the
    // click lands inside a lyrics pane, both rows and columns).
    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
      && let Some(index) = self.lyrics_index_at(mouse)
    {
      let _ = self.lyrics_seek_to(index);
      return true;
    }
    match mouse.kind {
      MouseEventKind::Down(MouseButton::Left) => {
        if let Some(row) = self.queue_row_index(mouse) {
          // Click selects; clicking the already-selected row (or a quick
          // second click) plays it — double-click without the timer.
          let selected = self.queue_state.selected();
          if selected == Some(row) {
            self.play_selected_queue_row();
          } else {
            self.select_queue_row(row);
          }
          return true;
        }
        self.band_scrubbing = self.mouse_on_band(mouse);
        if self.band_scrubbing {
          return self.seek_to_band_column(mouse.column);
        }
        false
      }
      MouseEventKind::Drag(MouseButton::Left) => {
        if self.band_scrubbing {
          return self.seek_to_band_column(mouse.column);
        }
        false
      }
      MouseEventKind::Up(MouseButton::Left) => {
        let was_scrubbing = self.band_scrubbing;
        self.band_scrubbing = false;
        was_scrubbing
      }
      MouseEventKind::ScrollUp => {
        if self.mouse_on_band(mouse) {
          self.mpdc(MpdCommand::NudgeSeek(5));
          true
        } else if self.mouse_on_queue(mouse).is_some() {
          self.scroll_queue_viewport(-3)
        } else if self.mouse_on_lyrics(mouse).is_some() {
          self.scroll_lyrics_viewport(-3)
        } else {
          false
        }
      }
      MouseEventKind::ScrollDown => {
        if self.mouse_on_band(mouse) {
          self.mpdc(MpdCommand::NudgeSeek(-5));
          true
        } else if self.mouse_on_queue(mouse).is_some() {
          self.scroll_queue_viewport(3)
        } else if self.mouse_on_lyrics(mouse).is_some() {
          self.scroll_lyrics_viewport(3)
        } else {
          false
        }
      }
      MouseEventKind::Down(MouseButton::Middle) => {
        if let Some(row) = self.queue_row_index(mouse) {
          self.select_queue_row(row);
          self.play_selected_queue_row();
          return true;
        }
        false
      }
      _ => false,
    }
  }

  fn mouse_on_lyrics(&self, mouse: MouseEvent) -> Option<Rect> {
    self.lyrics_pane_areas.iter().copied().find(|area| {
      mouse.row >= area.y
        && mouse.row < area.y + area.height
        && mouse.column >= area.x
        && mouse.column < area.x + area.width
    })
  }

  /// Map a mouse position to the visible lyric line under it (rows and
  /// columns must both be inside a lyrics pane).
  fn lyrics_index_at(&self, mouse: MouseEvent) -> Option<usize> {
    let area = self.mouse_on_lyrics(mouse)?;
    Some((mouse.row - area.y) as usize + self.lyrics_scroll as usize)
  }

  /// Scroll the queue by moving the viewport and letting the selection
  /// follow just enough to stay inside the visible window — the mouse
  /// scrolling convention the user asked for.
  fn scroll_queue_viewport(&mut self, delta: i32) -> bool {
    let len = self.visible_len();
    if len == 0 {
      return false;
    }
    let height = self
      .queue_pane_areas
      .first()
      .map(|area| area.height as usize)
      .filter(|height| *height > 0)
      .unwrap_or(1);
    let offset = self.queue_state.offset() as i32;
    let max_offset = len.saturating_sub(height) as i32;
    let next = (offset + delta).clamp(0, max_offset.max(0)) as usize;
    if next == self.queue_state.offset() {
      return false;
    }
    // Selection follows the viewport: clamp it into the new window.
    let selected = self.queue_state.selected().unwrap_or(next);
    let selected = selected.clamp(next, (next + height - 1).min(len - 1));
    let mut state = ratatui::widgets::ListState::default();
    state.select(Some(selected));
    self.queue_state = state.with_offset(next);
    true
  }

  /// Wheel-scroll over lyrics: leaves follow mode and pans the text window.
  /// Max inner height of the lyrics panes in the current tab (viewport
  /// height for scroll clamping), recorded at draw time.
  pub fn lyrics_view_height(&self) -> u16 {
    self
      .lyrics_pane_areas
      .iter()
      .map(|area| area.height)
      .max()
      .unwrap_or(0)
  }

  /// Scroll the lyrics viewport (wheel): the offset moves, the pointer
  /// passively follows and is clamped back inside the new viewport — same
  /// semantics as the queue view.
  fn scroll_lyrics_viewport(&mut self, delta: i32) -> bool {
    self.lyrics_follow = false;
    let line_count = self
      .lyrics
      .as_ref()
      .map(Lyrics::line_count)
      .unwrap_or(0);
    if line_count == 0 {
      return false;
    }
    let height = usize::from(self.lyrics_view_height().max(1));
    let max_scroll = line_count.saturating_sub(height);
    let base = usize::from(self.lyrics_scroll);
    let next = if delta < 0 {
      base.saturating_sub(delta.unsigned_abs() as usize)
    } else {
      base.saturating_add(delta.unsigned_abs() as usize)
    }
    .min(max_scroll);
    self.lyrics_scroll = next as u16;

    // The pointer only moves as much as needed to stay inside the viewport.
    let pointer = self
      .lyrics_cursor
      .unwrap_or_else(|| self.active_lyrics_index().unwrap_or(0));
    let last_visible = (next + height).saturating_sub(1).min(line_count.saturating_sub(1));
    self.lyrics_cursor = Some(pointer.clamp(next, last_visible));
    true
  }

  /// Scroll the f1 help dialog, clamped to the range computed at draw time.
  fn scroll_help(&mut self, delta: i32) -> bool {
    if self.max_help_scroll == 0 {
      return false;
    }
    let next = if delta < 0 {
      self.help_scroll.saturating_sub(delta.unsigned_abs() as usize)
    } else {
      self.help_scroll.saturating_add(delta as usize)
    };
    let next = next.min(self.max_help_scroll);
    if next == self.help_scroll {
      return false;
    }
    self.help_scroll = next;
    true
  }

  fn mouse_on_band(&self, mouse: MouseEvent) -> bool {
    self.progress_band_area.is_some_and(|area| {
      mouse.row == area.y && mouse.column >= area.x && mouse.column < area.x + area.width
    })
  }

  fn mouse_on_queue(&self, mouse: MouseEvent) -> Option<Rect> {
    self.queue_pane_areas.iter().copied().find(|area| {
      mouse.row >= area.y
        && mouse.row < area.y + area.height
        && mouse.column >= area.x
        && mouse.column < area.x + area.width
    })
  }

  /// Map a screen position to the visible queue row under it.
  fn queue_row_index(&self, mouse: MouseEvent) -> Option<usize> {
    let area = self.mouse_on_queue(mouse)?;
    let row = (mouse.row - area.y) as usize + self.queue_state.offset();
    (row < self.visible_len()).then_some(row)
  }

  fn select_queue_row(&mut self, row: usize) {
    self.queue_state.select(Some(row.min(self.visible_len().saturating_sub(1))));
  }

  fn play_selected_queue_row(&mut self) {
    if let Some(position) = self
      .queue_state
      .selected()
      .and_then(|row| self.filtered_position(row))
    {
      self.mpdc(MpdCommand::PlayPosition(position as u32));
    }
  }

  /// Seek to a synced lyric line and return whether anything happened.
  fn lyrics_seek_to(&mut self, index: usize) -> bool {
    let Some(Lyrics::Synced(lines)) = self.lyrics.as_ref() else {
      return false;
    };
    let Some(line) = lines.get(index) else {
      return false;
    };
    self.mpdc(MpdCommand::SeekCurrent(line.time_secs.max(0.0)));
    self.lyrics_follow = true;
    self.lyrics_cursor = None;
    self.set_message(format!("seek to {}", format_time(line.time_secs)));
    true
  }

  fn active_lyrics_index(&self) -> Option<usize> {
    self.lyrics
      .as_ref()
      .and_then(|lyrics| lyrics.active_index(Duration::from_secs_f64(self.elapsed())))
  }

  /// Seek to the playback position under a screen column of the progress band.
  fn seek_to_band_column(&mut self, column: u16) -> bool {
    let Some(area) = self.progress_band_area else {
      return false;
    };
    let Some(duration) = self.duration().filter(|duration| *duration > 0.0) else {
      return false;
    };
    let ratio = (f64::from(column.saturating_sub(area.x)) + 0.5) / f64::from(area.width);
    let position = (ratio.clamp(0.0, 1.0) * duration).max(0.0);
    self.mpdc(MpdCommand::SeekCurrent(position));
    true
  }

  fn apply_prompt_result(&mut self, result: PromptInputResult) -> bool {
    if self
      .prompt
      .as_ref()
      .is_some_and(|prompt| !prompt.is_command())
    {
      return self.apply_filter_prompt_result(result);
    }
    match result {
      PromptInputResult::Unhandled => false,
      PromptInputResult::Changed => {
        self.refresh_prompt_completion();
        true
      }
      PromptInputResult::Cancel => {
        self.prompt = None;
        self.command_state.reset_prompt_state();
        self.dispatcher.clear();
        self.set_message("cancelled");
        true
      }
      PromptInputResult::Submit => {
        let Some(prompt) = self.prompt.take() else {
          return false;
        };
        let input = prompt.buffer().input.trim().to_string();
        self.command_state.reset_prompt_state();
        self.dispatcher.clear();
        if input.is_empty() {
          return true;
        }
        self.command_state.push_history(input.clone());
        self.run_command_line(&input);
        true
      }
      PromptInputResult::EditInEditor { input } => {
        self.set_message("editing the command in an editor is not supported yet");
        let _ = input;
        true
      }
      PromptInputResult::UnknownAction(action) if action == "help" => {
        self.show_help = true;
        true
      }
      PromptInputResult::UnknownAction(action) => {
        self.set_message(format!("unknown input action: {action}"));
        true
      }
    }
  }

  /// The `/` queue filter prompt: typing filters live, enter keeps the
  /// filter, esc exits the filter state entirely.
  fn apply_filter_prompt_result(&mut self, result: PromptInputResult) -> bool {
    match result {
      PromptInputResult::Unhandled | PromptInputResult::UnknownAction(_) => false,
      PromptInputResult::Changed => {
        if let Some(input) = self.prompt.as_ref().map(Prompt::buffer).map(|buffer| buffer.input.clone()) {
          self.queue_filter = (!input.is_empty()).then_some(input);
          self.recompute_queue_filter();
          self.clamp_queue_selection();
        }
        true
      }
      PromptInputResult::Cancel => {
        self.prompt = None;
        self.command_state.reset_prompt_state();
        self.dispatcher.clear();
        self.clear_queue_filter();
        true
      }
      PromptInputResult::Submit => {
        let input = self
          .prompt
          .take()
          .map(|prompt| prompt.buffer().input.trim().to_string())
          .unwrap_or_default();
        self.command_state.reset_prompt_state();
        self.dispatcher.clear();
        self.queue_filter = (!input.is_empty()).then_some(input);
        self.recompute_queue_filter();
        self.clamp_queue_selection();
        true
      }
      PromptInputResult::EditInEditor { .. } => {
        self.set_message("editing the filter in an editor is not supported");
        true
      }
    }
  }

  /// Mirrors pdf-tui's refresh_command_completion: no prompt / non-command
  /// prompt clears the completion; command prompts recompute it from the
  /// buffer before the cursor.
  fn refresh_prompt_completion(&mut self) {
    let Some(prompt) = self.prompt.as_ref() else {
      self.command_state.clear_completion();
      return;
    };
    if !prompt.is_command() {
      self.command_state.clear_completion();
      return;
    }
    let buffer = prompt.buffer();
    let completion = self.command_completion_for(&buffer.input, buffer.cursor);
    self
      .command_state
      .set_completion_preserving_selection(completion);
  }

  /// Command-name completion for the first token, per-command candidates for
  /// subcommands — same shape as pdf-tui's command_completion_for.
  fn command_completion_for(
    &self,
    input: &str,
    cursor: usize,
  ) -> Option<framework_tui::CommandCompletion> {
    let cursor = cursor.min(input.len());
    let before_cursor = input.get(..cursor)?;
    let normalized = before_cursor.trim_start_matches(':');
    let tokens = normalized.split_whitespace().collect::<Vec<_>>();
    let ends_with_space = normalized.chars().last().is_some_and(char::is_whitespace);
    let word_start = framework_tui::current_word_start(input, cursor);
    let prefix = if ends_with_space {
      ""
    } else {
      input.get(word_start..cursor).unwrap_or_default()
    };

    if tokens.is_empty() || (tokens.len() == 1 && !ends_with_space) {
      return Some(framework_tui::CommandCompletion::new(
        word_start,
        cursor,
        prefix,
        framework_tui::filter_completion_candidates(COMMANDS.iter().copied(), prefix),
        true,
        0,
      ));
    }

    match tokens[0] {
      "tab" => {
        if tokens.len() > 2 || (tokens.len() == 2 && ends_with_space) {
          return None;
        }
        let replace_start = if ends_with_space { cursor } else { word_start };
        let prefix = if ends_with_space { "" } else { prefix };
        Some(framework_tui::CommandCompletion::new(
          replace_start,
          cursor,
          prefix,
          framework_tui::filter_completion_candidates(
            self.tabs.iter().map(|tab| tab.name.as_str()),
            prefix,
          ),
          true,
          0,
        ))
      }
      _ => None,
    }
  }

  fn run_command_line(&mut self, input: &str) {
    let mut parts = input.split_whitespace();
    let Some(command) = parts.next() else {
      return;
    };
    let args: Vec<&str> = parts.collect();
    match command {
      "quit" | "q" => self.quit = true,
      "help" => self.show_help = true,
      "play" => self.mpdc(MpdCommand::PlayPauseToggle),
      "pause" => self.mpdc(MpdCommand::Pause(true)),
      "toggle" => self.mpdc(MpdCommand::PlayPauseToggle),
      "stop" => self.mpdc(MpdCommand::Stop),
      "next" => self.mpdc(MpdCommand::Next),
      "prev" => self.mpdc(MpdCommand::Previous),
      "volume" | "vol" => match args.first() {
        Some(value) => {
          if let Some(delta) = value.strip_prefix(['+', '-']) {
            let magnitude: i16 = delta.parse().unwrap_or(0);
            let signed = if value.starts_with('-') { -magnitude } else { magnitude };
            self.mpdc(MpdCommand::NudgeVolume(signed));
          } else if let Ok(volume) = value.parse::<u8>() {
            self.mpdc(MpdCommand::SetVolume(volume.min(100)));
          } else {
            self.set_message(format!("invalid volume: {value}"));
          }
        }
        None => {
          let volume = self.status.as_ref().map(|status| status.volume);
          self.set_message(format!("volume: {}%", volume.unwrap_or(0)));
        }
      },
      "repeat" => self.mpdc(MpdCommand::SetRepeat(self.toggle_flag("repeat"))),
      "random" => self.mpdc(MpdCommand::SetRandom(self.toggle_flag("random"))),
      "single" => self.mpdc(MpdCommand::SetSingle(self.toggle_single())),
      "consume" => self.mpdc(MpdCommand::SetConsume(self.toggle_flag("consume"))),
      "clear" => self.mpdc(MpdCommand::ClearQueue),
      "update" => self.mpdc(MpdCommand::Rescan),
      "tab" => self.command_tab(&args),
      "add" => self.command_add(&args),
      other => self.set_message(format!("unknown command: {other}")),
    }
  }

  fn command_tab(&mut self, args: &[&str]) {
    let Some(target) = args.first() else {
      let names: Vec<String> = self
        .tabs
        .iter()
        .enumerate()
        .map(|(index, tab)| format!("{}) {}", index + 1, tab.name))
        .collect();
      self.set_message(format!("tabs: {}", names.join("  ")));
      return;
    };
    if let Ok(index) = target.parse::<usize>()
      && self.goto_tab(index - 1)
    {
      return;
    }
    if let Some(index) = self.tabs.iter().position(|tab| tab.name == *target) {
      self.goto_tab(index);
      return;
    }
    self.set_message(format!("no such tab: {target}"));
  }

  fn command_add(&mut self, args: &[&str]) {
    let Some(target) = args.first() else {
      self.set_message("usage: add <path>");
      return;
    };
    let path = expand_home(target);
    let Some(music_dir) = self.music_dir.clone() else {
      self.set_message("music directory is not configured");
      return;
    };
    let resolved = if path.is_absolute() {
      path
    } else {
      music_dir.join(&path)
    };
    let canonical = match resolved.canonicalize() {
      Ok(canonical) => canonical,
      Err(_) => {
        self.set_message(format!("path not found: {}", resolved.display()));
        return;
      }
    };
    if canonical.is_dir() {
      let recursive = args.iter().any(|arg| *arg == "--recursive" || *arg == "-r");
      let files = match crate::library::collect_audio_files(&canonical, recursive) {
        Ok(files) => files,
        Err(error) => {
          self.set_message(format!("scan failed: {error}"));
          return;
        }
      };
      let count = files.len();
      for file in files {
        if let Ok(uri) = crate::library::path_to_uri(&music_dir, &file) {
          self.mpdc(MpdCommand::AddUri(uri));
        }
      }
      self.set_message(format!("queued {count} song(s)"));
    } else if let Ok(uri) = crate::library::path_to_uri(&music_dir, &canonical) {
      self.mpdc(MpdCommand::AddUri(uri));
      self.set_message(format!("queued {}", canonical.display()));
    } else {
      self.set_message("path is outside the music directory".to_string());
    }
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

  fn run_action(&mut self, action: &str) -> bool {
    match action {
      "quit" => {
        self.quit = true;
        true
      }
      "help" => {
        self.help_scroll = 0;
        self.show_help = true;
        true
      }
      "command" => {
        self.prompt = Some(Prompt::command(""));
        self.command_state.reset_prompt_state();
        self.refresh_prompt_completion();
        true
      }
      "tab_next" => self.cycle_tab(1),
      "tab_previous" => self.cycle_tab(-1),
      "back" => {
        if self.detail.is_some() {
          self.close_detail();
          true
        } else if self.main_pane() == PaneKind::Queue && self.queue_filter.is_some() {
          self.clear_queue_filter();
          true
        } else {
          self.goto_tab(0)
        }
      }
      "queue_filter" => {
        let current = self.queue_filter.clone().unwrap_or_default();
        self.prompt = Some(Prompt::text("/", current));
        true
      }
      "queue_up" => self.move_selection(-1),
      "queue_down" => self.move_selection(1),
      "queue_page_up" => self.move_selection_page(-1),
      "queue_page_down" => self.move_selection_page(1),
      "queue_top" => {
        if self.visible_len() > 0 {
          self.queue_state.select(Some(0));
        }
        true
      }
      "queue_end" => {
        let len = self.visible_len();
        if len > 0 {
          self.queue_state.select(Some(len - 1));
        }
        true
      }
      "toggle_follow_current" => {
        self.follow_current = !self.follow_current;
        self.set_message(if self.follow_current {
          "following current song"
        } else {
          "selection unlocked from current song"
        });
        true
      }
      "queue_play" => {
        if let Some(position) = self
          .queue_state
          .selected()
          .and_then(|row| self.filtered_position(row))
        {
          self.mpdc(MpdCommand::PlayPosition(position as u32));
        }
        true
      }
      "play_pause" => {
        self.mpdc(MpdCommand::PlayPauseToggle);
        true
      }
      "next" => {
        self.mpdc(MpdCommand::Next);
        true
      }
      "previous" => {
        self.mpdc(MpdCommand::Previous);
        true
      }
      "stop" => {
        self.mpdc(MpdCommand::Stop);
        true
      }
      "queue_delete" => {
        if let Some(position) = self
          .queue_state
          .selected()
          .and_then(|row| self.filtered_position(row))
          && position < self.queue.len()
        {
          let title = self.queue[position]
            .song
            .title()
            .map(str::to_string)
            .unwrap_or_else(|| self.queue[position].song.url.clone());
          self.mpdc(MpdCommand::DeleteAt(position));
          self.set_message(format!("deleted: {title}"));
        }
        true
      }
      "queue_clear" => {
        self.mpdc(MpdCommand::ClearQueue);
        self.set_message("queue cleared");
        true
      }
      "volume_up" => {
        self.mpdc(MpdCommand::NudgeVolume(5));
        true
      }
      "volume_down" => {
        self.mpdc(MpdCommand::NudgeVolume(-5));
        true
      }
      "volume_mute" => {
        let muted = self.status.as_ref().is_some_and(|status| status.volume == 0);
        self.mpdc(if muted { MpdCommand::SetVolume(50) } else { MpdCommand::SetVolume(0) });
        true
      }
      "seek_forward" => {
        self.mpdc(MpdCommand::NudgeSeek(5));
        true
      }
      "seek_back" => {
        self.mpdc(MpdCommand::NudgeSeek(-5));
        true
      }
      "seek_forward_long" => {
        self.mpdc(MpdCommand::NudgeSeek(30));
        true
      }
      "seek_back_long" => {
        self.mpdc(MpdCommand::NudgeSeek(-30));
        true
      }
      "toggle_repeat" => {
        self.mpdc(MpdCommand::SetRepeat(self.toggle_flag("repeat")));
        true
      }
      "toggle_random" => {
        self.mpdc(MpdCommand::SetRandom(self.toggle_flag("random")));
        true
      }
      "cycle_single" => {
        let next = match self.status.as_ref().map(|status| status.single) {
          Some(SingleMode::Disabled) => SingleMode::Enabled,
          Some(SingleMode::Enabled) => SingleMode::Oneshot,
          _ => SingleMode::Disabled,
        };
        self.mpdc(MpdCommand::SetSingle(next));
        true
      }
      "toggle_consume" => {
        self.mpdc(MpdCommand::SetConsume(self.toggle_flag("consume")));
        true
      }
      "scroll_up" => {
        self.scroll_metadata_by(-1);
        true
      }
      "scroll_down" => {
        self.scroll_metadata_by(1);
        true
      }
      "page_up" => {
        self.scroll_metadata_by(-10);
        true
      }
      "page_down" => {
        self.scroll_metadata_by(10);
        true
      }
      "edit_metadata" => {
        self.request_metadata_editor();
        true
      }
      "lyrics_up" => {
        self.lyrics_follow = false;
        let cursor = self.lyrics_cursor.unwrap_or_else(|| self.active_lyrics_index().unwrap_or(0));
        self.lyrics_cursor = Some(cursor.saturating_sub(1));
        true
      }
      "lyrics_down" => {
        self.lyrics_follow = false;
        let cursor = self.lyrics_cursor.unwrap_or_else(|| self.active_lyrics_index().unwrap_or(0));
        let limit = self.lyrics.as_ref().map(Lyrics::line_count).unwrap_or(1).saturating_sub(1);
        self.lyrics_cursor = Some((cursor + 1).min(limit));
        true
      }
      "lyrics_page_up" => {
        self.lyrics_follow = false;
        let cursor = self.lyrics_cursor.unwrap_or_else(|| self.active_lyrics_index().unwrap_or(0));
        self.lyrics_cursor = Some(cursor.saturating_sub(10));
        true
      }
      "lyrics_page_down" => {
        self.lyrics_follow = false;
        let cursor = self.lyrics_cursor.unwrap_or_else(|| self.active_lyrics_index().unwrap_or(0));
        let limit = self.lyrics.as_ref().map(Lyrics::line_count).unwrap_or(1).saturating_sub(1);
        self.lyrics_cursor = Some((cursor + 10).min(limit));
        true
      }
      "lyrics_jump" => {
        // Enter: seek to the highlighted (cursor or active) lyric line and
        // resume auto-follow.
        let index = self
          .lyrics_cursor
          .or_else(|| self.active_lyrics_index());
        let Some(index) = index else { return false };
        self.lyrics_seek_to(index)
      }
      "lyrics_follow" => {
        self.lyrics_follow = !self.lyrics_follow;
        if self.lyrics_follow {
          self.lyrics_cursor = None;
        }
        self.set_message(if self.lyrics_follow {
          "lyrics: following playback"
        } else {
          "lyrics: manual scroll"
        });
        true
      }
      "queue_detail" => self.open_detail(),
      "queue_goto_playing" => self.goto_playing(),
      "visualizer_reset" => {
        self.spectrum.fill(0);
        true
      }
      "rescan" => {
        self.mpdc(MpdCommand::Rescan);
        self.set_message("database rescan started");
        true
      }
      _ => false,
    }
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
        detail.metadata_scroll.saturating_sub(delta.unsigned_abs() as u16)
      } else {
        detail.metadata_scroll.saturating_add(delta as u16)
      };
    } else if delta < 0 {
      self.metadata_scroll = self.metadata_scroll.saturating_sub(delta.unsigned_abs() as u16);
    } else {
      self.metadata_scroll = self.metadata_scroll.saturating_add(delta as u16);
    }
  }

  fn request_metadata_editor(&mut self) {
    // In the detail view the editor targets the detailed song; everywhere
    // else it targets the playing song.
    let (url, path, entries) = if let Some(detail) = self.detail.as_ref() {
      (
        detail.url.clone(),
        detail.path.clone(),
        detail.metadata.clone(),
      )
    } else {
      let Some(url) = self.current_song_url() else {
        self.set_message("nothing is playing");
        return;
      };
      let Some(path) = self.current_song_path() else {
        self.set_message("music directory is not configured");
        return;
      };
      (url, path, self.metadata_entries.clone())
    };
    if !path.is_file() {
      self.set_message(format!("file not found: {}", path.display()));
      return;
    }
    let entries = match entries.or_else(|| metadata::read_metadata(&path).ok()) {
      Some(entries) => entries,
      None => {
        self.set_message("failed to read metadata".to_string());
        return;
      }
    };
    let draft = metadata::metadata_draft(&path, &entries);
    self.editor_request = Some(EditorRequest::Metadata {
      song_url: url,
      path,
      original: entries,
      draft,
    });
  }

  pub fn finish_metadata_editor(&mut self, request: EditorRequest, edited: Option<String>) {
    let EditorRequest::Metadata {
      song_url,
      path,
      original,
      ..
    } = request;
    let Some(edited) = edited else {
      self.set_message("metadata edit cancelled");
      return;
    };
    let changes = match metadata::metadata_changes(&original, &edited) {
      Ok(changes) => changes,
      Err(error) => {
        self.set_message(format!("metadata edit failed: {error}"));
        return;
      }
    };
    if changes.is_empty() {
      self.set_message("metadata unchanged");
      return;
    }
    self.set_message(format!("writing {} tag change(s)...", changes.len()));
    let tx = self.events.clone();
    tokio::task::spawn_blocking(move || {
      let result = metadata::write_metadata(&path, &changes);
      let _ = tx.send(AsyncEvent::MetadataWrite(MetadataWriteOutcome {
        song_url,
        changed_tags: changes.len(),
        result: result.map(|_| ()),
      }));
    });
  }

  // --- draw-time helpers ---------------------------------------------------

  pub fn bindings(&self) -> &KeyBindings {
    self
      .view_bindings
      .get(self.main_pane().index())
      .expect("bindings for main pane")
  }
}

/// `mm:ss` for footer/seek messages.
fn format_time(secs: f64) -> String {
  let total = secs.max(0.0) as u64;
  format!("{}:{:02}", total / 60, total % 60)
}

fn build_bindings(keymap: &KeymapConfig) -> Vec<KeyBindings> {
  vec![
    keymap.queue_bindings(),
    keymap.cover_bindings(),
    keymap.lyrics_bindings(),
    keymap.metadata_bindings(),
    keymap.visualizer_bindings(),
  ]
}

fn build_input_bindings(keymap: &KeymapConfig) -> KeyBindings {
  keymap.input_only_bindings()
}
