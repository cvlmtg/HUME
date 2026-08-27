//! External file-change detection (`Editor::check_buffer_disk_state` /
//! `check_all_disk_state`, the `autoread` setting, the `:w` stale guard, and
//! the confirm overlay's reload/keep choices).

use super::*;
use crate::editor::buffer::{DiskCheckTrigger, DiskState};
use hume_grid::Rect;
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
/// Fail oracle: without `Editor::enter_buffer` routing `:bnext`/`:bprev`
/// through `switch_to_buffer_with_jump` and queuing `OnBufferEnter`, the
/// target buffer's disk state stays `InSync` no matter what changed
/// externally. Driven via `type_cmd_event`, not `type_cmd`: the check is
/// `OnBufferEnter`'s Rust reaction, observed only once `Editor::settle()`
/// runs its focus diff — `type_cmd_event` settles after the command,
/// `type_cmd` does not.
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
    type_cmd_event(&mut ed, ":bp");
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
/// The final `:b #` is driven via `type_cmd_event`: the disk check is
/// `OnBufferEnter`'s Rust reaction, observed only once `Editor::settle()`
/// runs its focus diff after the switch.
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
    type_cmd_event(&mut ed, ":b #");
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
        hume_scripting::host::PickerOpts::default(),
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

// ── `OnBufferEnter`'s disk check: closing/cycling/clicking reveals a
//    buffer with a deferred change ──────────────────────────────────────────
//
// The check is `OnBufferEnter`'s Rust reaction (`Editor::react_to_event`),
// raised by `Editor::settle()`'s own diff against `focused_buffer_id()` — not
// wired per command. Closing a buffer or a pane, cycling pane focus, and
// clicking into another pane all move focus with no write to `buffer_id`
// itself, so a diff taken inside `settle()`'s fixpoint is what catches them,
// not a call any of those commands makes. These tests drive input via
// `feed_event`/`type_cmd_event`, which call `settle()` after dispatch, not
// the usual `feed_key`/`type_cmd`, which leave it queued.

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
/// Fail oracle: before the `handle_input`-tail check, `:q` → `Editor::
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
/// to duplicate itself to cover this; `handle_input` covers both for free.
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
    ed.sync_viewport_dims(100, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    ed.handle_input(mouse_left_down(0, 0));
    ed.settle();

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

/// A macro that lands on a buffer whose change was already warned about
/// *before* the macro ever ran must still say something — going completely
/// silent would leave the user editing content that no longer matches disk
/// with no indication anywhere until `:w` refuses.
///
/// Fail oracle: with only the `!already_reported` warn clause (no
/// `is_buffer_enter && promptable` fallback), `already_reported` is already
/// `true` from the pre-replay ambient check below, so the replay-triggered
/// `BufferEnter` check would find nothing new to say and stay silent — the
/// final assertion would fail.
#[test]
fn macro_replay_onto_an_already_warned_stale_buffer_still_warns() {
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

    // Back on A so the replay below has somewhere to switch away from.
    type_cmd(&mut ed, ":b #");
    assert_eq!(ed.focused_buffer_id(), bid_a);

    // B changes externally and is warned about once, ambiently, *before*
    // replay ever runs — `already_reported` is true by the time replay hits.
    rewrite_externally(&tmp_b, "world, externally changed!\n");
    ed.check_buffer_disk_state(bid_b, DiskCheckTrigger::Ambient);
    let (_, warnings_before) = ed.state.message_log.totals();

    ed.handle_key(key('q'));
    ed.handle_key(key('q'));
    ed.drain_replay_queue();

    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(ed.focused_buffer_id(), bid_b);
    assert!(
        ed.state.config.confirm.is_none(),
        "no confirm may open mid-replay"
    );
    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "landing on an already-warned stale buffer during replay must still \
         warn, not go completely silent"
    );
}

