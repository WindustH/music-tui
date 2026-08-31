//! music-tui: a terminal music player backed by MPD.

mod app;
mod cli;
mod config;
mod cover;
mod event;
mod keymap;
mod layout;
mod library;
mod library_db;
mod logging;
mod lyrics;
mod metadata;
mod mpd;
mod open;
mod playlist;
mod render;
mod state;
mod strip;
mod terminal;
mod theme;
mod ui;
mod visualizer;

use std::{
  sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
  },
  thread,
  time::Duration,
};

use anyhow::Result;
use clap::Parser;
use framework_tui::editor::edit_text_in_editor;
use img_tui::{NativeImageConfig, RenderMode, capability, native_image};
use tokio::{sync::mpsc, time::sleep};
use tracing::{debug, info};

use crate::{
  app::{App, EditorRequest},
  config::Settings,
  event::{AsyncEvent, LibraryEvent},
  mpd::InterruptSession,
  render::CoverRenderStore,
  terminal::Tui,
};

#[tokio::main]
async fn main() -> Result<()> {
  let cli = cli::Cli::parse();
  let settings = config::load_or_create().await?;
  logging::init(&settings.cache_dir)?;

  let (initial_notice, interrupt) = match cli.command {
    Some(cli::Command::Open(args)) => match open::run_open(&args, &settings).await {
      Ok(outcome) => (Some(outcome.notice), outcome.interrupt),
      Err(error) => {
        eprintln!("music-tui open: {error:#}");
        std::process::exit(1);
      }
    },
    None => (None, None),
  };

  run_tui(settings, initial_notice, interrupt).await
}

async fn run_tui(
  settings: Settings,
  initial_notice: Option<String>,
  interrupt: Option<InterruptSession>,
) -> Result<()> {
  let terminal_capability = capability::detect();
  info!(
    capability = ?terminal_capability,
    "detected terminal capability"
  );

  let render_config = settings.config.render.clone();
  let render_modes = capability::render_modes_override_from_env()
    .or_else(|| {
      render_config.auto_detect.then(|| {
        let zellij_sixel = if render_config.zellij_sixel { "on" } else { "" };
        terminal_capability.preferred_render_modes(zellij_sixel)
      })
    })
    .unwrap_or_else(|| vec![RenderMode::Symbols, RenderMode::Ascii]);
  info!(
    modes = ?render_modes.iter().map(|mode| mode.label()).collect::<Vec<_>>(),
    "effective render modes"
  );

  let native_config = NativeImageConfig {
    cell_pixels: terminal_capability.cell_pixels,
    passthrough: terminal_capability.passthrough().map(str::to_string),
    kitty_unicode_placeholders: terminal_capability.kitty_unicode_placeholders(),
  };
  let protocol_reset = render_modes
    .contains(&RenderMode::Kitty)
    .then(|| {
      native_image::erase_sequence(
        RenderMode::Kitty,
        native_config.passthrough.as_deref(),
        None,
      )
    })
    .flatten();

  let (tx, mut rx) = mpsc::unbounded_channel::<AsyncEvent>();
  let mpd = mpd::spawn_mpd_worker(settings.config.mpd.clone(), tx.clone());
  mpd.set_queue_dedup(settings.config.behavior.queue_dedup);
  let visualizer = visualizer::spawn_visualizer(settings.config.visualizer.clone(), tx.clone());
  let band_renderer = visualizer
    .as_ref()
    .map(|_| visualizer::spawn_band_renderer(tx.clone()));
  let library_scan_tx =
    spawn_library_scanner(&settings.config.library, &settings.state_dir, tx.clone());

  let input_enabled = Arc::new(AtomicBool::new(true));
  let input_generation = Arc::new(AtomicU64::new(0));
  spawn_input_thread(tx.clone(), input_enabled.clone(), input_generation.clone());
  spawn_tick_task(tx.clone(), settings.config.behavior.clone());

  let mut renderer = CoverRenderStore::new(render_config, native_config, render_modes);
  let mut tui = Tui::new(protocol_reset)?;
  let mut app = App::new(settings, mpd, tx.clone(), initial_notice, interrupt);
  app.visualizer = visualizer.clone();
  app.visualizer_renderer = band_renderer;
  app.library_scan_tx = library_scan_tx;
  app.restore_state(state::PersistedState::load(&app.settings.state_dir));
  let mut saved_state = app.snapshot_state();
  let mut needs_draw = true;

  loop {
    if needs_draw {
      tui.draw(|frame| ui::draw(frame, &mut app, &mut renderer, &tx))?;
      needs_draw = false;
      if app.should_quit() {
        break;
      }
    }

    if let Some(request) = app.take_editor_request() {
      input_enabled.store(false, Ordering::SeqCst);
      input_generation.fetch_add(1, Ordering::SeqCst);
      tui.suspend()?;
      let EditorRequest::Metadata { draft, .. } = &request;
      let result = edit_text_in_editor(draft, &app.settings.cache_dir);
      let resume_result = tui.resume();
      if resume_result.is_ok() {
        discard_pending_terminal_events();
      }
      input_generation.fetch_add(1, Ordering::SeqCst);
      input_enabled.store(true, Ordering::SeqCst);
      app.finish_metadata_editor(request, result.ok());
      resume_result?;
      needs_draw = true;
      continue;
    }

    let Some(message) = rx.recv().await else {
      break;
    };
    needs_draw |= handle_async_event(message, &input_generation, &mut app, &mut renderer);
    while let Ok(message) = rx.try_recv() {
      needs_draw |= handle_async_event(message, &input_generation, &mut app, &mut renderer);
    }

    // Persist UI state whenever it changed (tab, follow, selection) —
    // crash-safe via atomic rename, cheap enough to run per event batch.
    let current_state = app.snapshot_state();
    if current_state != saved_state {
      current_state.save(&app.settings.state_dir);
      saved_state = current_state;
    }
  }
  if let Some(visualizer) = visualizer {
    visualizer.stop();
  }
  tui.restore()?;
  let final_state = app.snapshot_state();
  if final_state != saved_state {
    final_state.save(&app.settings.state_dir);
  }
  Ok(())
}

