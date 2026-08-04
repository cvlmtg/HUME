//! External file-change detection (`Editor::check_buffer_disk_state` /
//! `check_all_disk_state`, the `autoread` setting, the `:w` stale guard, and
//! the confirm overlay's reload/keep choices).

use super::*;
use crate::editor::buffer::{DiskCheckTrigger, DiskState};
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
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);

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
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
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
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);

    assert!(ed.doc().is_disk_stale());
    assert!(ed.state.config.confirm.is_none());
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some(format!("{name}: file no longer exists on disk").as_str())
    );
}

/// A deleted file warns once; a second check with the file still gone must
/// stay silent — same "don't nag again for the same thing" rule `Changed`
/// follows, just with no signature to compare (there's only one "vanished").
///
/// Fail oracle: if the `Vanished` arm never recorded that it had already
/// reported, every later ambient trigger (each terminal focus regain) would
/// warn all over again, forever.
#[test]
fn vanished_file_does_not_refire_on_every_trigger() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    std::fs::remove_file(&tmp).unwrap();
    let bid = ed.focused_buffer_id();

    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after_first) = ed.state.message_log.totals();

    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after_second) = ed.state.message_log.totals();

    assert_eq!(
        warnings_after_second, warnings_after_first,
        "a vanished file already reported must stay silent on a later check"
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

    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);

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

    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after_first) = ed.state.message_log.totals();

    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after_second) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after_second, warnings_after_first,
        "an unactioned, unchanged-since-last-report disk state must stay silent"
    );

    rewrite_externally(&tmp, "a third, distinctly different revision\n");
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after_third) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after_third,
        warnings_after_second + 1,
        "a genuinely new external change must still fire"
    );
}

/// `:bn`/`:bp` run the same buffer-enter disk check as `:e`/`:b` — landing on
/// an externally-changed buffer this way must not silently show stale
/// content with `:w` free to clobber the external edit.
///
/// Fail oracle: before `enter_buffer_with_jump` wired the check into these
/// commands, `:bnext`/`:bprev` called `switch_to_buffer_with_jump` directly
/// with no check at all, so the target buffer's disk state stayed `InSync`
/// no matter what changed externally.
#[test]
fn bnext_and_bprev_run_the_buffer_enter_disk_check() {
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: :e must open a distinct second buffer");

    // B is focused; rewrite A externally.
    rewrite_externally(&tmp_a, "hello, externally changed!\n");

    // With only two buffers open, either direction lands back on A.
    type_cmd(&mut ed, ":bp");
    assert_eq!(ed.focused_buffer_id(), bid_a);
    assert!(
        ed.state.config.confirm.is_some(),
        "Fail oracle: :bp must run the buffer-enter disk check, not just switch"
    );
}

/// A non-focused buffer's external change only warns — no confirm opens
/// off-focus. But switching into that buffer afterwards must still open the
/// confirm, even though nothing has changed on disk since the warning: this
/// is the documented "asked about on its own next buffer-enter" promise.
///
/// Fail oracle: if `BufferEnter` deduped an already-reported `Changed` state
/// exactly like `Ambient` does, `:b #` landing on the buffer would find
/// nothing new to report and stay silent — silently breaking the promise.
#[test]
fn deferred_change_on_non_focused_buffer_prompts_on_buffer_enter() {
    let (mut ed, _tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: :e must open a distinct second buffer");

    // Switch back to A — B stays open as the alternate, unfocused.
    type_cmd(&mut ed, ":b #");
    assert_eq!(
        ed.focused_buffer_id(),
        bid_a,
        "setup: :b # must return to A"
    );

    rewrite_externally(&tmp_b, "world, externally changed!\n");
    let (_, warnings_before) = ed.state.message_log.totals();
    ed.check_buffer_disk_state(bid_b, DiskCheckTrigger::Ambient);
    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "a non-focused change only warns"
    );
    assert!(ed.state.config.confirm.is_none());

    // Enter B via :b — the deferred prompt must appear now.
    type_cmd(&mut ed, ":b #");
    assert_eq!(ed.focused_buffer_id(), bid_b);
    assert!(
        ed.state.config.confirm.is_some(),
        "buffer-enter must prompt a pending change even if already warned about"
    );
}

