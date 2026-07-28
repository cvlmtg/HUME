//! External file-change detection (`Editor::check_buffer_disk_state` /
//! `check_all_disk_state`, the `autoread` setting, the `:w` stale guard, and
//! the confirm overlay's reload/keep choices).

use super::*;
use pretty_assertions::assert_eq;

/// Overwrite `path`'s content with something a different length than
/// whatever's there now, so the on-disk signature differs by size alone —
/// robust against a filesystem (HFS+, FAT) whose mtime only has one-second
/// resolution, where two writes in the same test could otherwise land on an
/// identical mtime.
fn rewrite_externally(path: &std::path::Path, content: &str) {
    std::fs::write(path, content).unwrap();
}

// ── Detection ─────────────────────────────────────────────────────────────────

/// An external rewrite with different content/length is detected as
/// `Changed`, and — since `autoread` defaults to `true` and this is the
/// focused buffer — opens a reload confirm.
#[test]
fn external_rewrite_is_detected_and_opens_confirm() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    rewrite_externally(&tmp, "hello, world!\n");

    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid);

    // Fail oracle: a mtime-only comparison could miss this on a filesystem
    // whose mtime resolution is coarser than the test's wall-clock delta.
    assert!(ed.doc().is_disk_stale());
    assert!(ed.state.config.confirm.is_some());
}

/// `autoread=false` only warns — no confirm, but the stale flag is still set
/// so `:w` still refuses until the buffer is reloaded or the write is forced.
#[test]
fn autoread_false_warns_without_opening_confirm() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    ed.state.settings.autoread = false;
    rewrite_externally(&tmp, "hello, world!\n");

    let (_, warnings_before) = ed.state.message_log.totals();
    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid);
    let (_, warnings_after) = ed.state.message_log.totals();

    assert_eq!(warnings_after, warnings_before + 1);
    assert!(ed.doc().is_disk_stale());
    assert!(ed.state.config.confirm.is_none());
}

/// A deleted file reads as `Vanished`: warns, marks the buffer stale, never
/// opens a confirm — there is nothing to reload from.
#[test]
fn deleted_file_warns_and_never_prompts() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let name = ed.doc().display_name();
    std::fs::remove_file(&tmp).unwrap();

    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid);

    assert!(ed.doc().is_disk_stale());
    assert!(ed.state.config.confirm.is_none());
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some(format!("{name}: file no longer exists on disk").as_str())
    );
}

/// A buffer with no backing file (scratch, or a synthetic view like
/// `[messages]`) has nothing to compare against and is always `Unchanged` —
/// checking it must never warn or mark the buffer stale.
#[test]
fn pathless_buffers_are_never_flagged() {
    let mut ed = editor_from("-[h]>ello\n"); // scratch buffer, no file_meta
    let bid = ed.focused_buffer_id();
    let (_, warnings_before) = ed.state.message_log.totals();

    ed.check_buffer_disk_state(bid);

    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(warnings_after, warnings_before);
    assert!(!ed.doc().is_disk_stale());
}

/// Once a change has been reported, a second check with nothing further
/// changed must stay silent — re-warning on every trigger (terminal focus,
/// buffer-enter) for a condition the user already saw once would be pure
/// noise. A *further*, distinct external change must still fire.
///
/// Fail oracle: if `check_buffer_disk_state` left the buffer's stored
/// signature untouched after reporting, this second check would compare
/// against the same stale signature and warn all over again.
#[test]
fn unactioned_change_does_not_refire_until_a_further_change_happens() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    ed.state.settings.autoread = false; // isolate the warning count from the confirm
    rewrite_externally(&tmp, "hello, world!\n");
    let bid = ed.focused_buffer_id();

    ed.check_buffer_disk_state(bid);
    let (_, warnings_after_first) = ed.state.message_log.totals();

    ed.check_buffer_disk_state(bid);
    let (_, warnings_after_second) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after_second, warnings_after_first,
        "an unactioned, unchanged-since-last-report disk state must stay silent"
    );

    rewrite_externally(&tmp, "a third, distinctly different revision\n");
    ed.check_buffer_disk_state(bid);
    let (_, warnings_after_third) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after_third,
        warnings_after_second + 1,
        "a genuinely new external change must still fire"
    );
}

