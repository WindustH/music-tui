//! Application state and input handling.

pub(crate) use std::{
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
  layout::{PaneKind, TabLayout, parse_tabs},
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
pub(crate) mod commands;
mod detail;
pub(crate) mod mouse;

pub use detail::DetailView;

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

  pub(crate)   fn active_lyrics_index(&self) -> Option<usize> {
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
    } else if delta < 0 {
      self.metadata_scroll = self.metadata_scroll.saturating_sub(delta.unsigned_abs() as usize);
    } else {
      self.metadata_scroll = self.metadata_scroll.saturating_add(delta as usize);
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
pub(crate) fn format_time(secs: f64) -> String {
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
