//! Platform abstraction layer for HUME.
//!
//! Consolidates all OS-specific operations so the rest of the codebase never
//! calls `std::fs`, `std::process::Command`, or terminal escape sequences
//! directly. Each sub-module is a narrow, auditable surface for one concern:
//!
//! - [`terminal`] — raw-mode lifecycle, ratatui `Terminal` type alias,
//!   cursor shape/colour, kitty keyboard protocol, synchronized updates,
//!   and the inline-subprocess output flow.
//! - [`io`] — atomic file writes that preserve permissions and ownership.
//! - [`fs`] — thin `std::fs` wrappers (the audit allow-list).
//! - [`process`] — `std::process::Command` wrappers (the audit allow-list).
//! - [`dirs`] — XDG/platform config, data, home, and runtime directories.
//! - [`path`] — tilde/env-var expansion and path-separator utilities.
//!
//! All platform-conditional code (`#[cfg(unix)]`, `#[cfg(windows)]`) is
//! hidden behind private sub-modules; every public function has a uniform
//! signature across platforms.

#[cfg(unix)]
mod unix;

pub mod dirs;
pub mod fs;
pub mod io;
pub mod path;
pub mod process;
pub mod target;
pub mod terminal;

use std::time::{Duration, Instant};

/// Install a process-wide signal handler that restores the terminal before
/// exiting.
///
/// Catches SIGINT/SIGTERM/SIGHUP on Unix and Ctrl+C/Ctrl+Break on Windows.
/// In raw mode the kernel does not deliver SIGINT for Ctrl+C (ISIG is
/// cleared), so this primarily covers `kill <pid>`, `kill -HUP`, and
/// terminal-window close sent by the OS.
///
/// The handler runs on a dedicated worker thread managed by `ctrlc`, so it is
/// safe to call `restore()` (which allocates and writes to the terminal).
pub fn install_signal_handlers(term: terminal::SharedTerm) -> Result<(), ctrlc::Error> {
    ctrlc::set_handler(move || {
        if let Err(e) = crate::terminal::restore(&term) {
            eprintln!("hume: terminal restore failed (signal): {e}");
        }
        // Exit with the conventional "killed by signal" code. ctrlc does not
        // tell us which signal fired, so 130 is a reasonable default.
        std::process::exit(130);
    })
}

/// Probe the terminal for kitty keyboard protocol support.
///
/// On Unix, constructs a [`ProbeChannel`] over native polling primitives and
/// delegates the query/response loop to [`run_probe`]. Returns `Ok(true)` if
/// the terminal supports kitty keyboard protocol push, `Ok(false)` otherwise.
///
/// On Windows, writes the same kitty query (plus a DA1 fence) through `term`
/// and waits for termina's event reader to decode a reply — see
/// [`probe_via_events`]. This is the same decode path real input goes
/// through, so a `true` here means kitty-encoded keys will actually work,
/// unlike asking the terminal in the abstract (ConPTY from Windows Terminal
/// ≥ 1.25 answers the query even when nothing downstream could decode the
/// reply).
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
/// Typed keys the user presses during the probe window are filtered out
/// (not consumed) and stay buffered in the [`EventReader`](termina::EventReader)
/// for the main loop to read — unlike the Unix byte-channel probe, which has
/// no such buffering and can eat a keystroke typed during the race.
#[cfg(windows)]
fn probe_via_events(term: &terminal::SharedTerm) -> std::io::Result<bool> {
    use std::io::Write;
    use termina::escape::csi::{Csi, Device, Keyboard};

    let mut out = term.clone();
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
/// `Some(true)` — the terminal reported kitty keyboard protocol flags: it
/// supports the query, so it supports the push we're about to send. `Some(false)`
/// — the DA1 fence arrived with no kitty report first: the terminal answered
/// every query we sent and kitty was not among the replies. `None` — an
/// event outside this classification (e.g. a stray key); keep waiting.
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
/// Implemented on top of the native readable-with-deadline primitive
/// (`poll(2)` on Unix). The trait isolates the one OS-specific concern — "is
/// there input ready before `deadline`?" — so the query/response loop in
/// [`run_probe`] is platform-agnostic and unit-testable via a mock channel.
///
/// Only `unix::probe_kitty_support` implements it in production; on Windows
/// (kitty probing goes through [`probe_via_events`] instead) it exists only
/// for its unit tests, which is also why the whole family stays testable
/// cross-platform rather than being gated to `#[cfg(unix)]` outright.
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
/// Writes three queries — `\x1B[?u` (kitty flags), `\x1B[>q` (XTVERSION),
/// `\x1B[c` (DA1 sentinel) — then reads replies until DA1 arrives or the
/// deadline expires, and classifies via [`has_kitty_response`] /
/// [`has_kitty_xtversion`]. A single 500 ms overall deadline bounds the whole
/// exchange; local terminals reply in single-digit ms, slow/remote ones get
/// one generous budget rather than per-read timeouts.
///
/// Assumes terminal replies arrive in order: DA1 is the last response the
/// terminal sends to our three-query burst, so stopping at the first complete
/// DA1 is safe. Terminals that reordered DA1 ahead of an earlier kitty/XTVERSION
/// reply would cause us to miss it and report `false` — no known terminal does
/// this, but it is the assumption the early-stop rests on.
///
/// `Ok(0)` from the channel (clean EOF) breaks the loop and reports `false` —
/// the terminal went away without answering, so kitty is unavailable. An `Err`
/// from `read` or `wait_until` is a permanent channel failure and propagates to
/// the caller, which surfaces it to the user rather than degrading silently.
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
        // Ok(0) = clean EOF: terminal closed the channel mid-probe — stop
        // and report unsupported. Err = permanent failure; propagate.
        let n = match ch.read(&mut buf) {
            Ok(0) => break,
            Err(e) => return Err(e),
            Ok(n) => n,
        };
        response.extend_from_slice(&buf[..n]);

        // DA1 is the last reply; once it lands the terminal has finished
        // responding to all three queries.
        if has_da1_response(&response) {
            break;
        }
    }

    Ok(has_kitty_response(&response) || has_kitty_xtversion(&response))
}