/// A message already on screen when a macro replay starts must still shadow
/// the disk-change warning `drain_replay_queue`'s own trailing `settle()`
/// produces — even though every key `handle_key` dispatches clears
/// `status_msg` at its own start (see `mappings/mod.rs`), so the sentinel
/// message itself is long gone from `status_msg` by the time the replayed
/// keys finish; what must survive is the *shadowing flag*, not the message
/// text. Without it, `report_disk_state` falls back to `Editor::report`
/// (loud — sets `status_msg`) instead of a silent `message_log` push,
/// clobbering whatever the last replayed key left in `status_msg` with the
/// disk warning's own text. No built-in mapping both logs a message and
/// enqueues a replay in the same dispatch today, so this drives
/// `EditorState` directly rather than through a real key.
///
/// Fail oracle: drop the `message_already_logged` save/restore in
/// `drain_replay_queue` → `status_msg` ends up as `"world: file has changed
/// on disk"` instead of staying `None` (what `:b #`'s own `Enter` dispatch
/// leaves it as, since a successful `:b` reports nothing on its own).
#[test]
fn message_logged_before_a_macro_replay_survives_its_trailing_settle() {
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

    rewrite_externally(&tmp_b, "world, externally changed!\n");

    // Simulate a dispatch that both logged its own message and populated
    // `replay_queue` — the effect a hypothetical mapping doing both in one
    // `handle_input` call would leave behind.
    ed.report(Severity::Warning, "sentinel message".to_string());
    ed.state.message_logged_this_input = true;
    for ch in ":b #".chars() {
        ed.state.replay_queue.push_back(key(ch));
    }
    ed.state.replay_queue.push_back(key_enter());

    let (_, warnings_before) = ed.state.message_log.totals();
    ed.drain_replay_queue();

    assert_eq!(
        ed.focused_buffer_id(),
        bid_b,
        "setup: the replayed `:b #` must have switched onto stale B"
    );
    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "B's stale-file warning must still land in :messages"
    );
    assert_eq!(
        ed.state.status_msg, None,
        "the disk warning must land in :messages only (report_disk_state's \
         silent-push branch), not overwrite status_msg via Editor::report — \
         status_msg must stay whatever the last replayed key (`:b #`'s own \
         Enter, which reports nothing on success) left it as"
    );
}

/// A command that fails after moving focus (`:qa` jumping to the first
/// dirty buffer to name it in its error) must keep its own error on screen —
/// a reload confirm for that same buffer's external change must not replace
/// it, and must not steal the keystroke meant to answer the error (e.g.
/// retrying with `:qa!`).
///
/// Fail oracle: without the `message_logged_this_input` guard in
/// `can_open_confirm` (set by `Editor::handle_input` for the duration of its
/// post-dispatch focus-change check), the focus diff `typed_quit_all`
/// produces would still let `check_buffer_disk_state` open a reload confirm
/// over the "Unsaved changes" error — the second assertion below
/// (`confirm.is_none()`) would fail. `open_disk_change_confirm` never writes
/// `status_msg`, so the first assertion doesn't distinguish the two cases;
/// it documents the desired behaviour, not this guard specifically — see
/// `quit_all_focus_move_still_logs_the_disk_change_without_shadowing_the_error`
/// for the guard that does.
#[test]
fn quit_all_error_is_not_shadowed_by_a_disk_confirm() {
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();
    ed.handle_key(key('i'));
    ed.handle_key(key('!'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty(), "setup: A is dirty");

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    assert_ne!(ed.focused_buffer_id(), bid_a, "setup: B is focused, clean");

    rewrite_externally(&tmp_a, "hello, externally changed!\n");

    type_cmd_event(&mut ed, ":qa");

    assert_eq!(
        ed.focused_buffer_id(),
        bid_a,
        "setup: :qa must jump to the dirty buffer to name it"
    );
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("Unsaved changes")),
        "the :qa error must stay on screen, got: {:?}",
        ed.state.status_msg
    );
    assert!(
        ed.state.config.confirm.is_none(),
        "no reload confirm may shadow the :qa error"
    );
}

