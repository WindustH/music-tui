//! music-tui: a terminal music player backed by MPD.

mod app;
mod cli;
mod config;
mod cover;
mod event;
mod fsutil;
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
mod sanitize;
mod state;
mod strip;
mod terminal;
mod theme;
mod ui;
mod visualizer;

use std::time::Instant;

use anyhow::Result;
use clap::Parser;
use framework_tui::editor::edit_text_in_editor;
use tokio::{sync::mpsc, time::sleep_until};
use tracing::debug;

use crate::{
  app::{App, EditorRequest},
  config::Settings,
  event::AsyncEvent,
  mpd::InterruptSession,
  render::CoverRenderStore,
  state::StateSaver,
  terminal::{InputReader, Tui},
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

  let notice = if settings.warnings.is_empty() {
    initial_notice
  } else {
    let mut joined = settings.warnings.join("\n");
    if let Some(notice) = initial_notice {
      joined.push('\n');
      joined.push_str(&notice);
    }
    Some(joined)
  };

  run_tui(settings, notice, interrupt).await
}

async fn run_tui(
  settings: Settings,
  initial_notice: Option<String>,
  interrupt: Option<InterruptSession>,
) -> Result<()> {
  let render_setup = render::setup(&settings.config.render);

  let (tx, mut rx) = mpsc::unbounded_channel::<AsyncEvent>();
  let mpd = mpd::spawn_mpd_worker(
    settings.config.mpd.clone(),
    settings.config.behavior.clone(),
    tx.clone(),
  );
  let visualizer = visualizer::spawn_visualizer(settings.config.visualizer.clone(), tx.clone());
  let band_renderer = visualizer
    .as_ref()
    .map(|_| visualizer::spawn_band_renderer(tx.clone()));
  let library_scan_tx =
    library_db::spawn_scanner(&settings.config.library, &settings.state_dir, tx.clone());
  let input = InputReader::spawn(tx.clone());

  let mut renderer = CoverRenderStore::new(
    settings.config.render.clone(),
    render_setup.native_config,
    render_setup.modes,
  );
  let mut tui = Tui::new(render_setup.protocol_reset)?;
  install_panic_hook();
  let mut app = App::new(settings, mpd, tx.clone(), initial_notice, interrupt);
  app.attach_workers(visualizer.clone(), band_renderer, library_scan_tx);
  app.restore_state(state::PersistedState::load(&app.settings.state_dir));
  let mut state_saver = StateSaver::new(&app.settings.state_dir, app.snapshot_state());
  let mut needs_draw = true;

  loop {
    if needs_draw {
      if let Some(visualizer) = &visualizer {
        // Analysis only runs while its pane is on screen.
        visualizer.set_active(app.pane_visible(layout::PaneKind::Visualizer));
      }
      tui.draw(|frame| ui::draw(frame, &mut app, &mut renderer, &tx))?;
      needs_draw = false;
      if app.should_quit() {
        break;
      }
    }

    if let Some(request) = app.take_editor_request() {
      input.pause();
      tui.suspend()?;
      let EditorRequest::Metadata { draft, .. } = &request;
      let result = edit_text_in_editor(draft, &app.settings.cache_dir);
      let resume_result = tui.resume();
      input.resume();
      app.finish_metadata_editor(request, result.ok());
      resume_result?;
      needs_draw = true;
      continue;
    }

    // Sleep until the next event, or until a footer message expires or a
    // state save falls due — no periodic wakeups while nothing happens.
    let deadline = [app.message_deadline(), state_saver.deadline()]
      .into_iter()
      .flatten()
      .min();
    let message = match deadline {
      Some(deadline) => tokio::select! {
        message = rx.recv() => message,
        _ = sleep_until(deadline.into()) => {
          let now = Instant::now();
          needs_draw |= app.expire_message(now);
          state_saver.flush(now, false);
          continue;
        }
      },
      None => rx.recv().await,
    };
    let Some(message) = message else {
      break;
    };
    needs_draw |= handle_async_event(message, &input, &mut app, &mut renderer);
    while let Ok(message) = rx.try_recv() {
      needs_draw |= handle_async_event(message, &input, &mut app, &mut renderer);
    }
    state_saver.update(app.snapshot_state(), Instant::now());
  }
  if let Some(visualizer) = visualizer {
    visualizer.stop();
  }
  tui.restore()?;
  state_saver.update(app.snapshot_state(), Instant::now());
  state_saver.flush(Instant::now(), true);
  Ok(())
}

fn handle_async_event(
  message: AsyncEvent,
  input: &InputReader,
  app: &mut App,
  renderer: &mut CoverRenderStore,
) -> bool {
  match message {
    AsyncEvent::Input { event, generation } => {
      let current = input.generation();
      if generation == current {
        app.handle_input(event)
      } else {
        debug!(?event, generation, current, "stale input event ignored");
        false
      }
    }
    AsyncEvent::Mpd(event) => app.handle_mpd_event(event),
    AsyncEvent::Lyrics(outcome) => app.handle_lyrics_outcome(outcome),
    AsyncEvent::Metadata(outcome) => app.handle_metadata_outcome(outcome),
    AsyncEvent::MetadataWrite(outcome) => app.handle_metadata_write_outcome(outcome),
    AsyncEvent::Cover(outcome) => app.handle_cover_outcome(outcome),
    AsyncEvent::Render(outcome) => renderer.finish(outcome),
    #[cfg(unix)]
    AsyncEvent::Spectrum(bars) => app.handle_spectrum(bars),
    AsyncEvent::VisualizerFrame(lines) => app.handle_visualizer_frame(lines),
    AsyncEvent::Library(event) => app.handle_library_event(event),
  }
}

/// Panics on the UI thread restore the terminal before the message is
/// printed (otherwise it lands on the alternate screen and vanishes).
/// Panics on worker threads are logged instead: writing them to stderr
/// would scribble over the running TUI.
fn install_panic_hook() {
  let default_hook = std::panic::take_hook();
  std::panic::set_hook(Box::new(move |info| {
    if std::thread::current().name() == Some("main") {
      terminal::emergency_restore();
      default_hook(info);
    } else {
      tracing::error!("background thread panicked: {info}");
    }
  }));
}
