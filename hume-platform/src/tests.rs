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