/// A pending change detected while the editor is in Insert never opens a
/// confirm — it would steal the very next keystroke from whatever the user
/// is mid-typing. It warns instead, same as a non-focused buffer or
/// `autoread` off — and, like that non-focused case, only a `BufferEnter`
/// check reopens the deferred prompt; a further `Ambient` recheck stays
/// silent for the same already-reported state.
///
/// Fail oracle: if the confirm ignored mode entirely, the first assertion
/// below would find a confirm open while `ed.state.mode` is `Insert`.
#[test]
fn change_detected_mid_insert_warns_instead_of_prompting() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    ed.state.mode = Mode::Insert;
    rewrite_externally(&tmp, "hello, externally changed!\n");

    let (_, warnings_before) = ed.state.message_log.totals();
    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after) = ed.state.message_log.totals();

    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "must warn instead of prompting"
    );
    assert!(ed.state.config.confirm.is_none());

    // Back in Normal, only a buffer-enter check reopens the deferred prompt
    // — same deferral rule as a non-focused buffer's warning (see
    // `deferred_change_on_non_focused_buffer_prompts_on_buffer_enter`).
    ed.state.mode = Mode::Normal;
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::BufferEnter);
    assert!(
        ed.state.config.confirm.is_some(),
        "a mode-blocked Changed must still prompt on the next buffer-enter"
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
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
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
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.config.confirm.is_some(),
        "setup: confirm must be open"
    );

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
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);

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
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);

    ed.handle_key(key_esc());

    assert!(ed.state.config.confirm.is_none());
    assert_eq!(ed.doc().text().to_string(), "hello\n");
    assert!(ed.doc().is_disk_stale());
}

/// Accepting the confirm after focus moved away from the target buffer
/// (an async Steel/LSP callback calling `switch-to-buffer!` between frames —
/// not through key dispatch, since the confirm intercept consumes every key
/// while open) must not reload, and must not panic.
///
/// Fail oracle: without the focus guard, `reload_buffer_from_disk` would
/// call `reload_buffer_in_place(bid_a, ..)` while focus is on B, and its
/// `.expect("focused pane must view the reloaded buffer")` would panic —
/// there is no pane state for A's post-heads at the now-focused pane.
#[test]
fn reload_confirm_accept_after_focus_moved_away_does_not_panic() {
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();
    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.config.confirm.is_some(),
        "setup: confirm must be open"
    );

    // Simulate an async callback moving focus without going through key
    // dispatch — the confirm is left open, still targeting A.
    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    let (bid_b, _) = ed.resolve_open_path(&tmp_b.display().to_string()).unwrap();
    ed.switch_to_buffer_without_jump(bid_b);
    assert_ne!(
        ed.focused_buffer_id(),
        bid_a,
        "setup: focus must have moved off A"
    );

    let (_, warnings_before) = ed.state.message_log.totals();
    ed.handle_key(key('r'));
    let (_, warnings_after) = ed.state.message_log.totals();

    assert!(
        ed.state.config.confirm.is_none(),
        "confirm must still close"
    );
    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "must warn instead of reloading"
    );
    assert_eq!(ed.focused_buffer_id(), bid_b, "focus must stay put");
    assert_eq!(
        ed.state.buffers.get(bid_a).text().to_string(),
        "hello\n",
        "A must not have been reloaded while unfocused"
    );
}

// ── `:w` stale guard ───────────────────────────────────────────────────────────

/// `:w` refuses when the file on disk no longer matches what the buffer
/// last read or wrote — a fresh stat at write time, not a cached flag set by
/// some earlier trigger. `:w!` overrides.
///
/// Fail oracle: no trigger (`check_buffer_disk_state`) runs anywhere in this
/// test, so a flag-based guard would see `disk_state == InSync` and let the
/// write through, silently clobbering the external change.
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