// ── Writing the buffer must not look like an external change ─────────────────

/// The editor's own `:w` renames a fresh inode into place, which always
/// bumps mtime — without `write_file_atomic` refreshing the stored
/// signature after a successful write, the very next check would
/// misreport the editor's own save as an external change.
#[test]
fn writing_the_buffer_does_not_flag_it_as_externally_changed() {
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    // Dirty the buffer so `:w` actually writes instead of no-oping.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    type_cmd(&mut ed, ":w");

    let bid = ed.focused_buffer_id();
    let (_, warnings_before) = ed.state.message_log.totals();
    ed.check_buffer_disk_state(bid);
    let (_, warnings_after) = ed.state.message_log.totals();

    assert_eq!(warnings_after, warnings_before);
    assert!(!ed.doc().is_disk_stale());
    assert!(ed.state.config.confirm.is_none());
}

// ── Confirm overlay choices ───────────────────────────────────────────────────

/// The confirm's `[r]eload` choice re-reads the file, clears the stale flag,
/// and preserves the cursor position (line 0, col 0 maps to the same char
/// index regardless of what the line's new content is).
#[test]
fn confirm_reload_choice_reloads_and_clears_disk_stale() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    rewrite_externally(&tmp, "HELLO!!\n");
    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid);
    assert!(ed.state.config.confirm.is_some(), "setup: confirm must be open");

    ed.handle_key(key('r'));

    assert!(ed.state.config.confirm.is_none());
    assert_eq!(ed.doc().text().to_string(), "HELLO!!\n");
    assert_eq!(state(&ed), "-[H]>ELLO!!\n");
    assert!(!ed.doc().is_dirty());
    assert!(
        !ed.doc().is_disk_stale(),
        "Fail oracle: without clearing the stale flag on reload, :w would keep refusing forever"
    );
    assert!(
        ed.doc().can_undo(),
        "the reload must be a recorded revision, not a history reset"
    );
}

/// The confirm's `[k]eep` choice leaves the buffer's content untouched and
/// the stale flag still set — `:w` still refuses until reloaded or forced.
#[test]
fn confirm_keep_choice_leaves_buffer_untouched_and_still_stale() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    rewrite_externally(&tmp, "HELLO!!\n");
    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid);

    ed.handle_key(key('k'));

    assert!(ed.state.config.confirm.is_none());
    assert_eq!(ed.doc().text().to_string(), "hello\n");
    assert!(ed.doc().is_disk_stale());
}

/// Any key other than the accept key — not just the listed `k` — dismisses
/// without reloading. This is the documented safe default.
#[test]
fn confirm_any_other_key_dismisses_without_reloading() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    rewrite_externally(&tmp, "HELLO!!\n");
    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid);

    ed.handle_key(key_esc());

    assert!(ed.state.config.confirm.is_none());
    assert_eq!(ed.doc().text().to_string(), "hello\n");
    assert!(ed.doc().is_disk_stale());
}

// ── `:w` stale guard ───────────────────────────────────────────────────────────

/// `:w` on a buffer flagged stale refuses; `:w!` overwrites the external
/// change and clears the flag.
#[test]
fn write_refuses_on_stale_buffer_but_bang_overrides() {
    use crate::editor::buffer::DiskState;

    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    ed.doc_mut().disk_state = DiskState::Vanished;

    type_cmd(&mut ed, ":w");
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("file has changed on disk (add ! to override)")
    );
    assert_eq!(
        std::fs::read_to_string(&tmp).unwrap(),
        "hello\n",
        "the refused write must not have touched the file"
    );

    type_cmd(&mut ed, ":w!");
    assert!(!ed.doc().is_disk_stale());
    assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "xhello\n");
}
