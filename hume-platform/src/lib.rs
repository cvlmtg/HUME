//! Platform abstraction layer for HUME.
//!
//! The codebase's platform-conditional code, one concern per module:
//!
//! - [`terminal`]: raw-mode lifecycle, cursor shape/colour, kitty keyboard
//!   protocol, synchronized updates, and the inline-subprocess output flow.
//! - [`screen`]: double-buffered frame presentation: the cell diff and the
//!   escape-sequence emitter.
//! - [`io`]: atomic file writes that preserve permissions and ownership.
//! - [`process`]: process-group-isolated spawning, plus the LSP-server
//!   install pipeline's platform-specific pieces (compiler selection,
//!   hashing, archive unpacking).
//! - [`dirs`]: XDG/platform config, data, home, and runtime directories.
//! - [`path`]: tilde/env-var expansion, path-separator utilities, and the
//!   programs on a search path.
//! - [`version`]: the running build's version and commit, stamped by `build.rs`.
//! - [`worker_panic`]: keeps a panic on a background thread from touching the
//!   terminal and queues it for the editor to report.
//! - [`wake`]: the callback a background thread wakes the main loop with, and
//!   the thread spawn that calls it when the thread ends.
//! - [`snapshots`]: a background thread that reports a growing result, read
//!   by its consumer one latest snapshot at a time.
//!
//! `#[cfg(unix)]`/`#[cfg(windows)]` code sits in private sub-modules; every
//! public function has one signature across platforms.

#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

pub mod dirs;
pub mod file_url;
pub mod io;
pub mod path;
pub mod process;
pub mod screen;
pub mod snapshots;
pub mod terminal;
pub mod version;
pub mod wake;
pub mod worker_panic;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Grace window between asking the editor to quit gracefully and forcing the
/// process down. Must exceed the editor's worst-case teardown:
/// `Editor::SHUTDOWN_GRACE` plus `WRITER_FLUSH_GRACE` per live LSP server,
/// each dropped in turn.
pub(crate) const QUIT_GRACE: Duration = Duration::from_millis(3000);

/// [`QUIT_GRACE`], for `hume-editor`'s test that checks it against the
/// teardown budget it must exceed.
#[cfg(any(test, feature = "test-util"))]
pub fn quit_grace() -> Duration {
    QUIT_GRACE
}

/// Windows' exit code for every console control event that ends HUME
/// (Ctrl-c, Ctrl-Break, console close, logoff, shutdown). With no signal
/// numbers to derive `128 + signo` from, every event exits with the shell's
/// code for an interrupted program.
#[cfg(windows)]
const WINDOWS_SIGNAL_EXIT_CODE: i32 = 130;

/// Set by whichever caller wins [`claim_exit`]'s race.
static EXIT_CLAIMED: AtomicBool = AtomicBool::new(false);

/// Returns `true` for the one call, process-wide, that wins the race to
/// tear the process down; `false` for every call after it.
fn claim_exit() -> bool {
    !EXIT_CLAIMED.swap(true, Ordering::AcqRel)
}

/// Restores the terminal on behalf of the one thread that gets to take the
/// process down, then returns so the caller can call `std::process::exit`.
///
/// The terminator thread ([`spawn_terminator`]) and the main thread (after
/// its graceful shutdown) can both reach an exit path once `QUIT_GRACE`
/// elapses. Two winners would interleave restore sequences on one
/// [`terminal::SharedTerm`], which can leave the shell in the alt screen or
/// raw mode, and a second `exit` re-enters the atexit/TLS teardown.
///
/// A caller that loses the race parks forever, since returning would race
/// the winner's `process::exit`.
pub fn restore_for_exit(term: &terminal::SharedTerm) -> std::io::Result<()> {
    if !claim_exit() {
        loop {
            std::thread::park();
        }
    }
    terminal::restore(term)
}

/// Restores the terminal, then kills every still-registered
/// [`process::tracked::TrackedChild`], then exits with `code`.
///
/// The reap runs after [`restore_for_exit`]'s claim: a losing caller parks
/// forever, and reaping first would kill the LSP servers of the thread that
/// won and is mid graceful shutdown.
///
/// `process::exit` runs no destructors, so this is the only place long-lived
/// children (normally reaped by their `Drop`) get killed on this path.
#[cfg_attr(not(any(unix, windows)), allow(dead_code))]
fn force_exit(term: &terminal::SharedTerm, code: i32) -> ! {
    let _ = restore_for_exit(term);
    process::tracked::kill_tracked_children();
    std::process::exit(code);
}

/// Exit code for a bare terminal hangup with no signal delivered. A hangup
/// has no `signo` of its own, so it takes `SIGINT`'s `128 + 2`.
const HANGUP_EXIT_CODE: i32 = 130;

