use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::time::Instant;

use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

/// Probe for kitty keyboard protocol support on Unix.
///
/// Opens `/dev/tty` directly (bypassing crossterm's internal event system,
/// which is subject to timing issues on some terminals), builds a
/// [`super::TtyChannel`] over it, and delegates the query/response loop to
/// [`super::run_probe`].
///
/// Must be called after `enable_raw_mode()`.
pub(super) fn probe_kitty_support() -> io::Result<bool> {
    let tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    super::run_probe(&mut TtyChannel { file: tty })
}

/// [`super::ProbeChannel`] backed by an open `/dev/tty` `File`, using
/// `poll(2)` to wait for input with a deadline.
struct TtyChannel {
    file: std::fs::File,
}

impl super::ProbeChannel for TtyChannel {
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.file.write_all(buf)
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }

    fn wait_until(&mut self, deadline: Instant) -> io::Result<bool> {
        let remaining_ms = deadline
            .checked_duration_since(Instant::now())
            .map(|d| d.as_millis() as u32)
            .unwrap_or(0);
        if remaining_ms == 0 {
            return Ok(false);
        }
        let mut pfds = [PollFd::new(self.file.as_fd(), PollFlags::POLLIN)];
        // `poll` returns ready count (>0), 0 on timeout, or Errno. EINTR is
        // transient — collapse it to "not ready" and let the shared loop
        // retry until the overall deadline expires. Any other errno (EBADF,
        // EINVAL, …) is a permanent channel failure the caller surfaces to
        // the user as a kitty-probe error. The 500 ms probe budget fits
        // comfortably in `u16` (max 65535 ms), nix's only non-trivial `From`
        // impl for `PollTimeout`.
        match poll(&mut pfds, PollTimeout::from(remaining_ms as u16)) {
            Ok(ready) => Ok(ready > 0),
            Err(nix::errno::Errno::EINTR) => Ok(false),
            Err(e) => Err(io::Error::from(e)),
        }
    }
}