/// The disk change blocked by `quit_all_error_is_not_shadowed_by_a_disk_confirm`
/// must not vanish outright — it lands in `:messages` (bumping the warning
/// total) without touching `status_msg`, so `:qa`'s own error keeps the
/// status line but the change is still discoverable, matching every other
/// blocked-confirm case (mode-blocked, mid-replay).
///
/// Fail oracle: without `report_disk_state` falling back to
/// `message_log.push` (log-only) instead of `Editor::report` (which also
/// writes `status_msg`) whenever `message_logged_this_input` is set, this
/// warning would either clobber `status_msg` (breaking the sibling test) or
/// — if suppressed outright instead — never increment the warning total at
/// all, and the final assertion here would fail.
#[test]
fn quit_all_focus_move_still_logs_the_disk_change_without_shadowing_the_error() {
    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();
    ed.handle_key(key('i'));
    ed.handle_key(key('!'));
    ed.handle_key(key_esc());

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    let (_, warnings_before) = ed.state.message_log.totals();

    type_cmd_event(&mut ed, ":qa");

    assert_eq!(ed.focused_buffer_id(), bid_a, "setup: :qa lands on A");
    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "the blocked disk-change warning must still land in :messages"
    );
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("Unsaved changes")),
        "status_msg must still be :qa's own error, not the disk warning"
    );
}

/// Declining a reload confirm (`[k]eep`) must not reopen the identical
/// question on a later, unrelated focus change — only a further external
/// change (a new signature) should ask again. See `DiskState::Declined`.
///
/// Fail oracle: without recording the decline, cycling away and back would
/// still find `disk_state` at `Changed` for the same signature, and the
/// `BufferEnter` re-prompt rule would reopen the confirm — the middle
/// assertion below would fail.
#[test]
fn declined_confirm_does_not_reopen_on_later_focus_change() {
    let (mut ed, tmp_a, _tmp_b_guard, bid_a, bid_b) = two_panes_with_b_focused();
    assert_eq!(ed.focused_buffer_id(), bid_b, "setup: right pane focused");

    rewrite_externally(&tmp_a, "hello, externally changed!\n");

    // Cycle onto A: the deferred prompt opens for the first time.
    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('p'));
    assert_eq!(ed.focused_buffer_id(), bid_a);
    assert!(
        ed.state.config.confirm.is_some(),
        "setup: A's confirm opens"
    );

    // Decline it.
    ed.feed_event(key('k'));
    assert!(ed.state.config.confirm.is_none());

    // Cycle away to B and back to A: must stay silent for the same change.
    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('p'));
    assert_eq!(ed.focused_buffer_id(), bid_b);
    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('p'));
    assert_eq!(ed.focused_buffer_id(), bid_a);
    assert!(
        ed.state.config.confirm.is_none(),
        "a declined change must not re-prompt on a later, unrelated focus change"
    );

    // A further external change (a different signature) still asks again.
    rewrite_externally(&tmp_a, "hello, changed again!\n");
    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('p'));
    ed.feed_event(key_ctrl('p'));
    ed.feed_event(key('p'));
    assert_eq!(ed.focused_buffer_id(), bid_a);
    assert!(
        ed.state.config.confirm.is_some(),
        "a further external change must still prompt"
    );
}

/// A decline silences the *automatic* nagging (`Ambient`/`BufferEnter`), but
/// a direct `:checktime` — a check the user asked for on purpose — must
/// still say something about it. Without this, a buffer with no further
/// focus changes or terminal-focus events to trigger on could sit silently
/// out of sync with disk indefinitely, with only `:w`'s bare refusal (no
/// explanation) as a clue.
///
/// Fail oracle: without the `DiskCheckTrigger::Explicit` branch in the
/// `Changed` arm's `Declined` early-return, `:checktime` here would find
/// `declined` true and return before reporting anything — the final
/// assertion (`warnings_after == warnings_before + 1`) would fail.
#[test]
fn declined_change_still_warns_on_explicit_checktime() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid = ed.focused_buffer_id();
    rewrite_externally(&tmp, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    assert!(ed.state.config.confirm.is_some(), "setup: confirm opens");

    ed.feed_event(key('k'));
    assert!(ed.state.config.confirm.is_none(), "setup: declined");

    // Ambient stays silent for the declined signature.
    let (_, warnings_before) = ed.state.message_log.totals();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_mid) = ed.state.message_log.totals();
    assert_eq!(warnings_mid, warnings_before, "setup: Ambient stays silent");

    // :checktime — a direct request — still warns, and still doesn't prompt.
    type_cmd(&mut ed, ":checktime");
    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after,
        warnings_before + 1,
        ":checktime must still warn about a declined change, not stay silent forever"
    );
    assert!(
        ed.state.config.confirm.is_none(),
        ":checktime must warn, not reopen the confirm the user already answered"
    );
}

