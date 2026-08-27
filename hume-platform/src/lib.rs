//! Platform abstraction layer for HUME.
//!
//! Home for the codebase's platform-conditional code — terminal control,
//! process spawning with process-group/reap discipline, and OS-specific
//! directory/path conventions. Each sub-module is a narrow surface for one
//! concern:
//!
//! - [`terminal`] — raw-mode lifecycle, cursor shape/colour, kitty keyboard
//!   protocol, synchronized updates, and the inline-subprocess output flow.
//! - [`screen`] — double-buffered frame presentation: the cell diff and the
//!   escape-sequence emitter that carries a composed frame to the terminal.
//! - [`io`] — atomic file writes that preserve permissions and ownership.
//! - [`process`] — process-group-isolated spawning, plus the LSP-server
//!   install pipeline's platform-specific pieces (compiler selection,
//!   hashing, archive unpacking).
//! - [`dirs`] — XDG/platform config, data, home, and runtime directories.
//! - [`path`] — tilde/env-var expansion and path-separator utilities.
//!
//! All platform-conditional code (`#[cfg(unix)]`, `#[cfg(windows)]`) is
//! hidden behind private sub-modules; every public function has a uniform
//! signature across platforms.

#[cfg(unix)]
mod unix;

pub mod dirs;
pub mod io;
pub mod path;
pub mod process;
pub mod screen;
pub mod target;
pub mod terminal;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Grace window between asking the editor to quit gracefully and forcing the
/// process down. Must comfortably exceed the editor's own worst-case
/// teardown cost: `Editor::SHUTDOWN_GRACE`'s 500 ms budget
/// (`hume-editor/src/editor/lsp/mod.rs`), plus up to `ServerHandle::drop`'s
/// 200 ms `WRITER_FLUSH_GRACE` (`hume-lsp/src/transport.rs`) *per still-live
/// LSP server* as each one is dropped afterward — so a handful of attached
/// servers doesn't blow through the window mid-teardown.
pub(crate) const QUIT_GRACE: Duration = Duration::from_millis(3000);

/// [`QUIT_GRACE`], exposed to consumer crates so the budget it documents
/// itself as needing — `Editor::SHUTDOWN_GRACE` plus `WRITER_FLUSH_GRACE` per
/// live LSP server — can be checked against the real values instead of just
/// a comment promising they're kept in step (see the invariant test in
/// `hume-editor`).
#[cfg(any(test, feature = "test-util"))]
pub fn quit_grace() -> Duration {
    QUIT_GRACE
}

/// Windows' uniform "killed by signal" exit code — `ctrlc` fires one handler
/// for every console control event (Ctrl+C, Ctrl+Break, console close,
/// logoff, shutdown) without saying which, so there's no per-event code to
/// derive the way Unix derives `128 + signo`. Numerically the same as Unix's
/// `SIGINT` code, but that's incidental — a different exit code with a
/// different rationale, not the same constant reused.
#[cfg(windows)]
const WINDOWS_SIGNAL_EXIT_CODE: i32 = 130;

/// Set by whichever caller wins [`claim_exit`]'s race — see there.
static EXIT_CLAIMED: AtomicBool = AtomicBool::new(false);

/// Returns `true` for the one call, process-wide, that wins the race to
/// tear the process down; `false` for every call after it. Split out from
/// [`restore_for_exit`] so the claim itself — not the losing side's
/// park-forever — is unit-testable.
fn claim_exit() -> bool {
    !EXIT_CLAIMED.swap(true, Ordering::AcqRel)
}

/// Restores the terminal on behalf of the one thread that gets to take the
/// process down, then returns so the caller can call `std::process::exit`.
///
/// The terminator thread ([`spawn_terminator`]'s force-exit arms) and the
/// main thread (`hume-editor`'s `run`, after its own graceful shutdown) can
/// both reach an exit path once `QUIT_GRACE` elapses — its budget is sized
/// to just barely exceed the main thread's worst-case teardown, so with
/// enough attached LSP servers the two windows overlap. Without a single
/// winner, both would write terminal-restore sequences to the same
/// [`terminal::SharedTerm`] and both would call `process::exit` —
/// interleaved restores can leave the shell in the alt screen or raw mode,
/// and a second `exit` re-enters the same atexit/TLS teardown.
///
/// A caller that loses the race parks forever: it has nothing left to do
/// once another thread is committed to tearing the process down, and
/// returning would race that thread's own `process::exit`.
pub fn restore_for_exit(term: &terminal::SharedTerm) -> std::io::Result<()> {
    if !claim_exit() {
        loop {
            std::thread::park();
        }
    }
    terminal::restore(term)
}

/// Kills every still-registered [`process::tracked::TrackedChild`], restores
/// the terminal, and exits with `code`. Shared by every force-exit path —
/// [`unix::spawn_terminator`]'s signal and hangup arms, and the Windows arm
/// below — so there is one reap-restore-exit sequence rather than each
/// platform repeating it.
///
/// `process::exit` runs no destructors, so this is the only place LSP
/// servers and other long-lived children (normally reaped by their own
/// `Drop`) get killed on this path — see `process::tracked`'s module doc.
#[cfg_attr(not(any(unix, windows)), allow(dead_code))]
fn force_exit(term: &terminal::SharedTerm, code: i32) -> ! {
    process::tracked::kill_tracked_children();
    let _ = restore_for_exit(term);
    std::process::exit(code);
}

