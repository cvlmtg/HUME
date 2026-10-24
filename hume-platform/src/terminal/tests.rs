use super::{
    Frame, FrameFlag, ModeSwitch, write_cwd_report, write_focus_disable, write_focus_enable,
    write_kitty_pop, write_kitty_push, write_mouse_disable, write_mouse_enable,
    write_paste_disable, write_paste_enable, write_unwind_escapes,
};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

// Regression pins against termina's escape-sequence encoding: a version
// bump that silently changes these bytes would otherwise only surface as
// a terminal that stops responding to kitty/mouse/paste input.

const ENTER_ALT_SCREEN: &[u8] = b"\x1b[?1049h";
// DISAMBIGUATE_ESCAPE_CODES(1) | REPORT_EVENT_TYPES(2)
// | REPORT_ALTERNATE_KEYS(4) = 7. Spec:
// https://sw.kovidgoyal.net/kitty/keyboard-protocol/#progressive-enhancement
const KITTY_PUSH: &[u8] = b"\x1b[>7u";
const MOUSE_ENABLE: &[u8] = b"\x1b[?1000h\x1b[?1006h";
const FOCUS_DISABLE: &[u8] = b"\x1b[?1004l";
const FOCUS_ENABLE: &[u8] = b"\x1b[?1004h";
const PASTE_ENABLE: &[u8] = b"\x1b[?2004h";

#[test]
fn kitty_push_emits_expected_csi() {
    let mut buf = Vec::new();
    write_kitty_push(&mut buf).unwrap();
    assert_eq!(buf, KITTY_PUSH);
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
    assert_eq!(buf, PASTE_ENABLE);
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
    assert_eq!(buf, FOCUS_ENABLE);
}

#[test]
fn focus_disable_emits_expected_csi() {
    let mut buf = Vec::new();
    write_focus_disable(&mut buf).unwrap();
    assert_eq!(buf, FOCUS_DISABLE);
}

