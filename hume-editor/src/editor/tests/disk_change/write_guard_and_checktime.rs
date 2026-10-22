//! The `:w` stale guard, `:checktime`, `:wa`, and `disk_state` resets.

use super::*;
use pretty_assertions::assert_eq;

// ── `:w` stale guard ───────────────────────────────────────────────────────────

/// `:w` refuses when the file on disk no longer matches what the buffer
/// last read or wrote: a fresh stat at write time, not a cached flag set by
/// some earlier trigger. `:w!` overrides.
///
/// No trigger (`check_buffer_disk_state`) runs anywhere in this test, so a
/// flag-based guard would see `disk_state == InSync` and let the write
/// through, clobbering the external change.
#[test]
fn write_refuses_on_externally_changed_file_but_bang_overrides() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    rewrite_externally(&tmp, "hello, externally changed!\n");
    assert!(
        !ed.doc().is_disk_stale(),
        "setup: no trigger has run, so the cached flag must still read clean"
    );

    type_cmd(&mut ed, ":w");
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("file has changed on disk (add ! to override)")
    );
    assert_eq!(
        std::fs::read_to_string(&tmp).unwrap(),
        "hello, externally changed!\n",
        "the refused write must not have touched the file"
    );

    type_cmd(&mut ed, ":w!");
    assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "xhello\n");
}

/// `:w` recreates a file that was deleted externally instead of refusing.
/// There is no external content to clobber, only the user's own unsaved
/// work to write back.
///
/// A guard that blocked on every stat error would refuse this write and
/// strand the user's edits.
#[test]
fn write_recreates_a_vanished_file() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    std::fs::remove_file(&tmp).unwrap();

    type_cmd(&mut ed, ":w");

    assert_eq!(
        std::fs::read_to_string(&tmp).unwrap(),
        "xhello\n",
        "a plain :w must recreate a vanished file, not refuse"
    );
    assert!(!focused_unsaved(&ed));
}

/// `:w %` (a save-as whose path resolves to the buffer's own file) must
/// refuse the same way a plain `:w` would: this is `:w` in disguise, not a
/// genuine save-as, so the stale-write guard still applies.
///
/// Opens the file via `:e` (not `editor_with_file`) so `ed.doc().path()` is
/// the same `fs::canonicalize`-d path `write_file` freshly reads back:
/// `editor_with_file` sets the buffer's path to the raw, uncanonicalized
/// tempfile path, which would make `targets_own_file` false on macOS
/// (`/var` → `/private/var`) and silently defeat this test.
#[test]
fn write_percent_refuses_on_externally_changed_own_file() {
    let (tmp, _tmp_guard) = temp_file("hello\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(tmp.to_str().unwrap())).unwrap();

    rewrite_externally(&tmp, "hello, externally changed!\n");

    type_cmd(&mut ed, ":w %");

    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("file has changed on disk (add ! to override)")
    );
    assert_eq!(
        std::fs::read_to_string(&tmp).unwrap(),
        "hello, externally changed!\n",
        "the refused write must not have touched the file"
    );
}

/// A genuine save-as to a path this buffer never read from must succeed even
/// though the buffer's *own* file changed on disk. `targets_own_file` is
/// false, so there is nothing to guard against on the new path.
///
/// A guard that only asked whether the buffer's own baseline changed, and
/// ignored which path is being written, would refuse this save-as too.
#[test]
fn save_as_to_unrelated_path_succeeds_despite_own_file_changing() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    rewrite_externally(&tmp, "hello, externally changed!\n");

    let (other, _other_guard) = temp_file("");
    type_cmd(&mut ed, &format!(":w {}", other.display()));

    assert_eq!(std::fs::read_to_string(&other).unwrap(), "hello\n");
    // Stored buffer paths are always `fs::canonicalize` output (see
    // `buffer_store.rs`'s note on this), so canonicalize the tempfile's own
    // path too so the comparison isn't tripped up by a macOS symlinked temp
    // dir (`/var` → `/private/var`).
    let other_canonical = std::fs::canonicalize(&other).unwrap();
    assert_eq!(
        ed.doc().path(),
        Some(other_canonical.as_path()),
        "save-as retargets the buffer"
    );
}

// ── `:checktime` ───────────────────────────────────────────────────────────────

/// `:checktime` is silent when nothing changed.
#[test]
fn checktime_is_silent_when_nothing_changed() {
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let (_, warnings_before) = ed.state.message_log.totals();

    type_cmd(&mut ed, ":checktime");

    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(warnings_after, warnings_before);
    assert!(ed.state.input.confirm().is_none());
}

/// `:checktime` runs the same check as any ambient trigger: it opens a
/// reload confirm for the focused, `file-reload-prompt`-on, externally-changed buffer
/// right now, instead of waiting for the next terminal-focus or
/// buffer-enter trigger.
#[test]
fn checktime_prompts_the_focused_changed_buffer() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    rewrite_externally(&tmp, "hello, externally changed!\n");

    type_cmd(&mut ed, ":checktime");

    assert!(ed.state.input.confirm().is_some());
}