/// `:w` recreates a file that was deleted externally instead of refusing —
/// there is no external content to clobber, only the user's own unsaved
/// work to write back.
///
/// Fail oracle: a guard that blocked on any stat error (not just a genuine
/// content mismatch) would refuse this write and strand the user's edits.
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
    assert!(!ed.doc().is_dirty());
}

/// `:w %` — a save-as whose path resolves to the buffer's own file — must
/// refuse the same way a plain `:w` would: this is `:w` in disguise, not a
/// genuine save-as, so the stale-write guard still applies.
///
/// Opens the file via `:e` (not `editor_with_file`) so `ed.doc().path()` is
/// the same `fs::canonicalize`-d path `write_file` freshly reads back —
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
/// though the buffer's *own* file changed on disk — `targets_own_file` is
/// false, so there is nothing to guard against on the new path.
///
/// Fail oracle: a guard keyed only on "did the buffer's own baseline
/// change" (ignoring which path is actually being written) would refuse
/// this save-as too, even though it targets an unrelated file.
#[test]
fn save_as_to_unrelated_path_succeeds_despite_own_file_changing() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    rewrite_externally(&tmp, "hello, externally changed!\n");

    let (other, _other_guard) = temp_file("");
    type_cmd(&mut ed, &format!(":w {}", other.display()));

    assert_eq!(std::fs::read_to_string(&other).unwrap(), "hello\n");
    // Stored buffer paths are always `fs::canonicalize` output (see
    // `buffer_store.rs`'s note on this) — canonicalize the tempfile's own
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
    assert!(ed.state.config.confirm.is_none());
}

/// `:checktime` runs the same check as any ambient trigger — it opens a
/// reload confirm for the focused, `autoread`-on, externally-changed buffer
/// right now, instead of waiting for the next terminal-focus or
/// buffer-enter trigger.
#[test]
fn checktime_prompts_the_focused_changed_buffer() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    rewrite_externally(&tmp, "hello, externally changed!\n");

    type_cmd(&mut ed, ":checktime");

    assert!(ed.state.config.confirm.is_some());
}

/// `:checktime` warns (doesn't prompt) for a changed buffer that isn't
/// focused — same off-focus rule as any other trigger.
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
    assert!(ed.state.config.confirm.is_none());
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
    assert!(
        ed.state.buffers.get(bid_a).is_dirty(),
        "A's write was skipped"
    );
    assert!(!ed.state.buffers.get(bid_b).is_dirty());
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some(format!("Skipped (changed on disk): {name_a}").as_str())
    );

    type_cmd(&mut ed, ":wa!");

    assert_eq!(std::fs::read_to_string(&tmp_a).unwrap(), "xhello\n");
    assert!(!ed.state.buffers.get(bid_a).is_dirty());
}

// ── `disk_state` resets when the file matches its baseline again ─────────────────

/// A `disk_state` left at `Changed` must reset to `InSync` the moment a
/// fresh stat genuinely matches the buffer's read/write baseline again —
/// otherwise a change that reverts and is later re-applied with the exact
/// signature already reported would silently fail to re-fire (the
/// `already_reported` comparison in the `Changed` arm would still find a
/// match). Injects the stale marker directly rather than depending on an
/// external rewrite landing back on the exact original mtime+size, which
/// isn't reliably reproducible from a test.
///
/// Fail oracle: without the `Unchanged` arm writing `InSync`, `disk_state`
/// would stay at the injected `Changed` value even though the file already
/// matches its baseline, and the second assertion would fail.
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
/// must never touch `disk_state` — unlike a genuine `Unchanged` match, it
/// says nothing about whether the buffer is actually back in sync. A
/// pathless buffer is the deterministic way to reach `Indeterminate` from a
/// test (a transient stat error isn't reproducible portably).
///
/// Fail oracle: if `Indeterminate` reset `disk_state` to `InSync` the same
/// way `Unchanged` does, the assertion below would fail.
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