#[test]
fn mouse_enable_without_select_emits_1000_and_1006_only() {
    let mut buf = Vec::new();
    write_mouse_enable(&mut buf, false).unwrap();
    assert_eq!(buf, MOUSE_ENABLE);
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

#[test]
fn write_unwind_escapes_keeps_going_after_one_step_fails() {
    let mut term = FakeTerm {
        failing_writes: 1,
        ..FakeTerm::default()
    };
    let result = write_unwind_escapes(&mut term, None);
    assert!(result.is_err(), "first error must still be reported");
    assert!(
        !term.out.is_empty(),
        "remaining unwind steps must still run after an earlier one fails"
    );
}

#[test]
fn write_cwd_report_emits_the_url_or_an_empty_one() {
    for (url, expected) in [
        (Some("file://box/tmp"), &b"\x1b]7;file://box/tmp\x1b\\"[..]),
        (None, &b"\x1b]7;\x1b\\"[..]),
    ] {
        let mut out = Vec::new();
        write_cwd_report(&mut out, url).expect("writing to a Vec cannot fail");
        assert_eq!(out, expected);
    }
}

#[test]
fn write_unwind_escapes_reports_the_launch_directory() {
    let mut out = Vec::new();
    write_unwind_escapes(&mut out, Some("file://box/tmp")).expect("writing to a Vec cannot fail");
    let bytes = String::from_utf8(out).expect("escapes are ASCII");
    assert!(
        bytes.contains("\x1b]7;file://box/tmp\x1b\\"),
        "teardown must report the launch directory, got {bytes:?}"
    );
}

#[test]
fn frame_flag_is_raised_for_its_own_lifetime() {
    let flag = Arc::new(AtomicBool::new(false));
    let raised = FrameFlag::try_raise(&flag).expect("the flag starts lowered");
    assert!(flag.load(Ordering::Acquire));
    assert!(FrameFlag::try_raise(&flag).is_none());
    drop(raised);
    assert!(!flag.load(Ordering::Acquire));
    assert!(FrameFlag::try_raise(&flag).is_some());
}

/// A terminal that records what reaches it and fails the steps a test asks
/// it to. `fail_next_after` fails the first write once `out` contains those
/// bytes, then disarms. `flag_at_raw` is the frame flag's value when raw
/// mode was entered.
#[derive(Default)]
struct FakeTerm {
    out: Vec<u8>,
    failing_writes: usize,
    fail_next_after: Option<&'static [u8]>,
    fail_cooked: bool,
    fail_raw: bool,
    flag: Arc<AtomicBool>,
    flag_at_raw: Option<bool>,
}

impl io::Write for FakeTerm {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.failing_writes > 0 {
            self.failing_writes -= 1;
            return Err(io::Error::other("injected write failure"));
        }
        if let Some(marker) = self.fail_next_after
            && contains(&self.out, marker)
        {
            self.fail_next_after = None;
            return Err(io::Error::other("injected write failure"));
        }
        self.out.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl ModeSwitch for &mut FakeTerm {
    fn raw_mode(&mut self) -> io::Result<()> {
        self.flag_at_raw = Some(self.flag.load(Ordering::Acquire));
        if self.fail_raw {
            return Err(io::Error::other("injected raw-mode failure"));
        }
        Ok(())
    }

    fn cooked_mode(&mut self) -> io::Result<()> {
        if self.fail_cooked {
            return Err(io::Error::other("injected cooked-mode failure"));
        }
        Ok(())
    }
}

fn open_frame(term: &mut FakeTerm) -> io::Result<Frame<&mut FakeTerm>> {
    let flag = Arc::clone(&term.flag);
    Frame::open(term, &flag, true, true, false)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    find(haystack, needle).is_some()
}

#[test]
fn failed_cooked_switch_turns_back_on_every_mode_but_the_alt_screen() {
    let mut term = FakeTerm {
        fail_cooked: true,
        ..FakeTerm::default()
    };
    assert!(open_frame(&mut term).is_err());
    assert!(!term.flag.load(Ordering::Acquire));
    assert_eq!(term.flag_at_raw, Some(true), "raw mode is entered again");
    for enable in [FOCUS_ENABLE, PASTE_ENABLE, KITTY_PUSH, MOUSE_ENABLE] {
        assert!(contains(&term.out, enable), "{enable:?} in {:?}", term.out);
    }
    assert!(!contains(&term.out, ENTER_ALT_SCREEN), "{:?}", term.out);
}

#[test]
fn failed_frame_open_turns_back_on_only_what_it_turned_off() {
    let mut term = FakeTerm {
        fail_next_after: Some(FOCUS_DISABLE),
        ..FakeTerm::default()
    };
    assert!(open_frame(&mut term).is_err());
    assert!(!term.flag.load(Ordering::Acquire));
    assert_eq!(term.flag_at_raw, None, "the cooked switch was never tried");
    let failure = find(&term.out, FOCUS_DISABLE).expect("the focus disable is written");
    let after = &term.out[failure + FOCUS_DISABLE.len()..];
    for enable in [FOCUS_ENABLE, PASTE_ENABLE] {
        assert!(contains(after, enable), "{enable:?} in {after:?}");
    }
    for untouched in [KITTY_PUSH, MOUSE_ENABLE, ENTER_ALT_SCREEN] {
        assert!(!contains(after, untouched), "{untouched:?} in {after:?}");
    }
}

#[test]
fn frame_close_restores_under_the_flag_and_lowers_it_even_when_restore_fails() {
    let mut term = FakeTerm::default();
    let frame = open_frame(&mut term).expect("the fake terminal opens");
    frame.term.fail_raw = true;
    assert!(frame.close().is_err());
    assert_eq!(term.flag_at_raw, Some(true));
    assert!(!term.flag.load(Ordering::Acquire));
}

#[test]
fn frame_restore_keeps_going_after_one_step_fails() {
    let mut term = FakeTerm::default();
    let frame = open_frame(&mut term).expect("the fake terminal opens");
    frame.term.out.clear();
    frame.term.failing_writes = 1;
    assert!(frame.close().is_err(), "the first error is still reported");
    assert!(contains(&term.out, KITTY_PUSH), "{:?}", term.out);
    assert!(contains(&term.out, MOUSE_ENABLE), "{:?}", term.out);
}

#[test]
fn frame_open_is_refused_while_a_frame_is_open() {
    let flag = Arc::new(AtomicBool::new(false));
    let mut first_term = FakeTerm::default();
    let first = Frame::open(&mut first_term, &flag, true, true, false);
    assert!(first.is_ok());

    let mut second_term = FakeTerm::default();
    assert!(Frame::open(&mut second_term, &flag, true, true, false).is_err());
    assert!(second_term.out.is_empty(), "{:?}", second_term.out);
    assert!(
        flag.load(Ordering::Acquire),
        "the first frame keeps the flag"
    );

    drop(first);
    let mut third_term = FakeTerm::default();
    assert!(Frame::open(&mut third_term, &flag, true, true, false).is_ok());
}