/// `Some(HANGUP_EXIT_CODE)` when `err` means the controlling terminal went
/// away, `None` for any other error, which the caller reports as-is.
///
/// Call it only on errors from terminal I/O: the kinds below carry no
/// source tag, so a file read's `UnexpectedEof` would be misreported as a
/// hangup.
///
/// - `UnexpectedEof`: termina's error for a zero-byte read on a tty `poll`
///   reported ready, what a pty slave gives on macOS/BSD once the master
///   closes.
/// - Raw `EIO`/`ENXIO` on Unix: what a Linux pty slave surfaces, or the
///   device lost from under the fd. Any other errno (`EBADF`, `ENOTTY`)
///   means a broken fd, not a hangup.
pub fn hangup_exit_code(err: &std::io::Error) -> Option<i32> {
    if err.kind() == std::io::ErrorKind::UnexpectedEof {
        return Some(HANGUP_EXIT_CODE);
    }
    #[cfg(unix)]
    {
        use nix::errno::Errno;
        if matches!(
            err.raw_os_error().map(Errno::from_raw),
            Some(Errno::EIO) | Some(Errno::ENXIO)
        ) {
            return Some(HANGUP_EXIT_CODE);
        }
    }
    None
}

/// Install the watcher that terminates the process on Ctrl-c or
/// `kill <pid>` (SIGINT/SIGTERM/SIGHUP/SIGQUIT) on Unix, and on Ctrl-c,
/// Ctrl-Break and console close on Windows.
///
/// `request_quit` gets the exit code to use (`128 + signo` on Unix, 130 on
/// Windows) and routes through the editor's normal quit path. The watcher
/// then waits up to `QUIT_GRACE` for the main loop to exit before
/// force-restoring and exiting with that code; a second trigger inside the
/// window is ignored. A bare terminal hangup with no signal is handled by
/// [`hangup_exit_code`] instead.
///
/// In raw mode Ctrl-c reaches HUME as a key, not as SIGINT or a console
/// event. While a `terminal::InlineOutputScreen` lives, the cooked terminal
/// sends Ctrl-c to the running child as well, and this watcher drops it: on
/// Unix SIGINT and SIGQUIT, on Windows Ctrl-c and Ctrl-Break.
pub fn spawn_terminator(
    term: terminal::SharedTerm,
    request_quit: impl Fn(i32) + Send + 'static,
) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        unix::spawn_terminator(term, request_quit)?;
    }
    #[cfg(windows)]
    {
        windows::install_terminator(term, request_quit)?;
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (term, request_quit);
    }
    Ok(())
}

/// Probe the terminal for kitty keyboard protocol support.
///
/// Returns `Ok(true)` if the terminal supports kitty keyboard protocol push.
/// Unix runs [`run_probe`] over a [`ProbeChannel`].
///
/// Windows decodes the reply through termina's event reader, the path real
/// input takes, so `true` means kitty-encoded keys will work: ConPTY from
/// Windows Terminal ≥ 1.25 answers the query even when nothing downstream
/// could decode the keys.
///
/// Must be called after `enable_raw_mode()`.
pub(crate) fn probe_kitty_support(term: &terminal::SharedTerm) -> std::io::Result<bool> {
    #[cfg(unix)]
    {
        let _ = term;
        unix::probe_kitty_support()
    }
    #[cfg(windows)]
    {
        probe_via_events(term)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = term;
        Ok(false)
    }
}

/// Windows kitty probe: write the progressive-enhancement query plus a DA1
/// fence, then wait for termina's event reader to decode either reply.
///
/// Keys typed during the probe are filtered out, not consumed, and stay
/// buffered in the [`EventReader`](termina::EventReader) for the main loop.
#[cfg(windows)]
fn probe_via_events(term: &terminal::SharedTerm) -> std::io::Result<bool> {
    use std::io::Write;
    use termina::escape::csi::{Csi, Device, Keyboard};

    let mut out = term;
    write!(
        out,
        "{}{}",
        Csi::Keyboard(Keyboard::QueryFlags),
        Csi::Device(Device::RequestPrimaryDeviceAttributes),
    )?;
    out.flush()?;

    let reader = term.event_reader();
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            return Ok(false); // timeout
        };
        if !reader.poll(Some(left), |e| classify_probe_event(e).is_some())? {
            return Ok(false); // timeout
        }
        let event = reader.read(|e| classify_probe_event(e).is_some())?;
        if let Some(verdict) = classify_probe_event(&event) {
            return Ok(verdict);
        }
    }
}