/// A vanished file recreated with different content must still re-report —
/// covers the transition out of `Vanished` alongside the `Unchanged` reset
/// above.
#[test]
fn vanished_file_recreated_with_different_content_rereports() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    ed.state.settings.autoread = false; // isolate the warning count
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

// ── Confirm can't open over another overlay or a pending key sequence ────────────

/// A confirm must never open while a picker is on screen — the confirm
/// intercept sits above the picker (`mappings/mod.rs`), so it would eat
/// every key the picker needs. The blocked change only warns, and the
/// deferred prompt still arrives on the next buffer-enter after the picker
/// closes, same deferral rule as a mode-blocked or non-focused change.
///
/// Fail oracle: without the picker check in `can_open_confirm`, the first
/// assertion below would find a confirm open while the picker is still
/// live.
#[test]
fn confirm_does_not_open_over_a_live_picker_but_defers_to_next_buffer_enter() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid = ed.focused_buffer_id();
    rewrite_externally(&tmp, "hello, externally changed!\n");

    let session = crate::editor::picker::PickerSession::new(
        steel::rvals::SteelVal::BoolV(false),
        String::new(),
        false,
    );
    crate::editor::picker::open_picker(&mut ed.state, Some(&mut ed.lsp), session);

    let (_, warnings_before) = ed.state.message_log.totals();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after) = ed.state.message_log.totals();

    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "must warn instead of prompting"
    );
    assert!(ed.state.config.confirm.is_none());
    assert!(
        ed.state.config.picker.is_some(),
        "the picker must stay open"
    );

    ed.state.config.picker = None;
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::BufferEnter);
    assert!(
        ed.state.config.confirm.is_some(),
        "the deferred prompt must arrive once the picker is gone"
    );
}

/// A confirm must never open while `pending_keys` is non-empty — a live
/// multi-key sequence (e.g. `d` waiting for its motion) already owns the
/// very next keystroke.
///
/// Fail oracle: without the `pending_keys` check in `can_open_confirm`, the
/// confirm would open and the assertion below would fail.
#[test]
fn confirm_does_not_open_mid_pending_key_sequence() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid = ed.focused_buffer_id();
    rewrite_externally(&tmp, "hello, externally changed!\n");
    ed.state.pending_keys.push(key('g'));

    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);

    assert!(ed.state.config.confirm.is_none());
}

// ── `check_focus_change_disk_state`: closing/cycling/clicking reveals a
//    buffer with a deferred change ──────────────────────────────────────────
//
// `enter_buffer_with_jump` only covers `:e`/`:b`/`:bn`/`:bp`. Every other way
// the focused pane can land on a different buffer — closing a buffer or a
// pane, cycling pane focus, clicking into another pane — runs through
// `Editor::handle_event`'s tail check instead, since the commands behind
// those are `EditorCmdFn`-shaped and cannot call `Editor` methods themselves.
// These tests drive input via `feed_event`/`type_cmd_event` (routed through
// `handle_event`), not the usual `feed_key`/`type_cmd`, since that is the
// one boundary the new check runs at.