/// Scan raw terminal response bytes for a kitty keyboard protocol reply.
///
/// Looks for the pattern `ESC [ ? <digits> u` which is the terminal's response
/// to the `\x1B[?u` query. DA1 sequences (`ESC [ ? <digits> c`) are skipped
/// over — they don't indicate kitty support but don't rule it out either, since
/// both responses may appear in the same buffer.
#[cfg_attr(not(any(unix, test)), allow(dead_code))]
fn has_kitty_response(buf: &[u8]) -> bool {
    let mut i = 0;
    while i + 2 < buf.len() {
        if buf[i] == 0x1B && buf[i + 1] == b'[' && buf[i + 2] == b'?' {
            let mut j = i + 3;
            while j < buf.len() {
                match buf[j] {
                    b'u' => return true, // kitty flags response
                    b'c' => break,       // DA1 — skip and keep scanning
                    b'0'..=b'9' | b';' => j += 1,
                    _ => break, // unexpected byte, abandon sequence
                }
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    false
}

/// Check XTVERSION response (`ESC P > | <name> ESC \`) against terminals known
/// to support kitty keyboard protocol push but that may not respond to the
/// `\x1B[?u` query (e.g. older WezTerm releases).
///
/// XTVERSION is sent alongside the kitty query and DA1 sentinel as a fallback
/// identification mechanism. Its response arrives before DA1.
#[cfg_attr(not(any(unix, test)), allow(dead_code))]
fn has_kitty_xtversion(buf: &[u8]) -> bool {
    // Find the DCS introducer for XTVERSION: ESC P > |
    let Some(pos) = buf.windows(4).position(|w| w == b"\x1BP>|") else {
        return false;
    };
    let name_start = pos + 4;
    // Find the String Terminator: ESC \
    let Some(st_pos) = buf[name_start..].windows(2).position(|w| w == b"\x1B\\") else {
        return false;
    };
    let name = &buf[name_start..name_start + st_pos];
    // Terminals confirmed to support kitty push regardless of query support.
    // kitty, ghostty, and foot also respond to the query, so they're redundant
    // here but harmless as a fallback.
    name.starts_with(b"WezTerm")
        || name.starts_with(b"kitty")
        || name.starts_with(b"ghostty")
        || name.starts_with(b"foot")
}

/// Returns true once the buffer contains a complete DA1 response (`ESC [ ? <digits> c`),
/// which signals the terminal has finished responding to all queries.
#[cfg_attr(not(any(unix, test)), allow(dead_code))]
fn has_da1_response(buf: &[u8]) -> bool {
    let mut i = 0;
    while i + 2 < buf.len() {
        if buf[i] == 0x1B && buf[i + 1] == b'[' && buf[i + 2] == b'?' {
            let mut j = i + 3;
            while j < buf.len() {
                match buf[j] {
                    b'c' => return true,
                    b'0'..=b'9' | b';' => j += 1,
                    _ => break,
                }
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    false
}

#[cfg(test)]
mod tests;
