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
mod tests {
    use super::{
        classify_probe_event, has_da1_response, has_kitty_response, has_kitty_xtversion, run_probe,
    };
    use std::io;
    use std::time::Instant;

    // ── has_kitty_response ────────────────────────────────────────────────────

    #[test]
    fn kitty_response_before_da1() {
        assert!(has_kitty_response(b"\x1B[?0u\x1B[?62;22c"));
    }

    #[test]
    fn kitty_response_after_da1() {
        // Race condition case: DA1 arrives first, kitty response second.
        assert!(has_kitty_response(b"\x1B[?62;22c\x1B[?0u"));
    }

    #[test]
    fn kitty_response_with_multi_semicolon_flags() {
        assert!(has_kitty_response(b"\x1B[?1;2;3u"));
    }

    #[test]
    fn kitty_response_preceded_by_noise() {
        assert!(has_kitty_response(b"noise\x1B[?0u"));
    }

    #[test]
    fn no_kitty_response_from_da1_only() {
        assert!(!has_kitty_response(b"\x1B[?1;0c"));
    }

    #[test]
    fn no_kitty_response_from_empty() {
        assert!(!has_kitty_response(b""));
    }

    #[test]
    fn no_kitty_response_from_three_byte_boundary() {
        assert!(!has_kitty_response(b"\x1B[?"));
    }

    // ── has_da1_response ──────────────────────────────────────────────────────

    #[test]
    fn da1_detected() {
        assert!(has_da1_response(b"\x1B[?1;0c"));
    }

    #[test]
    fn da1_detected_after_kitty_response() {
        assert!(has_da1_response(b"\x1B[?0u\x1B[?62;22c"));
    }

    #[test]
    fn no_da1_from_empty() {
        assert!(!has_da1_response(b""));
    }

    #[test]
    fn incomplete_da1_does_not_match() {
        assert!(!has_da1_response(b"\x1B[?62;2"));
    }

    #[test]
    fn no_da1_from_three_byte_boundary() {
        assert!(!has_da1_response(b"\x1B[?"));
    }

    // ── has_kitty_xtversion ───────────────────────────────────────────────────

    #[test]
    fn xtversion_wezterm() {
        let buf = b"\x1BP>|WezTerm 20240203-110809-5046fc22\x1B\\\x1B[?65;4;6;18;22c";
        assert!(!has_kitty_response(buf));
        assert!(has_kitty_xtversion(buf));
    }

    #[test]
    fn xtversion_kitty() {
        assert!(has_kitty_xtversion(b"\x1BP>|kitty(0.35.2)\x1B\\\x1B[?1c"));
    }

    #[test]
    fn xtversion_ghostty() {
        assert!(has_kitty_xtversion(b"\x1BP>|ghostty 1.0.0\x1B\\\x1B[?1c"));
    }

    #[test]
    fn xtversion_foot() {
        assert!(has_kitty_xtversion(b"\x1BP>|foot(1.17.0)\x1B\\\x1B[?1c"));
    }

    #[test]
    fn xtversion_iterm2_not_matched() {
        assert!(!has_kitty_xtversion(b"\x1BP>|iTerm2 3.5\x1B\\\x1B[?1;2c"));
    }

    #[test]
    fn xtversion_no_response() {
        assert!(!has_kitty_xtversion(b"\x1B[?1;2c"));
    }

    #[test]
    fn xtversion_incomplete_no_st() {
        assert!(!has_kitty_xtversion(b"\x1BP>|WezTerm 20240203"));
    }

    #[test]
    fn xtversion_empty_name() {
        // Empty name should not match any known terminal.
        assert!(!has_kitty_xtversion(b"\x1BP>|\x1B\\\x1B[?1c"));
    }

    // ── run_probe (shared probe loop via a mock channel) ──────────────────────
    //
    // These tests exercise the platform-agnostic body that unix/windows both
    // delegate to, so the probe classification loop runs identically on both
    // CI matrices. The classifier helpers exercised here are already covered
    // above; the mock verifies the query-write step and the read/stop loop,
    // not the classification helpers' internals.

    /// In-memory [`ProbeChannel`] that yields canned reply chunks and records
    /// the query bytes written so tests can assert both directions. The
    /// `read_err` / `wait_err` fields inject permanent channel failures used
    /// to pin the trait contract: when set, the next call returns `Err`
    /// instead of canned data, then clears so subsequent calls resume normal
    /// behaviour.
    #[derive(Default)]
    struct MockChannel {
        written: Vec<u8>,
        replies: Vec<Vec<u8>>,
        read_idx: usize,
        read_err: Option<io::Error>,
        wait_err: Option<io::Error>,
    }