/// Two panes side by side: the left pane keeps viewing A, the right
/// (focused) pane is retargeted onto B. `:vsplit` with no argument inherits
/// the source pane's buffer, so `:e <tmp_b>` on the new pane is what actually
/// splits the two buffers apart.
fn two_panes_with_b_focused() -> (
    Editor,
    tempfile::TempPath,
    tempfile::TempPath,
    BufferId,
    BufferId,
) {
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();
    type_cmd(&mut ed, ":vsplit");
    let (tmp_b, tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: :e must open a distinct second buffer");
    (ed, tmp_a, tmp_b_guard, bid_a, bid_b)
}

/// The reported repro: A and B both change on disk; B's confirm is answered
/// (implicitly, by the test just moving on); `:q` closes the focused buffer
/// (B) and reveals A, which must re-prompt for its own already-warned
/// change.
///
/// Fail oracle: before the `handle_event`-tail check, `:q` → `Editor::
/// close_buffer` → `lifecycle::close_buffer` → `switch_pane_to_buffer` moved
/// the focused pane onto A with no `BufferEnter` check anywhere on the path,
/// so A's already-warned `Changed` state was never re-surfaced and the
/// statusline fell back to the still-unseen log summary. This is also the
/// only test in this section that pins the *trigger*: an `Ambient` recheck
/// (the second assertion) must stay silent for an already-reported change.
#[test]
fn quit_closing_a_buffer_prompts_the_revealed_one() {
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: :e must open a distinct second buffer");

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    let (_, warnings_before) = ed.state.message_log.totals();
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "setup: A only warns while B is focused"
    );
    assert!(
        ed.state.config.confirm.is_none(),
        "setup: A is not focused, so no confirm yet"
    );

    type_cmd_event(&mut ed, ":q");

    assert_eq!(
        ed.focused_buffer_id(),
        bid_a,
        "setup: closing B must reveal A, the only other real buffer"
    );
    assert!(
        ed.state.config.confirm.is_some(),
        "landing on A via :q must re-open its deferred reload confirm"
    );
}

/// `:bd` reaches the reveal through the same `Editor::close_buffer` as `:q`,
/// but is a distinct command — a fix wired only into `typed_quit` would
/// leave this silent.
#[test]
fn buffer_delete_prompts_the_revealed_buffer() {
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: :e must open a distinct second buffer");

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(ed.state.config.confirm.is_none(), "setup: A is not focused");

    type_cmd_event(&mut ed, ":bd");

    assert_eq!(ed.focused_buffer_id(), bid_a);
    assert!(
        ed.state.config.confirm.is_some(),
        ":bd must run the same buffer-enter check as :q"
    );
}

/// Multi-pane `:q` closes the focused pane and reveals its sibling, which
/// must re-prompt for its own deferred change.
///
/// Fail oracle: `close_focused_pane` takes `(&mut EditorState, &mut
/// EngineView)` and cannot call an `Editor` method — a fix that only patched
/// `Editor::close_buffer` would leave multi-pane `:q` silent while
/// single-pane `:q` (`quit_closing_a_buffer_prompts_the_revealed_one`)
/// prompted correctly.
#[test]
fn multi_pane_quit_prompts_the_surviving_panes_buffer() {
    let (mut ed, tmp_a, _tmp_b_guard, bid_a, _bid_b) = two_panes_with_b_focused();

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.config.confirm.is_none(),
        "setup: A is not focused (right pane shows B)"
    );

    type_cmd_event(&mut ed, ":q");

    assert_eq!(
        ed.view.panes.len(),
        1,
        "setup: multi-pane :q must close the focused pane, not quit"
    );
    assert_eq!(ed.focused_buffer_id(), bid_a);
    assert!(
        ed.state.config.confirm.is_some(),
        "the surviving pane landing on A must re-open its deferred confirm"
    );
}

/// `Ctrl+p c` (`pane-close`) reaches the same reveal as multi-pane `:q`, but
/// as a keymap `EditorCmd` with no `&mut Editor` at all to call the check on
/// — pins that the post-dispatch chokepoint covers a keymap command with no
/// per-command plumbing. Also pins that `pending_keys` is cleared before the
/// `c` leaf runs (`mappings/normal.rs`'s Leaf arm), so `can_open_confirm`
/// isn't blocked by the still-just-consumed `Ctrl+p` prefix.
#[test]
fn ctrl_p_c_pane_close_prompts_the_surviving_panes_buffer() {
    let (mut ed, tmp_a, _tmp_b_guard, bid_a, _bid_b) = two_panes_with_b_focused();

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(ed.state.config.confirm.is_none(), "setup: A is not focused");

    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('c'));

    assert_eq!(
        ed.view.panes.len(),
        1,
        "setup: Ctrl+p c must close the pane"
    );
    assert_eq!(ed.focused_buffer_id(), bid_a);
    assert!(
        ed.state.config.confirm.is_some(),
        "landing on A via pane-close must re-open its deferred confirm"
    );
}

