//! The MPD fifo: opening (recreate + single-reader lock), waiting for
//! data with `poll(2)`, and the read/analyze loop of one opened fifo.

use std::{
  io::{Read, Result as IoResult},
  sync::atomic::Ordering,
  time::{Duration, Instant},
};

use tokio::sync::mpsc;

use super::{VisualizerHandle, analysis::Analyzer};
use crate::event::AsyncEvent;

/// How long one wait on the fifo may block before the reader re-checks
/// the stop / active flags.
pub(super) const FIFO_WAIT: Duration = Duration::from_millis(250);

/// Read and analyze one opened fifo until it fails (returns true: reopen)
/// or the visualizer stops / the app goes away (returns false).
pub(super) fn pump_fifo(
  mut fifo: std::fs::File,
  handle: &VisualizerHandle,
  analyzer: &mut Analyzer,
  read_buf: &mut [u8],
  frame_period: Duration,
  events: &mpsc::UnboundedSender<AsyncEvent>,
) -> bool {
  let mut last_frame = Instant::now();
  let mut was_active = false;
  loop {
    if handle.stop.load(Ordering::SeqCst) {
      return false;
    }
    let active = handle.active.load(Ordering::Relaxed);
    if !active {
      // Hidden pane: leave the fifo alone (MPD drains a full fifo itself)
      // and forget the partial analysis state.
      if was_active {
        analyzer.reset();
        was_active = false;
      }
      std::thread::sleep(FIFO_WAIT);
      continue;
    }
    if !was_active {
      // Skip the audio that piled up while hidden.
      drain_fifo(&mut fifo, read_buf);
      was_active = true;
    }
    match wait_readable(&fifo, FIFO_WAIT) {
      Ok(true) => {}
      Ok(false) => continue,
      Err(_) => return true,
    }
    match fifo.read(read_buf) {
      // The fifo is opened read-write, so EOF means it is no fifo at all.
      Ok(0) => return true,
      Ok(read) => analyzer.push_bytes(&read_buf[..read]),
      Err(error)
        if matches!(
          error.kind(),
          std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
        ) => {}
      Err(_) => return true,
    }
    if last_frame.elapsed() >= frame_period {
      let columns = handle.columns.load(Ordering::Relaxed);
      if let Some(bars) = analyzer.frame(columns) {
        last_frame = Instant::now();
        if events.send(AsyncEvent::Spectrum(bars)).is_err() {
          return false;
        }
      }
    }
  }
}

/// Block until the fifo has data (true) or `timeout` passes (false).
fn wait_readable(file: &std::fs::File, timeout: Duration) -> IoResult<bool> {
  use std::os::fd::AsRawFd;
  let mut poll_fd = libc::pollfd {
    fd: file.as_raw_fd(),
    events: libc::POLLIN,
    revents: 0,
  };
  let ready = unsafe { libc::poll(&mut poll_fd, 1, timeout.as_millis() as libc::c_int) };
  if ready < 0 {
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::Interrupted {
      return Ok(false);
    }
    return Err(error);
  }
  Ok(ready > 0)
}

/// Discard whatever is buffered in the (non-blocking) fifo.
fn drain_fifo(fifo: &mut std::fs::File, buf: &mut [u8]) {
  while let Ok(read) = fifo.read(buf) {
    if read == 0 {
      break;
    }
  }
}

pub(super) fn open_fifo(path: &str) -> IoResult<std::fs::File> {
  use std::os::fd::AsRawFd;
  use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
  // mpd only creates the fifo when it loads its config; anything that
  // wipes it afterwards (e.g. a /tmp cleaner) leaves both sides stranded
  // until the fifo exists again. Recreate it so mpd can reconnect on its
  // next output open.
  if !std::path::Path::new(path).exists()
    && let Ok(cpath) = std::ffi::CString::new(path)
  {
    unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) };
    // Ignore mkfifo errors: a racing creator (or a bad path) surfaces
    // as the open error below.
  }
  let file = std::fs::OpenOptions::new()
    .read(true)
    .write(true)
    .custom_flags(libc::O_NONBLOCK)
    .open(path)?;
  // A regular file here would be read to its end over and over.
  if !file.metadata()?.file_type().is_fifo() {
    return Err(std::io::Error::new(
      std::io::ErrorKind::InvalidInput,
      format!("{path} is not a fifo"),
    ));
  }
  // The fifo stream is single-consumer: if a second music-tui instance
  // (or any other reader) opened it first, the kernel would split the
  // samples between both readers and garble every spectrum. An exclusive
  // advisory lock on the fifo makes the loser back off cleanly. The lock
  // lives on the open file description and is released on close/drop.
  let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
  if locked != 0 {
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock {
      tracing::info!("visualizer fifo {path} is held by another instance; backing off");
      return Err(std::io::Error::new(
        std::io::ErrorKind::ResourceBusy,
        format!("fifo {path} is already read by another instance"),
      ));
    }
    return Err(error);
  }
  tracing::info!("visualizer locked fifo {path}");
  Ok(file)
}