    impl super::ProbeChannel for MockChannel {
        fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
            self.written.extend_from_slice(buf);
            Ok(())
        }

        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if let Some(e) = self.read_err.take() {
                return Err(e);
            }
            if self.read_idx >= self.replies.len() {
                return Ok(0);
            }
            let chunk = &self.replies[self.read_idx];
            let n = chunk.len().min(buf.len());
            buf[..n].copy_from_slice(&chunk[..n]);
            self.read_idx += 1;
            Ok(n)
        }

        // Mock has no real wait; "readable" iff a reply chunk remains
        // unconsumed, unless `wait_err` is set (then the trait contract
        // fires). The deadline argument is otherwise ignored.
        fn wait_until(&mut self, _deadline: Instant) -> io::Result<bool> {
            if let Some(e) = self.wait_err.take() {
                return Err(e);
            }
            Ok(self.read_idx < self.replies.len())
        }
    }

    #[test]
    fn probe_writes_three_queries() {
        let mut ch = MockChannel {
            replies: vec![b"\x1B[?1;0c".to_vec()],
            ..Default::default()
        };
        let _ = run_probe(&mut ch);
        assert_eq!(ch.written, b"\x1B[?u\x1B[>q\x1B[c");
    }

    #[test]
    fn probe_detects_kitty_flags_response() {
        let mut ch = MockChannel {
            replies: vec![b"\x1B[?0u\x1B[?62;22c".to_vec()],
            ..Default::default()
        };
        assert!(run_probe(&mut ch).unwrap());
    }

    #[test]
    fn probe_detects_xtversion_fallback() {
        // WezTerm XTVERSION but no kitty flags reply, terminated by DA1.
        let mut ch = MockChannel {
            replies: vec![b"\x1BP>|WezTerm 20240203\x1B\\\x1B[?65;22c".to_vec()],
            ..Default::default()
        };
        assert!(run_probe(&mut ch).unwrap());
    }

    #[test]
    fn probe_returns_false_for_unsupported_terminal() {
        // Only DA1, no kitty flags, no matching XTVERSION name.
        let mut ch = MockChannel {
            replies: vec![b"\x1B[?1;0c".to_vec()],
            ..Default::default()
        };
        assert!(!run_probe(&mut ch).unwrap());
    }

    #[test]
    fn probe_stops_after_da1_even_if_more_chunks_remain() {
        // DA1 arrives in the first chunk; a second "would-be-consumed" chunk
        // must NOT be read — DA1 terminates the loop.
        let mut ch = MockChannel {
            replies: vec![b"\x1B[?62;22c".to_vec(), b"\x1B[?0u".to_vec()],
            ..Default::default()
        };
        // No kitty flags before DA1 and DA1 carries no XTVERSION -> false.
        assert!(!run_probe(&mut ch).unwrap());
        // Exactly one read occurred — the loop did not consume the post-DA1 chunk.
        assert_eq!(ch.read_idx, 1);
    }

    #[test]
    fn probe_empty_replies_returns_false() {
        let mut ch = MockChannel::default();
        // `wait_until` reports "no input" -> loop breaks immediately -> false.
        assert!(!run_probe(&mut ch).unwrap());
    }

    // ── error / EOF paths (trait contract honesty) ───────────────────────────
    //
    // These pin the contract documented on `ProbeChannel` and `run_probe`:
    // an `Err` from `read` or `wait_until` is a permanent channel failure
    // and must propagate; `Ok(0)` (clean EOF) breaks the loop and reports
    // `false`. Without these, the production `wait_until`/`read` Err arms
    // would be untested dead code (CLAUDE.md test-validity rule).

    #[test]
    fn probe_propagates_read_error() {
        let mut ch = MockChannel {
            read_err: Some(io::Error::other("broken pipe")),
            // wait_until reports ready iff a chunk remains; supplying one
            // moves the loop past the wait step so the injected read Err
            // actually fires.
            replies: vec![b"x".to_vec()],
            ..Default::default()
        };
        let err = run_probe(&mut ch).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert_eq!(err.to_string(), "broken pipe");
    }

    #[test]
    fn probe_propagates_wait_until_error() {
        let mut ch = MockChannel {
            wait_err: Some(io::Error::other("poll failed")),
            ..Default::default()
        };
        let err = run_probe(&mut ch).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert_eq!(err.to_string(), "poll failed");
    }

    #[test]
    fn probe_eof_returns_false_not_error() {
        // Terminal closed the channel cleanly mid-probe (`read` returns
        // Ok(0) after wait reports ready): we must report "kitty
        // unavailable", not an error.
        let mut ch = MockChannel {
            replies: vec![Vec::new()],
            ..Default::default()
        };
        // Empty reply chunk → `read` returns Ok(0) on first call. wait_until
        // reports ready (a chunk remains at idx 0), then read sees an empty
        // chunk → n=0 not >0; the loop treats Ok(0) as EOF and breaks -> false.
        assert!(!run_probe(&mut ch).unwrap());
    }

    #[test]
    fn probe_accumulates_split_da1_across_reads() {
        // Kitty flags reply in chunk 0, DA1 split: '?' arrives in chunk 0 and
        // the 'c' terminator arrives in chunk 1. `has_da1_response` must fire
        // only after chunk 1, so the loop does exactly 2 reads and we still
        // detect kitty support from chunk 0.
        let mut ch = MockChannel {
            replies: vec![b"\x1B[?0u\x1B[?".to_vec(), b"62;22c".to_vec()],
            ..Default::default()
        };
        assert!(run_probe(&mut ch).unwrap());
        assert_eq!(ch.read_idx, 2);
    }

    // ── classify_probe_event (Windows event-based probe) ──────────────────────

    #[test]
    fn classify_kitty_report_flags_is_supported() {
        use termina::escape::csi::{Csi, Keyboard, KittyKeyboardFlags};

        let ev = termina::Event::Csi(Csi::Keyboard(Keyboard::ReportFlags(
            KittyKeyboardFlags::DISAMBIGUATE_ESCAPE_CODES,
        )));
        assert_eq!(classify_probe_event(&ev), Some(true));
    }

    #[test]
    fn classify_da1_fence_with_no_prior_report_is_unsupported() {
        use termina::escape::csi::{Csi, Device};

        let ev = termina::Event::Csi(Csi::Device(Device::DeviceAttributes(())));
        assert_eq!(classify_probe_event(&ev), Some(false));
    }

    #[test]
    fn classify_unrelated_event_keeps_waiting() {
        use termina::event::{KeyCode, KeyEvent, Modifiers};

        let ev = termina::Event::Key(KeyEvent::new(KeyCode::Char('x'), Modifiers::NONE));
        assert_eq!(classify_probe_event(&ev), None);
    }
}
