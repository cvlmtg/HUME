//! Platform abstraction layer for HUME.
//!
//! Home for the codebase's platform-conditional code — terminal control,
//! process spawning with process-group/reap discipline, and OS-specific
//! directory/path conventions. Each sub-module is a narrow surface for one
//! concern:
//!
//! - [`terminal`] — raw-mode lifecycle, ratatui `Terminal` type alias,
//!   cursor shape/colour, kitty keyboard protocol, synchronized updates,
//!   and the inline-subprocess output flow.
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
/// both reach an exit path once [`QUIT_GRACE`] elapses: `QUIT_GRACE`'s own
/// doc comment acknowledges its budget is sized to just barely exceed the
/// main thread's worst-case teardown, so with enough attached LSP servers
/// the two windows overlap. Without a single winner, both would write the
/// terminal-restore escape sequences to the same [`terminal::SharedTerm`]
/// and both would call `process::exit` — interleaved restores can leave the
/// shell in the alternate screen or raw mode, and a second `exit` re-enters
/// the same atexit/TLS teardown.
///
/// A caller that loses the race parks forever rather than returning: it has
/// nothing left to do once another thread is committed to tearing the
/// process down, and returning here would race that thread's own
/// `process::exit` on which one actually observes to run last.
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
/// hanging up with no signal delivered at all.
///
/// That last case is why this isn't just a signal handler: hume is rarely
/// the session leader of the tty it runs under, so a pty teardown (e.g. a
/// recording tool tearing down after capture) does not reliably deliver
/// SIGHUP. Without an independent watch on the terminal itself, the event
/// reader's idle wait spins at 100% CPU forever instead of returning, since
/// the underlying read primitive maps EOF to "no event" rather than an
/// error.
///
/// `request_quit` is called with the exit code the process should use —
/// `128 + signo` on Unix, or 130 on Windows, where `ctrlc` doesn't expose
/// which control event fired — and routes termination through the editor's
/// normal quit path (graceful LSP `shutdown`) rather than tearing the
/// terminal down from this thread directly. This thread then waits up to
/// [`QUIT_GRACE`] for the main loop to exit the process itself before
/// force-restoring and exiting with that same code anyway. A pty hangup
/// can't take this route at all: the tty is already gone, which pins the
/// main loop's own event reader in an internal spin that never observes a
/// wake, so `request_quit` is never called — this thread force-exits with
/// 130 immediately instead.
///
/// On Unix this is one thread multiplexing two wake sources with a single
/// `select`: a `signal_hook` self-pipe (SIGINT/SIGTERM/SIGHUP/SIGQUIT) and a
/// dedicated `/dev/tty` fd (hangup), best-effort — a process with no
/// controlling terminal gets signal handling with no hangup watch rather
/// than losing both — the same pattern `termina` itself uses internally for
/// `SIGWINCH`. On Windows it's `ctrlc`'s dedicated worker thread, which
/// already covers everything Windows needs.
///
/// A signal disposition is only ever replaced once something is able to act
/// on it: the Unix thread is spawned before any signal is registered, and a
/// `register_conditional_shutdown` fallback still terminates the process
/// (without a graceful LSP shutdown or terminal restore) if that thread is
/// ever lost — see `unix::spawn_terminator`'s module-level comment for why.
///
/// In raw mode the kernel does not deliver SIGINT for Ctrl+C (ISIG is
/// cleared), so on Unix this primarily covers `kill <pid>` and pty teardown
/// — SIGINT is still registered for the rare case something re-enables ISIG.
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

/// Scans `buf` for complete `ESC [ ? <digits/;>* <final>` replies — the shape
/// both the kitty-flags response (`ESC[?<n>u`) and the DA1 response
/// (`ESC[?<n>c`) take — and collects each one's final byte, in order.
/// Shared by [`has_kitty_response`] and [`has_da1_response`], which differ
/// only in which final byte they're looking for; both responses may appear
/// in the same buffer; that's what a match on the other one's final byte
/// mid-scan is for — it's still a complete sequence, just not the one this
/// caller wants, so scanning continues past it rather than aborting.
///
/// A sequence that runs out of buffer before a final byte arrives is
/// incomplete and contributes nothing — the scan simply stops there, so a
/// truncated tail can never fabricate a match.
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
