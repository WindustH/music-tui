//! Terminal ownership: raw mode / alternate screen setup and teardown,
//! suspend/resume around external editors, and the input reader thread.

use std::{
  io::{self, Stderr},
  sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
  },
  thread,
  time::{Duration, Instant},
};

use anyhow::Result;
use crossterm::{
  event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
  execute,
  terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use img_tui::{ProtocolFrameOutput, ProtocolFrameRenderer, reset_protocol_images};
use ratatui::{Frame, Terminal, prelude::CrosstermBackend};
use tokio::sync::mpsc;

use crate::event::AsyncEvent;

pub type FrameOutput = ProtocolFrameOutput;

pub struct Tui {
  terminal: Terminal<CrosstermBackend<Stderr>>,
  protocol_renderer: ProtocolFrameRenderer,
  protocol_reset: Option<String>,
  suspended: bool,
  restored: bool,
}

impl Tui {
  /// Create the terminal wrapper, restoring the original terminal modes if
  /// any initialization step fails. Side effects are applied in order and
  /// unwound in reverse on error, so a failed `new` never leaves raw mode,
  /// the alternate screen, or mouse/paste capture engaged.
  pub fn new(protocol_reset: Option<String>) -> Result<Self> {
    fn reset_modes(stderr: &mut Stderr) {
      let _ = disable_raw_mode();
      let _ = execute!(
        stderr,
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
      );
    }
    enable_raw_mode()?;
    let mut stderr = io::stderr();
    if let Err(error) = execute!(
      stderr,
      EnterAlternateScreen,
      EnableMouseCapture,
      EnableBracketedPaste
    ) {
      reset_modes(&mut stderr);
      return Err(error.into());
    }
    let backend = CrosstermBackend::new(stderr);
    let mut terminal = match Terminal::new(backend) {
      Ok(terminal) => terminal,
      Err(error) => {
        reset_modes(&mut io::stderr());
        return Err(error.into());
      }
    };
    if let Err(error) = reset_protocol_images(terminal.backend_mut(), protocol_reset.as_deref()) {
      reset_modes(&mut io::stderr());
      return Err(error);
    }
    Ok(Self {
      terminal,
      protocol_renderer: ProtocolFrameRenderer::default(),
      protocol_reset,
      suspended: false,
      restored: false,
    })
  }

  pub fn draw<F>(&mut self, render: F) -> Result<()>
  where
    F: FnOnce(&mut Frame) -> FrameOutput,
  {
    self.protocol_renderer.draw(&mut self.terminal, render)
  }

  pub fn restore(&mut self) -> Result<()> {
    if self.restored {
      return Ok(());
    }
    let backend = self.terminal.backend_mut();
    self
      .protocol_renderer
      .clear_and_reset(backend, self.protocol_reset.as_deref())?;
    disable_raw_mode()?;
    self.terminal.show_cursor()?;
    if !self.suspended {
      execute!(
        self.terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
      )?;
    }
    self.suspended = true;
    self.restored = true;
    Ok(())
  }

  pub fn suspend(&mut self) -> Result<()> {
    if self.suspended {
      return Ok(());
    }
    let backend = self.terminal.backend_mut();
    self
      .protocol_renderer
      .clear_and_reset(backend, self.protocol_reset.as_deref())?;
    disable_raw_mode()?;
    self.terminal.show_cursor()?;
    execute!(
      self.terminal.backend_mut(),
      LeaveAlternateScreen,
      DisableMouseCapture,
      DisableBracketedPaste
    )?;
    self.suspended = true;
    Ok(())
  }

  pub fn resume(&mut self) -> Result<()> {
    if !self.suspended {
      return Ok(());
    }
    enable_raw_mode()?;
    execute!(
      self.terminal.backend_mut(),
      EnterAlternateScreen,
      EnableMouseCapture,
      EnableBracketedPaste
    )?;
    self.terminal.clear()?;
    reset_protocol_images(self.terminal.backend_mut(), self.protocol_reset.as_deref())?;
    self.suspended = false;
    Ok(())
  }
}

impl Drop for Tui {
  fn drop(&mut self) {
    let _ = self.restore();
  }
}

/// Best-effort terminal reset for the panic hook: the `Tui` may be
/// mid-draw and cannot be reached, so undo the modes directly. `Tui`'s own
/// restore still runs while the panic unwinds; repeating it is harmless.
pub fn emergency_restore() {
  let _ = disable_raw_mode();
  let _ = execute!(
    io::stderr(),
    LeaveAlternateScreen,
    DisableMouseCapture,
    DisableBracketedPaste,
    crossterm::cursor::Show
  );
}

/// How long one `poll` may block before the reader re-checks its gate.
const INPUT_POLL: Duration = Duration::from_millis(50);

/// Terminal input reader thread. It forwards events tagged with a
/// generation number; `pause`/`resume` hand the terminal to a child
/// process (the metadata editor) and back without the reader competing
/// for — and stealing — the child's keystrokes.
pub struct InputReader {
  shared: Arc<InputShared>,
}

struct InputShared {
  enabled: AtomicBool,
  /// Set by the reader once it has observed `enabled == false` and will
  /// not touch the terminal until re-enabled.
  parked: AtomicBool,
  generation: AtomicU64,
}

impl InputReader {
  pub fn spawn(tx: mpsc::UnboundedSender<AsyncEvent>) -> Self {
    let shared = Arc::new(InputShared {
      enabled: AtomicBool::new(true),
      parked: AtomicBool::new(false),
      generation: AtomicU64::new(0),
    });
    let reader = shared.clone();
    thread::Builder::new()
      .name("music-tui-input".to_string())
      .spawn(move || read_loop(&reader, &tx))
      .expect("failed to spawn the input thread");
    Self { shared }
  }

  /// Generation of events that are still current (see `pause`).
  pub fn generation(&self) -> u64 {
    self.shared.generation.load(Ordering::SeqCst)
  }

  /// Stop reading input and wait (briefly) until the reader is parked, so
  /// it no longer polls the terminal. Events already queued become stale.
  pub fn pause(&self) {
    self.shared.enabled.store(false, Ordering::SeqCst);
    self.shared.generation.fetch_add(1, Ordering::SeqCst);
    let deadline = Instant::now() + INPUT_POLL * 4;
    while !self.shared.parked.load(Ordering::SeqCst) && Instant::now() < deadline {
      thread::sleep(Duration::from_millis(1));
    }
  }

  /// Drop input that arrived while paused and start reading again.
  pub fn resume(&self) {
    while crossterm::event::poll(Duration::ZERO).unwrap_or(false) {
      if crossterm::event::read().is_err() {
        break;
      }
    }
    self.shared.generation.fetch_add(1, Ordering::SeqCst);
    self.shared.enabled.store(true, Ordering::SeqCst);
  }
}

fn read_loop(shared: &InputShared, tx: &mpsc::UnboundedSender<AsyncEvent>) {
  let mut error_backoff = Duration::from_millis(10);
  loop {
    // Clear `parked` before checking the gate: `pause` treats a set flag
    // as proof that the gate check below has seen `enabled == false`.
    shared.parked.store(false, Ordering::SeqCst);
    if !shared.enabled.load(Ordering::SeqCst) {
      shared.parked.store(true, Ordering::SeqCst);
      thread::sleep(Duration::from_millis(10));
      continue;
    }
    // Tag with the generation current *before* waiting: an event read
    // after a concurrent `pause` then counts as stale.
    let generation = shared.generation.load(Ordering::SeqCst);
    let event = match crossterm::event::poll(INPUT_POLL) {
      Ok(false) => continue,
      Ok(true) => crossterm::event::read(),
      Err(error) => Err(error),
    };
    match event {
      Ok(event) => {
        error_backoff = Duration::from_millis(10);
        if tx.send(AsyncEvent::Input { event, generation }).is_err() {
          return;
        }
      }
      Err(_) => {
        // A vanished terminal fails every read; back off instead of
        // spinning.
        thread::sleep(error_backoff);
        error_backoff = (error_backoff * 2).min(Duration::from_secs(1));
      }
    }
  }
}