/// `:checktime` warns (doesn't prompt) for a changed buffer that isn't
/// focused, the same off-focus rule as any other trigger.
#[test]
fn checktime_warns_for_a_changed_non_focused_buffer() {
    let (mut ed, _tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    assert_ne!(ed.focused_buffer_id(), bid_a);

    type_cmd(&mut ed, ":b #");
    assert_eq!(
        ed.focused_buffer_id(),
        bid_a,
        "setup: :b # must return to A"
    );

    rewrite_externally(&tmp_b, "world, externally changed!\n");
    let (_, warnings_before) = ed.state.message_log.totals();

    type_cmd(&mut ed, ":checktime");

    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(warnings_after, warnings_before + 1);
    assert!(ed.state.input.confirm().is_none());
}

// ── `:wa` stale-buffer skip ──────────────────────────────────────────────────────

/// `:wa` skips a buffer whose file changed on disk (leaving that file
/// untouched) while still writing every other dirty buffer, and reports
/// which ones were skipped. `:wa!` writes through all of them.
#[test]
fn write_all_skips_stale_buffer_but_writes_the_rest_bang_overrides() {
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();
    let name_a = ed.state.buffers.get(bid_a).display_name();
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    ed.handle_key(key('i'));
    ed.handle_key(key('y'));
    ed.handle_key(key_esc());

    // Change A's file only after both buffers are dirty, so :wa sees a
    // genuine race rather than a write that never should have happened.
    rewrite_externally(&tmp_a, "hello, externally changed!\n");

    type_cmd(&mut ed, ":wa");

    assert_eq!(
        std::fs::read_to_string(&tmp_a).unwrap(),
        "hello, externally changed!\n",
        "A must be skipped, not overwritten"
    );
    assert_eq!(std::fs::read_to_string(&tmp_b).unwrap(), "yworld\n");
    assert!(ed.state.has_unsaved_changes(bid_a), "A's write was skipped");
    assert!(!ed.state.has_unsaved_changes(bid_b));
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some(format!("Skipped (changed on disk): {name_a}").as_str())
    );

    type_cmd(&mut ed, ":wa!");

    assert_eq!(std::fs::read_to_string(&tmp_a).unwrap(), "xhello\n");
    assert!(!ed.state.has_unsaved_changes(bid_a));
}

// ── `disk_state` resets when the file matches its baseline again ─────────────────

/// A `disk_state` left at `Changed` must reset to `InSync` the moment a
/// fresh stat matches the buffer's read/write baseline again.
/// Otherwise a change that reverts and is later re-applied with the exact
/// signature already reported would silently fail to re-fire (the
/// `already_reported` comparison in the `Changed` arm would still find a
/// match). Injects the stale marker directly rather than depending on an
/// external rewrite landing back on the exact original mtime+size, which
/// isn't reliably reproducible from a test.
///
/// The reset happens in the `Unchanged` arm, which writes `InSync`.
#[test]
fn unchanged_check_resets_disk_state_to_in_sync() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid = ed.focused_buffer_id();

    let injected_sig = hume_platform::io::read_signature(&tmp).unwrap();
    ed.state.buffers.get_mut(bid).disk_state = DiskState::Changed(injected_sig);
    assert!(
        ed.doc().is_disk_stale(),
        "setup: disk_state must start Changed"
    );

    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);

    assert!(
        !ed.doc().is_disk_stale(),
        "a fresh stat matching the baseline must reset disk_state to InSync"
    );
}

/// `Indeterminate` (no backing file, or a stat error other than `NotFound`)
/// must never touch `disk_state`. Unlike a genuine `Unchanged` match, it
/// says nothing about whether the buffer is actually back in sync. A
/// pathless buffer is the deterministic way to reach `Indeterminate` from a
/// test (a transient stat error isn't reproducible portably).
#[test]
fn indeterminate_check_never_touches_disk_state() {
    let mut ed = editor_from("-[h]>ello\n"); // scratch buffer, no file_meta
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).disk_state = DiskState::Vanished;

    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);

    assert_eq!(
        ed.state.buffers.get(bid).disk_state,
        DiskState::Vanished,
        "Indeterminate must leave a prior disk_state untouched"
    );
}

/// A vanished file recreated with different content must still re-report.
/// Covers the transition out of `Vanished` alongside the `Unchanged` reset
/// above.
#[test]
fn vanished_file_recreated_with_different_content_rereports() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    ed.state.settings.file_reload_prompt = false; // isolate the warning count
    let path = tmp.to_path_buf();
    std::fs::remove_file(&tmp).unwrap();
    let bid = ed.focused_buffer_id();

    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after_vanish) = ed.state.message_log.totals();

    rewrite_externally(&path, "recreated with different content\n");
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after_recreate) = ed.state.message_log.totals();

    assert_eq!(
        warnings_after_recreate,
        warnings_after_vanish + 1,
        "recreating with different content must re-report after a Vanished warning"
    );
}
