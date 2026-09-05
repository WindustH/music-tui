use std::io::{self, Stderr};

use anyhow::Result;
use crossterm::{
  event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
  execute,
  terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use img_tui::{ProtocolFrameOutput, ProtocolFrameRenderer, reset_protocol_images};
use ratatui::{Frame, Terminal, prelude::CrosstermBackend};

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