/// `Declined` only silences the *specific declined signature*'s `Changed`
/// report — it must not swallow an unrelated later `Vanished` warning for
/// the same buffer.
///
/// Fail oracle: if the `Declined` short-circuit lived above the match on
/// `DiskChange` instead of inside the `Changed` arm alone, a stale guard
/// would suppress the `Vanished` warning too — the final assertion would
/// fail.
#[test]
fn declined_change_then_vanished_file_still_warns() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid = ed.focused_buffer_id();
    rewrite_externally(&tmp, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    ed.feed_event(key('k'));
    assert!(ed.state.config.confirm.is_none(), "setup: declined");

    std::fs::remove_file(&tmp).unwrap();
    let (_, warnings_before) = ed.state.message_log.totals();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    let (_, warnings_after) = ed.state.message_log.totals();

    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "a declined Changed signature must not swallow a later Vanished warning"
    );
    assert_eq!(ed.state.buffers.get(bid).disk_state, DiskState::Vanished);
}

/// A confirm left open for a buffer the user just clicked away from
/// (`handle_mouse` has no confirm-key intercept, unlike `handle_key`) must
/// be retired rather than left on screen pointing at a pane no longer in
/// view — and the buffer the click actually landed on must get its own
/// deferred prompt, not be blocked by the stale one.
///
/// Fail oracle: without `enter_buffer_disk_check` (`OnBufferEnter`'s Rust
/// reaction) retiring a confirm that no longer targets the newly-focused
/// buffer, the assertion below would still find B's confirm naming B,
/// blocking A's own prompt via `can_open_confirm`'s `confirm.is_none()`
/// gate.
#[test]
fn mouse_click_into_another_pane_retires_a_stale_confirm() {
    let (mut ed, tmp_a, tmp_b_guard, bid_a, bid_b) = two_panes_with_b_focused();
    assert_eq!(ed.focused_buffer_id(), bid_b, "setup: right pane focused");

    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    ed.check_buffer_disk_state(bid_a, DiskCheckTrigger::Ambient);
    assert!(
        ed.state.config.confirm.is_none(),
        "setup: A is not focused yet"
    );

    rewrite_externally(&tmp_b_guard, "world, externally changed!\n");
    ed.check_buffer_disk_state(bid_b, DiskCheckTrigger::Ambient);
    assert!(
        matches!(
            ed.state.config.confirm.as_ref().unwrap().action,
            crate::ui::confirm::ConfirmAction::ReloadBuffer(id) if id == bid_b
        ),
        "setup: B's own confirm is open"
    );

    // 100×25, 0.5 split: pane A (left) spans x ∈ [0, 49); col 0 lands inside
    // it regardless of gutter width (see `clicking_into_another_pane_prompts_that_panes_buffer`).
    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(100, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    ed.handle_input(mouse_left_down(0, 0));
    ed.settle();

    assert_eq!(ed.focused_buffer_id(), bid_a);
    assert!(
        matches!(
            ed.state.config.confirm.as_ref().unwrap().action,
            crate::ui::confirm::ConfirmAction::ReloadBuffer(id) if id == bid_a
        ),
        "B's orphaned confirm must be retired and A's own prompt opened in its place"
    );
}

// ── OnBufferEnter / OnFocusGained ──────────────────────────────────────────────

/// The originating bug, end to end: a picker accept switching onto a buffer
/// whose backing file changed externally must open the reload confirm. Built
/// on `tests/picker_steel.rs`'s harness — a picker's `on_select` callback
/// queues as a `PendingWork::Call`, drained by the next `render_to_buf`
/// (`settle()` internally), same as `on-buffer-enter`.
///
/// Fail oracle: a disk check wired only into typed commands — the picker
/// path never ran through `enter_buffer_with_jump` (a fuzzy picker doesn't
/// dispatch `:e`/`:b`) or
/// `handle_input`'s tail check (the switch happens a frame later, inside the
/// drain), so `ed.state.config.confirm` stays `None`.
#[test]
fn picker_accept_onto_an_externally_changed_buffer_opens_the_reload_confirm() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");

    let target = tmp.path().join("target.md");
    std::fs::write(&target, "hi\n").unwrap();
    let path = target.to_string_lossy().replace('\\', "/");

    // Open the target buffer up front so its stored signature reflects the
    // pre-change content — the picker below switches onto this *already-open*
    // buffer without re-reading, matching a buffer-switcher picker (not a
    // file-opener, which would read the post-change content fresh and never
    // see a mismatch).
    let bid = ed.resolve_open_path(&path).unwrap().0;
    ed.settle();

    // File changes on disk while the buffer stays open in the background.
    rewrite_externally(&target, "hi, externally changed!\n");

    let mut host = hume_scripting::ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        &format!(
            r#"(define-command! "go" "" (lambda ()
                 (picker! (list (cons "target" "{path}"))
                   (lambda (p) (when p (switch-to-buffer! (open-buffer! p)))))))"#
        ),
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":go");
    assert!(ed.state.config.picker.is_some(), "sanity: picker open");

    let rect = Rect::new(0, 0, 40, 12);
    let _ = ed.render_to_buf(rect);

    // Accept: close_picker queues on_select as a PendingWork::Call.
    ed.feed_key(key_enter());

    // The next render_to_buf drains the callback (switching to the stale
    // buffer) and settles — settle()'s OnBufferEnter diff must observe the
    // switch and run the disk check in the same drain.
    let _ = ed.render_to_buf(rect);

    assert_eq!(
        ed.focused_buffer_id(),
        bid,
        "sanity: picker accept switched onto the target buffer"
    );
    assert!(
        ed.state.config.confirm.is_some(),
        "picker accept onto an externally-changed buffer must open the reload confirm \
         — the originating bug this refactor fixes"
    );
}

