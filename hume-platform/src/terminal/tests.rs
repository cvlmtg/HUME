use super::{
    write_focus_disable, write_focus_enable, write_kitty_pop, write_kitty_push,
    write_mouse_disable, write_mouse_enable, write_paste_disable, write_paste_enable,
    write_unwind_escapes,
};
use std::io;

// Regression pins against termina's escape-sequence encoding: a version
// bump that silently changes these bytes would otherwise only surface as
// a terminal that stops responding to kitty/mouse/paste input.

#[test]
fn kitty_push_emits_expected_csi() {
    let mut buf = Vec::new();
    write_kitty_push(&mut buf).unwrap();
    // DISAMBIGUATE_ESCAPE_CODES(1) | REPORT_EVENT_TYPES(2)
    // | REPORT_ALTERNATE_KEYS(4) = 7. Spec:
    // https://sw.kovidgoyal.net/kitty/keyboard-protocol/#progressive-enhancement
    assert_eq!(buf, b"\x1b[>7u");
}

#[test]
fn kitty_pop_emits_expected_csi() {
    let mut buf = Vec::new();
    write_kitty_pop(&mut buf).unwrap();
    assert_eq!(buf, b"\x1b[<1u");
}

#[test]
fn paste_enable_emits_expected_csi() {
    let mut buf = Vec::new();
    write_paste_enable(&mut buf).unwrap();
    assert_eq!(buf, b"\x1b[?2004h");
}

#[test]
fn paste_disable_emits_expected_csi() {
    let mut buf = Vec::new();
    write_paste_disable(&mut buf).unwrap();
    assert_eq!(buf, b"\x1b[?2004l");
}

#[test]
fn focus_enable_emits_expected_csi() {
    let mut buf = Vec::new();
    write_focus_enable(&mut buf).unwrap();
    assert_eq!(buf, b"\x1b[?1004h");
}

#[test]
fn focus_disable_emits_expected_csi() {
    let mut buf = Vec::new();
    write_focus_disable(&mut buf).unwrap();
    assert_eq!(buf, b"\x1b[?1004l");
}

#[test]
fn mouse_enable_without_select_emits_1000_and_1006_only() {
    let mut buf = Vec::new();
    write_mouse_enable(&mut buf, false).unwrap();
    assert_eq!(buf, b"\x1b[?1000h\x1b[?1006h");
}

#[test]
fn mouse_enable_with_select_also_emits_1002() {
    let mut buf = Vec::new();
    write_mouse_enable(&mut buf, true).unwrap();
    assert_eq!(buf, b"\x1b[?1000h\x1b[?1006h\x1b[?1002h");
}

#[test]
fn mouse_disable_emits_1002_1000_1006_in_that_order() {
    let mut buf = Vec::new();
    write_mouse_disable(&mut buf).unwrap();
    assert_eq!(buf, b"\x1b[?1002l\x1b[?1000l\x1b[?1006l");
}

/// Fails the first write, then succeeds. Counts writes that reach past
/// the injected failure, to prove teardown keeps going after one step
/// errors rather than short-circuiting on the first `?`.
struct FailingWriter {
    failed_once: bool,
    writes_after_failure: usize,
}

impl io::Write for FailingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !self.failed_once {
            self.failed_once = true;
            return Err(io::Error::other("injected failure"));
        }
        self.writes_after_failure += 1;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn write_unwind_escapes_keeps_going_after_one_step_fails() {
    let mut out = FailingWriter {
        failed_once: false,
        writes_after_failure: 0,
    };
    let result = write_unwind_escapes(&mut out);
    assert!(result.is_err(), "first error must still be reported");
    assert!(
        out.writes_after_failure > 0,
        "remaining unwind steps must still run after an earlier one fails"
    );
}
