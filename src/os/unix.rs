use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::os::unix::io::AsRawFd;

/// Probe for kitty keyboard protocol support by querying the terminal directly.
///
/// Sends three queries in one write: `\x1B[?u` (kitty keyboard protocol flags
/// query), `\x1B[>q` (XTVERSION — terminal name/version, used as a fallback
/// for terminals that support kitty push but don't answer the flags query),
/// and `\x1B[c` (DA1 sentinel — virtually all terminals respond to this,
/// bounding the read loop).
///
/// We write to and read from `/dev/tty` directly, bypassing crossterm's
/// internal event system, which is subject to timing issues on some terminals.
///
/// Must be called after `enable_raw_mode()`.
pub(super) fn probe_kitty_support() -> io::Result<bool> {
    let mut tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    let fd = tty.as_raw_fd();

    // Send three queries together:
    //   \x1B[?u   — kitty keyboard protocol flags query
    //   \x1B[>q   — XTVERSION (terminal name/version), fallback for terminals
    //               that support kitty push but don't answer the flags query
    //   \x1B[c    — DA1 sentinel; virtually all terminals respond to this,
    //               and its response arrives last, so it terminates our read loop
    tty.write_all(b"\x1B[?u\x1B[>q\x1B[c")?;
    tty.flush()?;

    let mut response = Vec::with_capacity(256);
    let mut buf = [0u8; 256]; // large enough for kitty + XTVERSION + DA1

    // We sent three queries and expect up to three responses:
    //   \x1B[?<flags>u        — kitty flags reply   (only on kitty-capable terminals)
    //   \x1BP>|<name>\x1B\\   — XTVERSION reply     (most modern terminals)
    //   \x1B[?<attrs>c        — DA1 sentinel         (virtually all terminals)
    //
    // We read until we have seen the DA1 'c' terminator, which signals that the
    // terminal has finished responding. Stopping at the first 'c' or 'u' would
    // risk missing the kitty response if the replies arrive in separate reads.
    //
    // Initial timeout is generous to handle slow/remote terminals; subsequent
    // reads use a short timeout since bytes arrive nearly instantaneously once
    // the terminal starts responding.
    let mut timeout_ms: i32 = 500;
    loop {
        // SAFETY: poll is safe to call with a valid fd and a properly initialised pollfd.
        let ready = unsafe {
            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            libc::poll(&mut pfd, 1, timeout_ms)
        };

        if ready <= 0 {
            break; // timeout or error — stop reading
        }

        let n = tty.read(&mut buf)?;
        if n == 0 {
            break;
        }
        response.extend_from_slice(&buf[..n]);

        // Keep reading until we see a complete DA1 response (ESC [ ? ... c),
        // which is the last thing the terminal sends. Checking for the full
        // CSI sequence rather than a raw 'c' byte avoids false positives from
        // any 'c' that might appear mid-stream.
        if super::has_da1_response(&response) {
            break;
        }

        // Short timeout for follow-up reads: if more bytes are coming they'll
        // arrive almost immediately.
        timeout_ms = 50;
    }

    Ok(super::has_kitty_response(&response) || super::has_kitty_xtversion(&response))
}