/// Cycling pane focus (`Ctrl+p p`, `pane-focus-next`) is a bare
/// `state.focused_pane_id = …` assignment (`commands/jump.rs`) — without the
/// check, cycling onto a pane showing an externally-changed file would show
/// stale content with `:w` free to clobber the external edit.
#[test]
fn pane_focus_cycling_prompts_the_buffer_it_lands_on() {
    let (mut ed, tmp_a, _tmp_b_guard, bid_a, bid_b) = two_panes_with_b_focused();
    assert_eq!(ed.focused_buffer_id(), bid_b, "setup: right pane focused");

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(ed.state.config.confirm.is_none(), "setup: A is not focused");

    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('p'));

    assert_eq!(
        ed.focused_buffer_id(),
        bid_a,
        "setup: with only two panes, cycling lands on the other one"
    );
    assert!(
        ed.state.config.confirm.is_some(),
        "landing on A via pane-focus-next must re-open its deferred confirm"
    );
}

/// A click into another pane (`mouse_left_down`'s click-to-focus) is the
/// same bare `focused_pane_id` assignment and never touches `handle_key` at
/// all — a chokepoint placed only in `handle_key`/`handle_mouse` would have
/// to duplicate itself to cover this; `handle_event` covers both for free.
#[test]
fn clicking_into_another_pane_prompts_that_panes_buffer() {
    let (mut ed, tmp_a, _tmp_b_guard, bid_a, bid_b) = two_panes_with_b_focused();
    assert_eq!(ed.focused_buffer_id(), bid_b, "setup: right pane focused");

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(ed.state.config.confirm.is_none(), "setup: A is not focused");

    // 100×25, 0.5 split: pane A (left) spans x ∈ [0, 49); col 0 lands inside
    // it regardless of gutter width, since click-to-focus happens before
    // char-offset resolution (`mouse_left_down`, `mouse.rs`).
    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.prepare_frame(100, 25, &mut ctx);
    ed.handle_event(mouse_left_down(0, 0));

    assert_eq!(ed.focused_buffer_id(), bid_a);
    assert!(
        ed.state.config.confirm.is_some(),
        "clicking into A's pane must re-open its deferred confirm"
    );
}

// ── Confirm hygiene: a closed buffer's confirm, and one confirm at a time ────

/// Closing a buffer that an open confirm targets must retire the confirm —
/// otherwise the prompt outlives its subject and answering `[r]eload` would
/// silently no-op via `reload_buffer_from_disk`'s `try_get` bail. Mirrors a
/// non-interactive close (Steel's `close-buffer!`), which never routes
/// through a key, so the confirm intercept can't dismiss it for us.
///
/// Fail oracle: without `close_buffer_and_notify` retiring a confirm that
/// `targets_buffer(id)`, the assertion below would find the stale confirm
/// still present — and with the `confirm.is_none()` guard in place, it would
/// additionally block every later prompt until some stray key dismissed it.
#[test]
fn closing_a_buffer_retires_its_open_reload_confirm() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid = ed.focused_buffer_id();
    rewrite_externally(&tmp, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.config.confirm.is_some(),
        "setup: confirm must be open"
    );

    crate::editor::buffer::lifecycle::close_buffer_and_notify(
        &mut ed.view,
        &mut ed.state,
        Some(&mut ed.lsp),
        bid,
    );

    assert!(
        ed.state.config.confirm.is_none(),
        "closing the confirm's target buffer must retire the confirm"
    );
}

