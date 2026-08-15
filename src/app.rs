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
use mpd_client::responses::{PlayState, SongInQueue, Status};
use ratatui::{
  layout::Rect,
  widgets::{ListState, ScrollbarState},
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
  lyrics,
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

  pub metadata_entries: Option<Vec<metadata::MetadataEntry>>,
  pub metadata_url: String,
  pub metadata_error: Option<String>,
  pub metadata_scroll: u16,
  pub editor_request: Option<EditorRequest>,

  pub cover_path: Option<(String, PathBuf)>,
  pub cover_error: Option<String>,

  pub spectrum: Vec<u8>,

  /// Screen area of the bottom progress band, recorded at draw time for
  /// mouse hit-testing (click / drag to seek).
  pub progress_band_area: Option<Rect>,
  band_scrubbing: bool,

  dispatcher: KeyDispatcher,
  /// Bindings per pane kind, indexed by `PaneKind::index`.
  view_bindings: Vec<KeyBindings>,
  input_bindings: KeyBindings,
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
      metadata_entries: None,
      metadata_url: String::new(),
      metadata_error: None,
      metadata_scroll: 0,
      editor_request: None,
      cover_path: None,
      cover_error: None,
      spectrum: Vec::new(),
      progress_band_area: None,
      band_scrubbing: false,
      dispatcher: KeyDispatcher::default(),
      view_bindings,
      input_bindings,
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
        let now_playing = status.state == PlayState::Playing;
        self.status = Some(status);
        self.queue = queue;
        self.clamp_queue_selection();
        if song_changed {
          self.on_song_changed();
        } else if self.follow_current && now_playing {
          self.follow_playing_position();
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
    if self.queue.is_empty() {
      self.queue_state.select(None);
      return;
    }
    let len = self.queue.len();
    let current = self.queue_state.selected().unwrap_or(0).min(len - 1);
    self.queue_state.select(Some(current));
  }

  fn follow_playing_position(&mut self) {
    if let Some(status) = &self.status
      && let Some((position, _)) = status.current_song
      && position.0 < self.queue.len()
    {
      self.queue_state.select(Some(position.0));
    }
  }

  fn on_song_changed(&mut self) {
    self.lyrics = None;
    self.lyrics_error = None;
    self.lyrics_scroll = 0;
    self.metadata_entries = None;
    self.metadata_error = None;
    self.metadata_scroll = 0;
    self.cover_path = None;
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
    let tx = self.events.clone();
    tokio::task::spawn_blocking(move || {
      let result = metadata::read_metadata(&path);
      let _ = tx.send(AsyncEvent::Metadata(MetadataOutcome { song_url: url, result }));
    });
  }

  fn request_cover(&mut self, url: String, path: PathBuf) {
    let cache_dir = self.settings.cache_dir.join("covers");
    let tx = self.events.clone();
    tokio::task::spawn_blocking(move || {
      let result = cover::find_cover(&path, &cache_dir);
      let _ = tx.send(AsyncEvent::Cover(CoverOutcome { song_url: url, result }));
    });
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
    if outcome.song_url != self.metadata_url {
      return false;
    }
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
    true
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
    match outcome.result {
      Ok(path) => {
        self.cover_path = Some((outcome.song_url, path));
        self.cover_error = None;
      }
      Err(error) => {
        if outcome.song_url == self.current_song_url().unwrap_or_default() {
          self.cover_path = None;
          self.cover_error = Some(error);
        }
      }
    }
    self.tab_contains(PaneKind::Cover)
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
      self.show_help = false;
      return true;
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
      if matches!(mouse.kind, MouseEventKind::Down(_)) {
        self.show_help = false;
        return true;
      }
      return false;
    }
    match mouse.kind {
      MouseEventKind::Down(MouseButton::Left) => {
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
      MouseEventKind::ScrollUp if self.mouse_on_band(mouse) => {
        self.mpdc(MpdCommand::NudgeSeek(5));
        true
      }
      MouseEventKind::ScrollDown if self.mouse_on_band(mouse) => {
        self.mpdc(MpdCommand::NudgeSeek(-5));
        true
      }
      _ => false,
    }
  }

  fn mouse_on_band(&self, mouse: MouseEvent) -> bool {
    self.progress_band_area.is_some_and(|area| {
      mouse.row == area.y && mouse.column >= area.x && mouse.column < area.x + area.width
    })
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
    match result {
      PromptInputResult::Unhandled => false,
      PromptInputResult::Changed => {
        self.refresh_prompt_completion();
        true
      }
      PromptInputResult::Cancel => {
        self.prompt = None;
        true
      }
      PromptInputResult::Submit => {
        let Some(prompt) = self.prompt.as_ref() else {
          return false;
        };
        let input = prompt.buffer().input.trim().to_string();
        self.prompt = None;
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
      PromptInputResult::UnknownAction(_) => false,
    }
  }

  fn refresh_prompt_completion(&mut self) {
    let Some(prompt) = self.prompt.as_ref() else {
      return;
    };
    if !prompt.is_command() {
      return;
    }
    let input = &prompt.buffer().input;
    let cursor = prompt.buffer().cursor;
    let word_start = framework_tui::current_word_start(input, cursor);
    let prefix = input[word_start..cursor].to_string();
    let candidates = framework_tui::filter_completion_candidates(COMMANDS.iter().copied(), &prefix);
    self
      .command_state
      .set_completion_preserving_selection(Some(framework_tui::CommandCompletion {
        replace_start: word_start,
        replace_end: cursor,
        prefix,
        candidates,
        append_space: true,
        selected: 0,
      }));
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
      "back" => self.goto_tab(0),
      "queue_up" => self.move_selection(-1),
      "queue_down" => self.move_selection(1),
      "queue_page_up" => self.move_selection_page(-1),
      "queue_page_down" => self.move_selection_page(1),
      "queue_top" => {
        if !self.queue.is_empty() {
          self.queue_state.select(Some(0));
        }
        true
      }
      "queue_end" => {
        if !self.queue.is_empty() {
          self.queue_state.select(Some(self.queue.len() - 1));
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
        if let Some(position) = self.queue_state.selected() {
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
        if let Some(position) = self.queue_state.selected()
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
        self.metadata_scroll = self.metadata_scroll.saturating_sub(1);
        true
      }
      "scroll_down" => {
        self.metadata_scroll = self.metadata_scroll.saturating_add(1);
        true
      }
      "page_up" => {
        self.metadata_scroll = self.metadata_scroll.saturating_sub(10);
        true
      }
      "page_down" => {
        self.metadata_scroll = self.metadata_scroll.saturating_add(10);
        true
      }
      "edit_metadata" => {
        self.request_metadata_editor();
        true
      }
      "lyrics_up" => {
        self.lyrics_follow = false;
        self.lyrics_scroll = self.lyrics_scroll.saturating_sub(1);
        true
      }
      "lyrics_down" => {
        self.lyrics_follow = false;
        self.lyrics_scroll = self.lyrics_scroll.saturating_add(1);
        true
      }
      "lyrics_page_up" => {
        self.lyrics_follow = false;
        self.lyrics_scroll = self.lyrics_scroll.saturating_sub(10);
        true
      }
      "lyrics_page_down" => {
        self.lyrics_follow = false;
        self.lyrics_scroll = self.lyrics_scroll.saturating_add(10);
        true
      }
      "lyrics_follow" => {
        self.lyrics_follow = !self.lyrics_follow;
        self.set_message(if self.lyrics_follow {
          "lyrics follow playback"
        } else {
          "lyrics scroll unlocked"
        });
        true
      }
      "visualizer_reset" => {
        self.spectrum.fill(0);
        true
      }
      "rescan" => {
        self.mpdc(MpdCommand::Rescan);
        self.set_message("database rescan started");
        true
      }
      tab_goto => {
        if let Some(number) = tab_goto.strip_prefix("tab_goto_")
          && let Ok(index) = number.parse::<usize>()
        {
          return self.goto_tab(index - 1);
        }
        false
      }
    }
  }

  fn move_selection(&mut self, delta: i32) -> bool {
    if self.queue.is_empty() {
      return false;
    }
    let len = self.queue.len() as i32;
    let current = self.queue_state.selected().unwrap_or(0) as i32;
    let next = (current + delta).clamp(0, len - 1) as usize;
    self.queue_state.select(Some(next));
    true
  }

  fn move_selection_page(&mut self, direction: i32) -> bool {
    self.move_selection(direction * 10)
  }

  fn request_metadata_editor(&mut self) {
    let Some(url) = self.current_song_url() else {
      self.set_message("nothing is playing");
      return;
    };
    let Some(path) = self.current_song_path() else {
      self.set_message("music directory is not configured");
      return;
    };
    if !path.is_file() {
      self.set_message(format!("file not found: {}", path.display()));
      return;
    }
    let entries = match self
      .metadata_entries
      .clone()
      .or_else(|| metadata::read_metadata(&path).ok())
    {
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

  pub fn dispatcher_hints(&self) -> &[framework_tui::KeyHint] {
    self.dispatcher.hints()
  }

  pub fn queue_scroll_state(&self) -> (ListState, ScrollbarState) {
    let list = self.queue_state.clone();
    let scrollbar = ScrollbarState::new(self.queue.len().max(1)).position(
      self.queue_state.selected().unwrap_or(0),
    );
    (list, scrollbar)
  }
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