fn handle_async_event(
  message: AsyncEvent,
  input_generation: &AtomicU64,
  app: &mut App,
  renderer: &mut CoverRenderStore,
) -> bool {
  match message {
    AsyncEvent::Input { event, generation } => {
      let current_generation = input_generation.load(Ordering::SeqCst);
      if generation == current_generation {
        app.handle_input(event)
      } else {
        debug!(
          ?event,
          generation, current_generation, "input event ignored"
        );
        false
      }
    }
    AsyncEvent::Mpd(event) => app.handle_mpd_event(event),
    AsyncEvent::Tick => app.handle_tick(),
    AsyncEvent::Lyrics(outcome) => app.handle_lyrics_outcome(outcome),
    AsyncEvent::Metadata(outcome) => app.handle_metadata_outcome(outcome),
    AsyncEvent::MetadataWrite(outcome) => app.handle_metadata_write_outcome(outcome),
    AsyncEvent::Cover(outcome) => app.handle_cover_outcome(outcome),
    AsyncEvent::Render(outcome) => renderer.finish(outcome),
    #[cfg(unix)]
    AsyncEvent::Spectrum(bars) => app.handle_spectrum(bars),
    AsyncEvent::VisualizerFrame(lines) => app.handle_visualizer_frame(lines),
    AsyncEvent::Library(event) => match event {
      LibraryEvent::Scanning { scanned, changed } => {
        app.library_scanning = Some((scanned, changed));
        true
      }
      LibraryEvent::Loaded(tracks) => {
        app.library_loaded(tracks);
        app.set_message("library ready");
        true
      }
      LibraryEvent::Error(error) => {
        app.library_scanning = None;
        app.set_message(format!("library scan failed: {error}"));
        true
      }
    },
  }
}

fn spawn_library_scanner(
  library: &config::LibraryConfig,
  state_dir: &std::path::Path,
  tx: mpsc::UnboundedSender<AsyncEvent>,
) -> Option<std::sync::mpsc::Sender<()>> {
  if library.paths.is_empty() {
    return None;
  }
  let library = library.clone();
  let db_path = state_dir.join("library.db");
  let (scan_tx, scan_rx) = std::sync::mpsc::channel::<()>();
  thread::spawn(move || {
    let send = |event: LibraryEvent| {
      let _ = tx.send(AsyncEvent::Library(event));
    };
    loop {
      let result = (|| -> anyhow::Result<Vec<crate::library_db::LibraryTrack>> {
        let mut connection = crate::library_db::open_db(&db_path)?;
        crate::library_db::sync_roots(&connection, &library)?;
        {
          let mut progress = |scanned: usize, changed: usize| {
            send(LibraryEvent::Scanning { scanned, changed });
          };
          crate::library_db::scan_roots(&mut connection, &library, &mut progress)?;
        }
        crate::library_db::all_tracks(&connection)
      })();
      match result {
        Ok(tracks) => send(LibraryEvent::Loaded(tracks)),
        Err(error) => send(LibraryEvent::Error(error.to_string())),
      }
      // Wait for a rescan request (`u` in the library pane); the sender
      // lives in the App, so the loop exits when the UI shuts down.
      if scan_rx.recv().is_err() {
        break;
      }
    }
  });
  Some(scan_tx)
}

fn spawn_input_thread(
  tx: mpsc::UnboundedSender<AsyncEvent>,
  enabled: Arc<AtomicBool>,
  generation: Arc<AtomicU64>,
) {
  thread::spawn(move || {
    loop {
      if !enabled.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(10));
        continue;
      }
      match crossterm::event::read() {
        Ok(event) => {
          let generation = generation.load(Ordering::SeqCst);
          if tx.send(AsyncEvent::Input { event, generation }).is_err() {
            break;
          }
        }
        Err(_) => thread::sleep(Duration::from_millis(10)),
      }
    }
  });
}

/// Regular UI tick: expires footer messages and keeps the clock moving even
/// when mpd is idle. Playback position updates arrive via mpd snapshots.
fn spawn_tick_task(tx: mpsc::UnboundedSender<AsyncEvent>, behavior: config::BehaviorConfig) {
  tokio::spawn(async move {
    let period = Duration::from_millis(behavior.tick_ms.clamp(100, 10_000));
    loop {
      sleep(period).await;
      if tx.send(AsyncEvent::Tick).is_err() {
        break;
      }
    }
  });
}

fn discard_pending_terminal_events() {
  while crossterm::event::poll(Duration::from_millis(0)).unwrap_or(false) {
    if crossterm::event::read().is_err() {
      break;
    }
  }
}