/// Spawn a background watcher that terminates the process on: Ctrl+C, `kill
/// <pid>` (SIGINT/SIGTERM/SIGHUP/SIGQUIT) on Unix, Ctrl+Break and
/// console-close on Windows, and — Unix only — the controlling terminal
/// hanging up with no signal delivered at all. Not a plain signal handler:
/// hume is rarely the session leader of its tty, so a pty teardown (e.g. a
/// recording tool closing after capture) doesn't reliably deliver SIGHUP,
/// and without an independent watch the event reader's idle wait spins at
/// 100% CPU forever instead of returning (the read primitive maps EOF to
/// "no event", not an error). See `unix::spawn_terminator` for how Unix
/// multiplexes both wake sources on one thread.
///
/// `request_quit` is called with the exit code the process should use —
/// `128 + signo` on Unix, 130 on Windows (`ctrlc` doesn't expose which
/// control event fired) — and routes through the editor's normal quit path
/// (graceful LSP `shutdown`) rather than tearing the terminal down here.
/// This thread then waits up to `QUIT_GRACE` for the main loop to exit on
/// its own before force-restoring and exiting with that code anyway. A pty
/// hangup can't take this route: the main loop's event reader is pinned at
/// tty EOF and never wakes, so this thread force-exits with 130 immediately.
///
/// In raw mode the kernel does not deliver SIGINT for Ctrl+C (ISIG is
/// cleared), so on Unix this primarily covers `kill <pid>` and pty teardown
/// — SIGINT stays registered for the rare case something re-enables ISIG.
pub fn spawn_terminator(
    term: terminal::SharedTerm,
    request_quit: impl Fn(i32) + Send + 'static,
) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        unix::spawn_terminator(term, request_quit)
    }
    #[cfg(windows)]
    {
        ctrlc::set_handler(move || {
            // ctrlc fires this handler for every console control event
            // (Ctrl+C, Ctrl+Break, console close, logoff, shutdown) without
            // telling us which one, so every trigger uses the same
            // conventional "killed by signal" code.
            request_quit(WINDOWS_SIGNAL_EXIT_CODE);
            // Unlike Unix's `wait_for_second_signal`, this sleep isn't
            // interruptible by a second control event: `ctrlc` gives no
            // shared wait primitive to interrupt, and every event already
            // maps to the same `WINDOWS_SIGNAL_EXIT_CODE`, so there's no
            // second signal's code to race ahead for. A repeat Ctrl+C during
            // this window is a harmless no-op, not a faster exit — accepted
            // asymmetry with the Unix path rather than a bug.
            std::thread::sleep(QUIT_GRACE);
            // The main loop had a full grace window and didn't exit the
            // process itself — force it down.
            force_exit(&term, WINDOWS_SIGNAL_EXIT_CODE);
        })
        .map_err(std::io::Error::other)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (term, request_quit);
        Ok(())
    }
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
/// [`has_kitty_xtversion`]. A single 500 ms deadline bounds the whole
/// exchange; local terminals reply in single-digit ms, slow/remote ones get
/// one generous budget rather than per-read timeouts.
///
/// Assumes replies arrive in order, so stopping at the first complete DA1 —
/// the terminal's last response to the three-query burst — is safe. A
/// terminal that reordered DA1 ahead of an earlier reply would be missed and
/// reported `false`; no known terminal does this.
///
/// Clean EOF (`Ok(0)`) breaks the loop and reports `false` — the terminal
/// went away without answering. Any other `Err` is a permanent channel
/// failure and propagates to the caller rather than degrading silently.
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

        // DA1 is the last reply; once it lands the terminal has finished
        // responding to all three queries.
        if has_da1_response(&response) {
            break;
        }
    }

    Ok(has_kitty_response(&response) || has_kitty_xtversion(&response))
}

/// Scans `buf` for complete `ESC [ ? <digits/;>* <final>` replies — the shape
/// both the kitty-flags response (`ESC[?<n>u`) and the DA1 response
/// (`ESC[?<n>c`) take — and collects each one's final byte, in order. Shared
/// by [`has_kitty_response`] and [`has_da1_response`], which only differ in
/// which final byte they look for; a match on the *other* byte mid-scan is
/// still a complete sequence, so scanning continues past it rather than
/// aborting.
///
/// A sequence that runs out of buffer before a final byte arrives is
/// incomplete and contributes nothing — a truncated tail can never
/// fabricate a match.
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

/// Scan raw terminal response bytes for a kitty keyboard protocol reply.
///
/// Looks for the pattern `ESC [ ? <digits> u` which is the terminal's response
/// to the `\x1B[?u` query. DA1 sequences (`ESC [ ? <digits> c`) are skipped
/// over — they don't indicate kitty support but don't rule it out either, since
/// both responses may appear in the same buffer.
#[cfg_attr(not(any(unix, test)), allow(dead_code))]
fn has_kitty_response(buf: &[u8]) -> bool {
    csi_final_bytes(buf).contains(&b'u')
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
    csi_final_bytes(buf).contains(&b'c')
}

#[cfg(test)]
mod tests;