/// A non-interactive `switch-to-buffer!` — Steel's builtin, LSP goto-
/// definition, any async callback — onto a stale buffer must open the reload
/// confirm too. This path used to run no check at all: the deleted
/// `enter_buffer_with_jump` was only reachable from typed commands, and
/// `switch_to_buffer_with_jump` (what non-interactive callers use) never
/// called it.
///
/// Fail oracle: gate the reaction on `Editor::handle_input`'s dispatch
/// somehow surviving instead of living in `settle()`'s own diff → a switch
/// with no interactive dispatch behind it never reaches the check.
#[test]
fn non_interactive_switch_to_buffer_onto_a_stale_buffer_opens_the_reload_confirm() {
    let (mut ed, _tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    let bid_b = ed.resolve_open_path(tmp_b.to_str().unwrap()).unwrap().0;
    ed.settle();
    assert_ne!(bid_a, bid_b, "setup: two distinct buffers, A still focused");

    rewrite_externally(&tmp_b, "world, externally changed!\n");

    // The non-interactive primitive: Steel's `switch-to-buffer!` and every
    // LSP goto-definition call go through exactly this, never a typed
    // command.
    ed.switch_to_buffer_with_jump(bid_b);
    ed.settle();

    assert_eq!(ed.focused_buffer_id(), bid_b);
    assert!(
        ed.state.config.confirm.is_some(),
        "a non-interactive switch onto a stale buffer must still prompt"
    );
}

/// `OnFocusGained`'s reaction is `check_all_disk_state(Ambient)` — a sweep
/// over every open buffer, not just the focused one.
///
/// Fail oracle: wire the reaction to a single-buffer check on the focused
/// buffer instead → the non-focused buffer's warning never fires.
#[test]
fn focus_gained_sweeps_every_open_buffer_not_just_the_focused_one() {
    use termina::event::Event as TerminalEvent;

    let (mut ed, tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: B is now focused, A is not");

    // A, the non-focused buffer, changes externally.
    rewrite_externally(&tmp_a, "hello, externally changed!\n");
    let (_, warnings_before) = ed.state.message_log.totals();

    ed.handle_input(TerminalEvent::FocusIn);
    ed.settle();

    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after,
        warnings_before + 1,
        "OnFocusGained's sweep must warn for A even though B is focused"
    );
    assert!(
        ed.state.config.confirm.is_none(),
        "A is not focused, so an ambient check only warns — never opens a confirm"
    );
}

/// `:b <other>` onto a stale buffer must run the disk check exactly once —
/// pinning against a double-check regression from a direct call surviving
/// alongside `settle()`'s event-driven diff. A second run would
/// find the confirm already open (`can_open_confirm`'s `confirm.is_none()`
/// guard blocks it) and fall through to `report_disk_state`'s warn fallback
/// instead — an extra `:messages` entry alongside the confirm.
///
/// Fail oracle: leave a direct `check_buffer_disk_state`/
/// `enter_buffer_disk_check` call wired in at the `:b` command itself,
/// alongside the diff → `warnings_after` is one higher than
/// `warnings_before`.
#[test]
fn switching_onto_a_stale_buffer_checks_disk_state_exactly_once() {
    let (mut ed, _tmp_a) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid_a = ed.focused_buffer_id();

    let (tmp_b, _tmp_b_guard) = temp_file("world\n");
    type_cmd(&mut ed, &format!(":e {}", tmp_b.display()));
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: two distinct buffers, B focused");

    type_cmd(&mut ed, ":b #");
    assert_eq!(ed.focused_buffer_id(), bid_a, "setup: back on A");

    rewrite_externally(&tmp_b, "world, externally changed!\n");
    let (_, warnings_before) = ed.state.message_log.totals();

    type_cmd_event(&mut ed, ":b #");
    assert_eq!(
        ed.focused_buffer_id(),
        bid_b,
        "setup: switched onto stale B"
    );

    assert!(
        ed.state.config.confirm.is_some(),
        "sanity: the switch onto B must open the reload confirm"
    );
    let (_, warnings_after) = ed.state.message_log.totals();
    assert_eq!(
        warnings_after, warnings_before,
        "the disk check must run exactly once — a second run would find the \
         confirm already open and fall back to warning instead"
    );
}

// ── Inline-output disk sweep (dispatch.rs's `OnFocusGained`) ─────────────────

/// An `#:inline-output` command that both rewrites the focused file (a
/// formatter, a `git checkout` wrapper) *and* logs its own warning (a
/// non-zero exit, a lint note) must still open the reload confirm — its own
/// warning must not shadow the very disk change it caused.
///
/// `can_open_confirm`'s message-shadow clause exists to protect the
/// `BufferEnter` case (`:qa` landing on a dirty buffer): a command's own
/// failure message must survive an *unrelated* buffer-enter check that
/// happens to run right after. The `Ambient` sweep this test exercises is
/// never unrelated — it exists because of this exact command — so it must be
/// exempt.
///
/// Fail oracle: apply the message-shadow clause unconditionally (drop the
/// `trigger != DiskCheckTrigger::BufferEnter` guard) — `message_logged_this_input`
/// is `true` from this dispatch's own `log!` warning by the time the queued
/// `OnFocusGained` reaction runs in the next `settle()`, so `confirm` would
/// stay `None` and only a `:messages` line would appear.
#[test]
fn inline_output_commands_own_warning_does_not_shadow_its_own_reload_confirm() {
    use crate::editor::keymap::BindMode;
    use crate::editor::scripting_setup::make_init_host;
    use hume_scripting::ScriptingHost;

    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    // No real terminal in this harness — Armed (not Headless) is what makes
    // `run_steel_command` queue `OnFocusGained` at all; only `Entered` (which
    // this command's body never reaches, since it only logs) needs one.
    ed.tui_active = true;

    let mut host = ScriptingHost::new();
    {
        let mut init_host = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_source(
            r#"(define-command! "lint-and-fix" "doc"
                 (lambda () (log! 'warn "lint: 1 issue auto-fixed")) #:inline-output #t)"#,
            &mut init_host,
        )
        .expect("eval failed");
    }
    ed.scripting = Some(host);
    ed.state.config.keymap.bind_user_with_extend(
        BindMode::Normal,
        &[key('\\')],
        "lint-and-fix".into(),
        false,
    );

    // The command's own subprocess rewrote the focused file while it ran.
    rewrite_externally(&tmp, "hello, world!\n");

    ed.feed_event(key('\\'));

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.text.contains("lint: 1 issue auto-fixed")),
        "sanity: the command's own warning must have logged"
    );
    assert!(
        ed.state.config.confirm.is_some(),
        "the reload confirm must open despite the command's own warning"
    );
}
