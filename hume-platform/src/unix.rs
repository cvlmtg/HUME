use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::time::Instant;

use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

/// Probe for kitty keyboard protocol support on Unix.
///
/// Opens `/dev/tty` directly on its own side channel — independent of the
/// [`SharedTerm`](crate::terminal::SharedTerm) event reader, so probe replies
/// can never race or interleave with it — builds a [`super::TtyChannel`]
/// over it, and delegates the query/response loop to [`super::run_probe`].
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
        let mut pfds = [PollFd::new(self.file.as_fd(), PollFlags::POLLIN)];
        // Retry on EINTR (SIGWINCH/SIGCONT are common at startup) with the
        // budget recomputed each pass, so `Ok(false)` always means a genuine
        // deadline timeout — never a transient signal. Any other errno (EBADF,
        // EINVAL, …) is a permanent channel failure and propagates.
        // `PollTimeout` takes `u16`; the 500 ms probe budget fits comfortably.
        loop {
            let remaining_ms = deadline
                .checked_duration_since(Instant::now())
                .map(|d| d.as_millis() as u32)
                .unwrap_or(0);
            if remaining_ms == 0 {
                return Ok(false);
            }
            match poll(&mut pfds, PollTimeout::from(remaining_ms as u16)) {
                Ok(ready) => return Ok(ready > 0),
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => return Err(io::Error::from(e)),
            }
        }
    }
}