/// Classifies one event from the Windows probe's query/response exchange.
///
/// `Some(true)` for a kitty keyboard flags report, `Some(false)` for the DA1
/// fence arriving first (kitty unsupported), `None` for anything else (a
/// stray key): keep waiting.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn classify_probe_event(ev: &termina::Event) -> Option<bool> {
    use termina::Event;
    use termina::escape::csi::{Csi, Device, Keyboard};

    match ev {
        Event::Csi(Csi::Keyboard(Keyboard::ReportFlags(_))) => Some(true),
        Event::Csi(Csi::Device(Device::DeviceAttributes(_))) => Some(false),
        _ => None,
    }
}

/// Bidirectional byte channel used by [`run_probe`] to query the terminal.
///
/// Isolates the one OS-specific concern, "is there input before
/// `deadline`?" (`poll(2)` on Unix), so [`run_probe`] is unit-testable with a
/// mock channel on every platform. Only Unix implements it in production.
#[cfg_attr(not(any(unix, test)), allow(dead_code))]
trait ProbeChannel {
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()>;
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
    /// Block until readable or until `deadline` elapses. Returns `Ok(true)`
    /// when input is available, `Ok(false)` on timeout, `Err` on a permanent
    /// channel failure.
    fn wait_until(&mut self, deadline: Instant) -> std::io::Result<bool>;
}

/// Shared kitty-keyboard-protocol probe body.
///
/// Writes `\x1B[?u` (kitty flags), `\x1B[>q` (XTVERSION) and `\x1B[c` (DA1
/// sentinel), then reads replies until DA1 arrives or a single 500 ms
/// deadline for the whole exchange expires.
///
/// Assumes replies arrive in order, so DA1 is the last; no known terminal
/// reorders them.
///
/// Clean EOF (`Ok(0)`) reports `false`. Any other `Err` propagates.
#[cfg_attr(not(any(unix, test)), allow(dead_code))]
fn run_probe(ch: &mut impl ProbeChannel) -> std::io::Result<bool> {
    ch.write_all(b"\x1B[?u\x1B[>q\x1B[c")?;

    let mut response = Vec::with_capacity(256);
    let mut buf = [0u8; 256];
    let deadline = Instant::now() + Duration::from_millis(500);

    loop {
        if !ch.wait_until(deadline)? {
            break; // timeout
        }
        let n = match ch.read(&mut buf) {
            Ok(0) => break,
            Err(e) => return Err(e),
            Ok(n) => n,
        };
        response.extend_from_slice(&buf[..n]);

        if has_da1_response(&response) {
            break;
        }
    }

    Ok(has_kitty_response(&response) || has_kitty_xtversion(&response))
}

/// The final byte of each complete `ESC [ ? <digits/;>* <final>` reply in
/// `buf`, in order: the shape of both the kitty-flags (`u`) and DA1 (`c`)
/// responses. A truncated tail contributes nothing.
#[cfg_attr(not(any(unix, test)), allow(dead_code))]
fn csi_final_bytes(buf: &[u8]) -> Vec<u8> {
    let mut finals = Vec::new();
    let mut i = 0;
    while i + 2 < buf.len() {
        if buf[i] == 0x1B && buf[i + 1] == b'[' && buf[i + 2] == b'?' {
            let mut j = i + 3;
            while j < buf.len() && matches!(buf[j], b'0'..=b'9' | b';') {
                j += 1;
            }
            if j < buf.len() {
                finals.push(buf[j]);
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    finals
}

/// Whether `buf` holds a kitty keyboard flags reply (`ESC [ ? <digits> u`).
#[cfg_attr(not(any(unix, test)), allow(dead_code))]
fn has_kitty_response(buf: &[u8]) -> bool {
    csi_final_bytes(buf).contains(&b'u')
}

/// Check XTVERSION response (`ESC P > | <name> ESC \`) against terminals known
/// to support kitty keyboard protocol push but that may not respond to the
/// `\x1B[?u` query (e.g. older WezTerm releases).
#[cfg_attr(not(any(unix, test)), allow(dead_code))]
fn has_kitty_xtversion(buf: &[u8]) -> bool {
    // DCS introducer: ESC P > |
    let Some(pos) = buf.windows(4).position(|w| w == b"\x1BP>|") else {
        return false;
    };
    let name_start = pos + 4;
    // String Terminator: ESC \
    let Some(st_pos) = buf[name_start..].windows(2).position(|w| w == b"\x1B\\") else {
        return false;
    };
    let name = &buf[name_start..name_start + st_pos];
    // kitty, ghostty and foot also answer the query; listed as a fallback.
    name.starts_with(b"WezTerm")
        || name.starts_with(b"kitty")
        || name.starts_with(b"ghostty")
        || name.starts_with(b"foot")
}

/// Whether `buf` holds a complete DA1 response (`ESC [ ? <digits> c`).
#[cfg_attr(not(any(unix, test)), allow(dead_code))]
fn has_da1_response(buf: &[u8]) -> bool {
    csi_final_bytes(buf).contains(&b'c')
}

#[cfg(test)]
mod tests;
