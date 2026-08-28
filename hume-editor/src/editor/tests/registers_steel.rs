// End-to-end Steel coverage for `write-register!` / `read-register`.
//
// Distinct from `registers.rs`, which covers the `"<reg>` keymap prefix —
// this file covers the two Steel builtins that read/write the same store
// directly, independent of any yank/delete/paste keypress.

use super::*;
use hume_ops::register::{BLACK_HOLE_REGISTER, KILL_RING_REGISTER};
use hume_scripting::ScriptingHost;

/// `(write-register! "3" (list "hi"))` stores exactly the given list — the
/// same in-memory slot `"3y`/`"3p` read and write.
///
/// Fail oracle: reading the result back through `read-register` instead of
/// `ed.state.registers` directly would pass even if the builtin wrote
/// somewhere else that `read-register` happens to also read from.
#[test]
fn write_register_stores_text_for_a_named_register() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");

    eval_with_real_host(
        &mut ed,
        &mut ScriptingHost::new(),
        r#"(write-register! "3" (list "hi"))"#,
        tmp.path(),
    );

    assert_eq!(reg(&ed, '3'), &["hi"]);
}

/// Multiple entries — one per selection — round-trip in order.
#[test]
fn write_register_multi_value_preserves_order() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");

    eval_with_real_host(
        &mut ed,
        &mut ScriptingHost::new(),
        r#"(write-register! "3" (list "a" "b"))"#,
        tmp.path(),
    );

    assert_eq!(reg(&ed, '3'), &["a", "b"]);
}

/// `read-register` on a register written from the Rust side (not through
/// `write-register!`) returns the same list — proves the builtin reads the
/// real `RegisterSet`, not a private shadow copy.
#[test]
fn read_register_returns_text_written_from_rust() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");
    ed.state.registers.write_text('3', vec!["hi".to_string()]);

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (read-register "3") (list "hi"))"#,
    );
    assert!(
        fired,
        "read-register must return the register's real contents"
    );
}

/// `(write-register! "k" …)` must behave exactly like `"ky`: it goes through
/// `capture_to_ring`, which pushes the ring head *and* stamps `paste_stamp`
/// as one operation — a bare `p` right after must resume from the ring, not
/// fall through to the clipboard.
///
/// Fail oracle: writing straight to a hypothetical `'k'` slot in `RegisterSet`
/// (bypassing `capture_to_ring`) would leave `paste_stamp` unset, silently
/// breaking the following bare paste's smart-paste routing.
#[test]
fn write_register_k_pushes_ring_and_stamps_paste() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");

    eval_with_real_host(
        &mut ed,
        &mut ScriptingHost::new(),
        r#"(write-register! "k" (list "x"))"#,
        tmp.path(),
    );

    assert_eq!(
        ed.state.kill_ring.head(),
        Some(["x".to_string()].as_slice())
    );
    assert!(
        ed.state.paste_stamp.is_some(),
        "write-register! \"k\" must stamp paste_stamp, same as \"ky"
    );
}

/// `read-register` on `'k'` reads the ring head, round-tripping with the
/// write above.
#[test]
fn read_register_k_reads_ring_head() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(write-register! "k" (list "x"))
           (equal? (read-register "k") (list "x"))"#,
    );
    assert!(fired, "read-register \"k\" must read the kill-ring head");
}

/// The black hole discards writes and reads as empty, matching `"bd`/`"bp`.
#[test]
fn write_register_b_discards() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");

    eval_with_real_host(
        &mut ed,
        &mut ScriptingHost::new(),
        r#"(write-register! "b" (list "x"))"#,
        tmp.path(),
    );

    assert!(
        ed.state.registers.read(BLACK_HOLE_REGISTER).is_none(),
        "black-hole writes must be discarded"
    );
}

#[test]
fn read_register_b_returns_false() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(eq? (read-register "b") #f)"#,
    );
    assert!(fired, "read-register \"b\" must be #f");
}

/// An unwritten register reads as `#f`, same as the black hole.
#[test]
fn read_register_unwritten_returns_false() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(eq? (read-register "9") #f)"#,
    );
    assert!(fired, "an unwritten register must read as #f");
}

/// A register holding a recorded macro reads as `#f` — indistinguishable
/// from empty, since there is no wire format yet for a `Vec<KeyEvent>`. See
/// `registers.rs`'s (the builtins module) doc comment for the *why*.
#[test]
fn read_register_holding_a_macro_returns_false() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");
    ed.state.registers.write_macro('3', Vec::new());

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(eq? (read-register "3") #f)"#,
    );
    assert!(
        fired,
        "a macro register must read as #f, not raise or panic"
    );
}

/// The clipboard register round-trips through the in-process virtual
/// clipboard — real OS clipboard access is neither available nor desired in
/// a unit test.
#[test]
fn clipboard_register_round_trips_through_the_mock_clipboard() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");
    ed.state.clipboard = crate::editor::clipboard::SystemClipboard::new_mock();

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(write-register! "c" (list "hi"))
           (equal? (read-register "c") (list "hi"))"#,
    );
    assert!(
        fired,
        "the clipboard register must round-trip through write-register!/read-register"
    );
}

/// `write-register!`/`read-register` are `open`-gated — usable directly at
/// `init.scm` top level, unlike `set-register-prefix!` (`cmd`-gated, needs a
/// command body). `eval_with_real_host` runs `source` as `init.scm`; a
/// `cmd`-gated builtin here would raise "not available during init
/// evaluation" and fail the `.expect("eval_init")` inside it.
#[test]
fn callable_directly_from_init_scm() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");

    eval_with_real_host(
        &mut ed,
        &mut ScriptingHost::new(),
        r#"(write-register! "3" (list "init-value"))"#,
        tmp.path(),
    );

    assert_eq!(reg(&ed, '3'), &["init-value"]);
}

/// Sanity check that `KILL_RING_REGISTER`/`BLACK_HOLE_REGISTER` are the
/// constants this file's `"k"`/`"b"` literals mean — guards against the
/// string literals above silently drifting from the real register names if
/// they're ever renumbered.
#[test]
fn register_name_literals_match_the_constants() {
    assert_eq!(KILL_RING_REGISTER, 'k');
    assert_eq!(BLACK_HOLE_REGISTER, 'b');
}