/// A confirm already open for one buffer must not be replaced by a second
/// buffer's deferred prompt — the user would answer a question they never
/// saw. B's deferred change stays a warning instead, same as any other
/// blocked-confirm case.
///
/// Fail oracle: without `confirm.is_none()` in `can_open_confirm`, the
/// second `open_disk_change_confirm` call would replace the live model with
/// B's, and the first assertion below would fail.
#[test]
fn a_second_confirm_never_replaces_a_live_one() {
    // Both buffers opened up front: once A's confirm is live, every key
    // (including a typed `:e`) would be swallowed by the confirm intercept,
    // so B has to already exist before that point.
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();
    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: :e must open a distinct second buffer");
    type_cmd(&mut ed, ":b #");
    assert_eq!(ed.focused_buffer_id(), bid_a, "setup: back on A");

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.config.confirm.is_some(),
        "setup: A's confirm must be open"
    );

    // Switch focus to B directly, bypassing key dispatch (and so the live
    // confirm's intercept), the same way a Steel `switch-to-buffer!` would.
    ed.switch_to_buffer_without_jump(bid_b);
    rewrite_externally(&tmp_b, "world, externally changed!\n");

    let (_, warnings_before) = ed.state.message_log.totals();
    ed.check_buffer_disk_state(bid_b, DiskCheckTrigger::BufferEnter);
    let (_, warnings_after) = ed.state.message_log.totals();

    assert!(
        matches!(
            ed.state.config.confirm.as_ref().unwrap().action,
            crate::ui::confirm::ConfirmAction::ReloadBuffer(id) if id == bid_a
        ),
        "A's confirm must survive B's check untouched"
    );
    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "B's blocked change must warn instead"
    );
}

/// A change hit mid macro-replay must never open a confirm — its intercept
/// would consume the next replayed key, silently truncating the macro.
/// Mirrors `change_detected_mid_insert_warns_instead_of_prompting`'s shape:
/// blocked during replay, deferred prompt still honoured afterward.
///
/// Fail oracle: without `!self.state.is_replaying` in `can_open_confirm`,
/// the confirm would open partway through `drain_replay_queue`, and the
/// first assertion below (confirm still `None`) would fail; the second
/// (focus actually reached B) would also fail if the intercept had eaten the
/// macro's remaining keys.
#[test]
fn confirm_does_not_open_during_macro_replay() {
    let (mut ed, _tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();
    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    type_cmd(&mut ed, ":b #");
    assert_eq!(
        ed.focused_buffer_id(),
        bid_a,
        "setup: back on A, B is the alternate"
    );

    // Record a macro into register 'q' that switches to the alternate (B).
    ed.handle_key(key('Q'));
    ed.handle_key(key('Q'));
    for ch in ":b #".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    ed.handle_key(key('Q'));
    assert_eq!(
        ed.focused_buffer_id(),
        bid_b,
        "setup: recording a live macro also executes it once"
    );

    // Back on A so the replay below has somewhere to switch away from.
    type_cmd(&mut ed, ":b #");
    assert_eq!(ed.focused_buffer_id(), bid_a);

    rewrite_externally(&tmp_b, "world, externally changed!\n");
    let (_, warnings_before) = ed.state.message_log.totals();

    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    let (_, warnings_after) = ed.state.message_log.totals();
    assert!(
        ed.state.config.confirm.is_none(),
        "no confirm may open mid-replay"
    );
    assert_eq!(
        ed.focused_buffer_id(),
        bid_b,
        "the whole macro must still have run, unswallowed by an intercept"
    );
    assert_eq!(warnings_after, warnings_before + 1, "must warn instead");

    // Outside replay, the deferred prompt still arrives on the next real
    // buffer-enter — same deferral rule as any other blocked case.
    ed.check_buffer_disk_state(bid_b, DiskCheckTrigger::BufferEnter);
    assert!(
        ed.state.config.confirm.is_some(),
        "the deferred prompt must arrive once replay is over"
    );
}
