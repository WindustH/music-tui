//! Terminal ownership: the framework-tui session plus the cover images
//! drawn on it, which are erased before the terminal is handed to another
//! program and reset when it comes back.

use anyhow::Result;
use framework_tui::{SuspendTerminal, TerminalOptions, TerminalOutput, TerminalSession};
use img_tui::{ProtocolFrameOutput, ProtocolFrameRenderer, reset_protocol_images};
use ratatui::Frame;

pub type FrameOutput = ProtocolFrameOutput;

pub struct Tui {
  session: TerminalSession,
  protocol_renderer: ProtocolFrameRenderer,
  protocol_reset: Option<String>,
}

impl Tui {
  /// Enter the TUI. A failed step leaves the terminal as it was.
  pub fn new(protocol_reset: Option<String>) -> Result<Self> {
    let mut session = TerminalSession::enter(TerminalOptions {
      output: TerminalOutput::Stderr,
      // Unbuffered, as before; a buffer would batch each frame's writes.
      buffer_capacity: 0,
      ..TerminalOptions::default()
    })?;
    reset_protocol_images(session.backend_mut(), protocol_reset.as_deref())?;
    Ok(Self {
      session,
      protocol_renderer: ProtocolFrameRenderer::default(),
      protocol_reset,
    })
  }

  pub fn draw<F>(&mut self, render: F) -> Result<()>
  where
    F: FnOnce(&mut Frame) -> FrameOutput,
  {
    self
      .protocol_renderer
      .draw(self.session.terminal_mut(), render)
  }

  pub fn restore(&mut self) -> Result<()> {
    let images = self.clear_images();
    let session = self.session.restore();
    images.and(session.map_err(Into::into))
  }

  fn clear_images(&mut self) -> Result<()> {
    if self.session.is_suspended() || self.session.is_restored() {
      return Ok(());
    }
    self
      .protocol_renderer
      .clear_and_reset(self.session.backend_mut(), self.protocol_reset.as_deref())
  }
}

impl SuspendTerminal for Tui {
  type Error = anyhow::Error;

  fn suspend(&mut self) -> Result<()> {
    let images = self.clear_images();
    let session = self.session.suspend();
    images.and(session.map_err(Into::into))
  }

  fn resume(&mut self) -> Result<()> {
    if !self.session.is_suspended() {
      return Ok(());
    }
    self.session.resume()?;
    reset_protocol_images(self.session.backend_mut(), self.protocol_reset.as_deref())
  }
}

impl Drop for Tui {
  fn drop(&mut self) {
    if !std::thread::panicking() {
      let _ = self.restore();
    }
  }
}
