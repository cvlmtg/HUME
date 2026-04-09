//! Editor-level command functions.
//!
//! Each function in this module is a command that requires `&mut Editor`
//! context — composite operations involving mode changes, registers, undo
//! groups, or parameterized motions (find/till/replace).
//!
//! They are registered in [`super::registry`] and called via function pointer
//! from `execute_keymap_command`, exactly like the pure `cmd_*` functions in
//! `ops/motion.rs`, `ops/edit.rs`, etc.
//!
//! The `count` parameter is the user's numeric prefix (default 1). Commands
//! that don't use a count accept it and ignore it (`_count`).



use std::sync::Arc;

use crate::core::buffer::Buffer;
use crate::core::grapheme::next_grapheme_boundary;
use crate::core::selection::{Selection, SelectionSet};
use crate::ops::edit::{delete_selection, insert_char};
use crate::ops::motion::{
    cmd_goto_first_nonblank, cmd_goto_line_end, cmd_goto_line_start,
    cmd_move_left, cmd_move_right,
    find_char_backward, find_char_forward,
};
use crate::ops::MotionMode;
use crate::ops::register::{yank_selections, DEFAULT_REGISTER, SEARCH_REGISTER};
use crate::ops::search::{compile_search_regex, escape_regex, find_all_matches, find_match_from_cache, find_next_match};
use crate::ops::selection_cmd::cmd_collapse_selection;
use crate::ops::text_object::inner_word_impl;
use crate::helpers::is_word_boundary;

use engine::types::EditorMode;

use super::{Editor, FindChar, FindKind, MiniBuffer, Mode, SearchDirection};

// ── Mode transitions ──────────────────────────────────────────────────────────

pub(super) fn cmd_insert_before(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    ed.apply_motion(|_b, sels| sels.map(|s| Selection::collapsed(s.start())));
    ed.begin_insert_session();
}

pub(super) fn cmd_insert_after(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    ed.apply_motion(|b, s| cmd_move_right(b, s, 1, MotionMode::Move));
    ed.begin_insert_session();
}

pub(super) fn cmd_insert_at_line_start(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    ed.apply_motion(|b, s| cmd_goto_first_nonblank(b, s, 1, MotionMode::Move));
    ed.begin_insert_session();
}

pub(super) fn cmd_insert_at_line_end(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    ed.apply_motion(|b, s| cmd_goto_line_end(b, s, 1, MotionMode::Move));
    ed.apply_motion(|b, s| cmd_move_right(b, s, 1, MotionMode::Move));
    ed.begin_insert_session();
}

/// Open a new line below the cursor and enter insert mode.
///
/// `begin_insert_session` opens the edit group so the structural `\n` and
/// everything typed before Esc form one undo step — the same pattern as
/// `cmd_change`.
pub(super) fn cmd_open_line_below(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    ed.begin_insert_session();
    ed.apply_motion(|b, s| cmd_goto_line_end(b, s, 1, MotionMode::Move));
    ed.apply_motion(|b, s| cmd_move_right(b, s, 1, MotionMode::Move));
    ed.doc.apply_edit_grouped(|b, s| insert_char(b, s, '\n'));
}

/// Open a new line above the cursor and enter insert mode.
pub(super) fn cmd_open_line_above(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    ed.begin_insert_session();
    ed.apply_motion(|b, s| cmd_goto_line_start(b, s, 1, MotionMode::Move));
    ed.doc.apply_edit_grouped(|b, s| insert_char(b, s, '\n'));
    ed.apply_motion(|b, s| cmd_move_left(b, s, 1, MotionMode::Move));
}

pub(super) fn cmd_command_mode(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    ed.set_mode(Mode::Command);
    ed.minibuf = Some(MiniBuffer { prompt: ':', input: String::new(), cursor: 0 });
}

pub(super) fn cmd_exit_insert(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    ed.end_insert_session();
}

// ── Edit composites ───────────────────────────────────────────────────────────

/// Yank selections into the default register, then delete them.
pub(super) fn cmd_delete(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    let yanked = yank_selections(ed.doc.buf(), ed.doc.sels());
    ed.doc.apply_edit(delete_selection);
    ed.registers.write_text(DEFAULT_REGISTER, yanked);
}

/// Yank, delete, then enter insert mode — all in one undo group.
///
/// `begin_insert_session` opens the group so the delete and everything typed
/// before Esc form one undo step.
pub(super) fn cmd_change(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    let yanked = yank_selections(ed.doc.buf(), ed.doc.sels());
    ed.begin_insert_session();
    ed.doc.apply_edit_grouped(delete_selection);
    ed.registers.write_text(DEFAULT_REGISTER, yanked);
}

/// Yank selections into the default register without deleting.
pub(super) fn cmd_yank(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    let yanked = yank_selections(ed.doc.buf(), ed.doc.sels());
    ed.registers.write_text(DEFAULT_REGISTER, yanked);
}

/// Shared body for paste commands: read the default register, run `paste_fn`,
/// then write displaced text back if any selection was non-cursor (replace-and-swap).
fn do_paste(
    ed: &mut Editor,
    paste_fn: impl Fn(Buffer, SelectionSet, &[String]) -> (Buffer, SelectionSet, crate::core::changeset::ChangeSet, Vec<String>),
) {
    if let Some(reg) = ed.registers.read(DEFAULT_REGISTER)
        && let Some(values) = reg.as_text()
    {
        let values = values.to_vec();
        let displaced = ed.doc.apply_edit(|b, s| paste_fn(b, s, &values));
        if displaced.iter().any(|s| !s.is_empty()) {
            ed.registers.write_text(DEFAULT_REGISTER, displaced);
        }
    }
}

/// Paste after the selection; swap displaced text back into the register when
/// the selection was non-cursor (replace-and-swap semantics).
pub(super) fn cmd_paste_after(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    use crate::ops::edit::paste_after;
    do_paste(ed, paste_after);
}

/// Paste before the selection; same replace-and-swap semantics as `cmd_paste_after`.
pub(super) fn cmd_paste_before(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    use crate::ops::edit::paste_before;
    do_paste(ed, paste_before);
}

pub(super) fn cmd_undo(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    ed.doc.undo();
}

pub(super) fn cmd_redo(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    ed.doc.redo();
}

// ── Selection state ───────────────────────────────────────────────────────────

pub(super) fn cmd_toggle_extend(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    ed.mode = if ed.mode == EditorMode::Extend {
        EditorMode::Normal
    } else {
        EditorMode::Extend
    };
}

/// Collapse each selection to its cursor AND exit extend mode.
///
/// Collapsing is a "done selecting" signal, so extend mode is always cleared.
pub(super) fn cmd_collapse_and_exit_extend(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    // Mode is SSOT for extend state; setting Normal implicitly clears Extend.
    ed.mode = EditorMode::Normal;
    ed.apply_motion(|b, s| cmd_collapse_selection(b, s, MotionMode::Move));
}

// ── Find / till character ─────────────────────────────────────────────────────
//
// All eight find/till commands read the character argument from
// `ed.pending_char`, which was stored by the WaitChar consumption path.

/// Shared implementation for the eight find/till commands.
fn find_char(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
    kind: FindKind,
    find_fn: fn(&Buffer, SelectionSet, MotionMode, usize, char, FindKind) -> SelectionSet,
) {
    if let Some(ch) = ed.pending_char.take() {
        ed.apply_motion(|b, s| find_fn(b, s, mode, count, ch, kind));
        ed.last_find = Some(FindChar { ch, kind });
    }
}

pub(super) fn cmd_find_forward(ed: &mut Editor, count: usize, mode: MotionMode) {
    find_char(ed, count, mode, FindKind::Inclusive, find_char_forward);
}
pub(super) fn cmd_find_backward(ed: &mut Editor, count: usize, mode: MotionMode) {
    find_char(ed, count, mode, FindKind::Inclusive, find_char_backward);
}
pub(super) fn cmd_till_forward(ed: &mut Editor, count: usize, mode: MotionMode) {
    find_char(ed, count, mode, FindKind::Exclusive, find_char_forward);
}
pub(super) fn cmd_till_backward(ed: &mut Editor, count: usize, mode: MotionMode) {
    find_char(ed, count, mode, FindKind::Exclusive, find_char_backward);
}

// ── Repeat find ───────────────────────────────────────────────────────────────

/// Shared implementation for the four repeat-find commands.
fn repeat_find(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
    find_fn: fn(&Buffer, SelectionSet, MotionMode, usize, char, FindKind) -> SelectionSet,
) {
    if let Some(FindChar { ch, kind }) = ed.last_find {
        ed.apply_motion(|b, s| find_fn(b, s, mode, count, ch, kind));
    }
}

pub(super) fn cmd_repeat_find_forward(ed: &mut Editor, count: usize, mode: MotionMode) {
    repeat_find(ed, count, mode, find_char_forward);
}
pub(super) fn cmd_repeat_find_backward(ed: &mut Editor, count: usize, mode: MotionMode) {
    repeat_find(ed, count, mode, find_char_backward);
}

// ── Replace ───────────────────────────────────────────────────────────────────

/// Replace every character in each selection with the next typed character.
///
/// Reads the replacement character from `ed.pending_char`.
pub(super) fn cmd_replace(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    use crate::ops::edit::replace_selections;
    if let Some(ch) = ed.pending_char.take() {
        ed.doc.apply_edit(|b, s| replace_selections(b, s, ch));
    }
}

// ── Dot repeat ───────────────────────────────────────────────────────────────

/// Replay the last repeatable editing action.
///
/// Count semantics: if the user typed an explicit count before `.`, that count
/// overrides the original; otherwise the original count is reused. This mirrors
/// Vim's behaviour (`3.` → repeat with 3; `.` alone → repeat with original count).
pub(super) fn cmd_repeat(ed: &mut Editor, count: usize, _mode: MotionMode) {
    let Some(action) = ed.last_action.take() else { return };

    // Prefer an explicit user count; fall back to the count from the original action.
    let effective_count = if ed.explicit_count { count } else { action.count };

    // Restore the char arg so wait-char commands (replace, find/till) work.
    ed.pending_char = action.char_arg;

    // Pre-open the edit group before re-executing. This is the replay signal:
    // `begin_insert_session` checks `is_group_open()` and suppresses both the
    // redundant `begin_edit_group` call and keystroke recording when the group
    // is already open. For non-insert commands the group stays empty and the
    // commit below is a no-op.
    ed.doc.begin_edit_group();

    // Re-execute the original command through the normal dispatch path.
    // extend=false because the replayed command was already resolved to its
    // final form (the resolved name is what gets stored in RepeatableAction).
    // Clone the name while `action` is locally owned (moved out via `.take()`).
    ed.execute_keymap_command(action.command.clone(), effective_count, false);

    // Feed recorded insert keystrokes through the normal insert handler.
    // `KeyEvent` is `Copy`, so iterate by reference and dereference each key.
    for key in &action.insert_keys {
        ed.handle_insert(*key);
    }

    // Close the insert session / edit group:
    // - For insert commands: `end_insert_session` commits the group (delete +
    //   typed text as one undo step). `insert_session` is `None` here (replay
    //   suppressed it), so no keystrokes are moved into `last_action`.
    // - For non-insert commands: the group is empty (no `apply_edit_grouped`
    //   calls), so `commit_edit_group` is a no-op and the command's own
    //   `apply_edit` revision stands alone in history.
    if ed.mode == EditorMode::Insert {
        ed.end_insert_session();
    } else {
        ed.doc.commit_edit_group();
    }

    // Restore the original action so `.` can be pressed again.
    // `execute_keymap_command` may have overwritten `last_action` during
    // replay; this final assignment ensures the stored action is always the
    // one the user actually performed.
    ed.last_action = Some(action);
}

// ── Page / half-page scroll ───────────────────────────────────────────────────
//
// Uses `view.height` (or half of it) as the move count rather than the user's
// numeric prefix. Calls the visual-move commands directly instead of going
// through the registry to avoid a runtime string lookup.

pub(super) fn cmd_page_down(ed: &mut Editor, _count: usize, mode: MotionMode) {
    let count = ed.viewport().height as usize;
    cmd_visual_move_down(ed, count, mode);
}
pub(super) fn cmd_page_up(ed: &mut Editor, _count: usize, mode: MotionMode) {
    let count = ed.viewport().height as usize;
    cmd_visual_move_up(ed, count, mode);
}
pub(super) fn cmd_half_page_down(ed: &mut Editor, _count: usize, mode: MotionMode) {
    let count = (ed.viewport().height as usize / 2).max(1);
    cmd_visual_move_down(ed, count, mode);
}
pub(super) fn cmd_half_page_up(ed: &mut Editor, _count: usize, mode: MotionMode) {
    let count = (ed.viewport().height as usize / 2).max(1);
    cmd_visual_move_up(ed, count, mode);
}

// Visual-line movement lives in visual_move.rs; re-export for the registry glob.
pub(super) use super::visual_move::{cmd_visual_move_down, cmd_visual_move_up};

// ── Search ────────────────────────────────────────────────────────────────────

/// Enter forward search mode.
///
/// Snapshots the current selections for cancel-restore, then opens the
/// mini-buffer with the `/` prompt.
pub(super) fn cmd_search_forward(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    ed.search.pre_search_sels = Some(ed.doc.sels().clone());
    ed.search.direction = SearchDirection::Forward;
    // Capture extend state before mode becomes Search — live search uses it.
    ed.search.extend = ed.mode == EditorMode::Extend;
    ed.set_mode(Mode::Search);
    ed.minibuf = Some(MiniBuffer { prompt: '/', input: String::new(), cursor: 0 });
}

/// Enter backward search mode.
pub(super) fn cmd_search_backward(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    ed.search.pre_search_sels = Some(ed.doc.sels().clone());
    ed.search.direction = SearchDirection::Backward;
    // Capture extend state before mode becomes Search — live search uses it.
    ed.search.extend = ed.mode == EditorMode::Extend;
    ed.set_mode(Mode::Search);
    ed.minibuf = Some(MiniBuffer { prompt: '?', input: String::new(), cursor: 0 });
}

/// Build the primary selection after a search match.
///
/// `anchor = Some(a)` — extend mode: keep the caller's anchor, move head to
/// the match edge that faces the search direction.
/// `anchor = None` — move mode: cover the matched text exactly.
pub(super) fn search_sel(
    start: usize,
    end_incl: usize,
    anchor: Option<usize>,
    direction: SearchDirection,
) -> Selection {
    match anchor {
        Some(a) => Selection::new(a, match direction {
            SearchDirection::Forward  => end_incl,
            SearchDirection::Backward => start,
        }),
        None => Selection::new(start, end_incl),
    }
}

/// Ensure `ed.search.regex` is populated, compiling from `SEARCH_REGISTER` if
/// needed. Returns `true` if a usable regex is now in place, `false` otherwise.
fn ensure_search_regex(ed: &mut Editor) -> bool {
    if ed.search.regex.is_some() { return true; }
    let pattern = ed
        .registers
        .read(SEARCH_REGISTER)
        .and_then(|r| r.as_text().and_then(|v| v.first()).cloned())
        .unwrap_or_default();
    if pattern.is_empty() { return false; }
    match compile_search_regex(&pattern) {
        Some(r) => { ed.search.set_regex(Some(r)); true }
        None => false,
    }
}

/// Shared body for `search-next` / `search-prev` / extend variants.
///
/// Reads the cached `search_regex` (compiled during the search session), or
/// recompiles from the `'s'` register if the cache is empty. Repeats `count`
/// times (e.g. `3n` jumps 3 matches forward). Moves or extends the primary
/// selection depending on `extend`.
fn search_jump(ed: &mut Editor, count: usize, direction: SearchDirection, mode: MotionMode) {
    if !ensure_search_regex(ed) { return; }
    let Some(regex) = &ed.search.regex else { return };

    // Capture anchor before the loop (extend mode keeps the original anchor fixed).
    let (mut from_char, anchor) = {
        let buf = ed.doc.buf();
        let primary = ed.doc.sels().primary();
        let from = match direction {
            // Step past the current match so we don't re-find it on the first jump.
            SearchDirection::Forward => next_grapheme_boundary(buf, primary.end_inclusive(buf)),
            SearchDirection::Backward => primary.start(),
        };
        (from, if mode == MotionMode::Extend { Some(primary.anchor) } else { None })
    };

    // Jump `count` times, advancing `from_char` after each match so that
    // `3n` really does land on the 3rd match from the current position.
    //
    // When the match cache is populated we binary-search it (O(log M) per
    // jump). When it is empty — e.g. the very first `n` after startup before
    // the cache is warmed — we fall back to the O(buffer) regex-scan path.
    let count = count.max(1);
    let mut last_match: Option<(usize, usize)> = None;
    let mut any_wrapped = false;
    let use_cache = !ed.search.matches.is_empty();

    for _ in 0..count {
        let result = if use_cache {
            find_match_from_cache(&ed.search.matches, from_char, direction)
        } else {
            find_next_match(ed.doc.buf(), regex, from_char, direction)
        };
        match result {
            Some((start, end_incl, wrapped)) => {
                any_wrapped |= wrapped;
                last_match = Some((start, end_incl));
                from_char = match direction {
                    SearchDirection::Forward => next_grapheme_boundary(ed.doc.buf(), end_incl),
                    // For backward: next search must land before the current match start.
                    SearchDirection::Backward => start,
                };
            }
            None => {
                last_match = None;
                break;
            }
        }
    }

    match last_match {
        Some((start, end_incl)) => {
            ed.search.wrapped = any_wrapped;
            let new_sel = search_sel(start, end_incl, anchor, direction);
            ed.set_primary_selection(new_sel);
        }
        None => {
            ed.status_msg = Some("no match".into());
        }
    }
}

/// Clear the active search regex and dismiss all match highlights.
///
/// Also invocable as `:clear-search` / `:cs` in command mode.
pub(super) fn cmd_clear_search(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    ed.search.clear();
    // update_search_cache() is called by the event loop after handle_key returns,
    // but search.clear() already zeroes the cache fields directly, so the render
    // path sees a clean state immediately regardless of event-loop ordering.
}

pub(super) fn cmd_search_next(ed: &mut Editor, count: usize, mode: MotionMode) {
    search_jump(ed, count, SearchDirection::Forward, mode);
}
pub(super) fn cmd_search_prev(ed: &mut Editor, count: usize, mode: MotionMode) {
    search_jump(ed, count, SearchDirection::Backward, mode);
}

// ── Select all matches ────────────────────────────────────────────────────────

/// Turn every search match in the buffer into a selection.
///
/// Uses the active search regex, falling back to recompiling from the `'s'`
/// register (same as `n`/`N`). If there is no active search, does nothing.
/// The first match becomes primary.
pub(super) fn cmd_select_all_matches(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    if !ensure_search_regex(ed) { return; }
    let Some(regex) = &ed.search.regex else { return };

    let matches = find_all_matches(ed.doc.buf(), regex);
    if matches.is_empty() {
        ed.status_msg = Some("no matches".into());
        return;
    }

    let sels: Vec<Selection> = matches.into_iter().map(|(s, e)| Selection::new(s, e)).collect();
    ed.doc.set_selections(SelectionSet::from_vec(sels, 0));
}

// ── Select within (s) ────────────────────────────────────────────────────────

/// Enter Select mode.
///
/// Snapshots the current selections for cancel-restore, then opens the
/// mini-buffer with the `s` prompt. The user types a regex; all matches
/// within the current selections become new selections (live preview).
pub(super) fn cmd_select_within(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    // Nothing meaningful to search within a single-char selection.
    if ed.doc.sels().iter_sorted().all(Selection::is_collapsed) {
        return;
    }
    ed.pre_select_sels = Some(ed.doc.sels().clone());
    ed.set_mode(Mode::Select);
    ed.minibuf = Some(MiniBuffer { prompt: '⫽', input: String::new(), cursor: 0 });
}

// ── Use selection as search (*) ──────────────────────────────────────────────

/// Use the primary selection text as the search pattern.
///
/// If the primary selection is a cursor (1-char), expands to the word under
/// the cursor first (same as Helix). The escaped text is compiled as a search
/// regex, stored in the `'s'` register, and search highlights appear immediately.
pub(super) fn cmd_use_selection_as_search(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    let buf = ed.doc.buf();
    let primary = ed.doc.sels().primary();

    // If cursor (1-char selection), expand to inner word first.
    let (text, new_sel): (String, Option<Selection>) = if primary.is_collapsed() {
        let Some((start, end)) = inner_word_impl(buf, primary.head, is_word_boundary) else {
            return; // cursor on structural newline or similar — nothing to do
        };
        let word_text = buf.slice(start..end + 1).to_string();
        (word_text, Some(Selection::new(start, end)))
    } else {
        let text = buf.slice(primary.start()..primary.end_inclusive(buf) + 1).to_string();
        (text, None)
    };

    if text.is_empty() {
        return;
    }

    // Update the primary selection to cover the word (for cursor expansion).
    if let Some(sel) = new_sel {
        ed.set_primary_selection(sel);
    }

    let escaped = escape_regex(&text);
    let Some(regex) = compile_search_regex(&escaped) else {
        return;
    };

    // Store in search register and set as active search.
    ed.registers.write_text(SEARCH_REGISTER, vec![escaped]);
    ed.search.direction = SearchDirection::Forward;
    ed.search.set_regex(Some(regex));
}

// ── Misc ──────────────────────────────────────────────────────────────────────

pub(super) fn cmd_quit(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    ed.should_quit = true;
}

// ── Typed command implementations ────────────────────────────────────────────
//
// These functions are registered in `CommandRegistry` as typed commands
// (`:` command line). They are `pub(super)` so `registry.rs` can import them.

pub(super) fn typed_quit(ed: &mut Editor, _arg: Option<&str>, force: bool) {
    if !force && ed.doc.is_dirty() {
        ed.status_msg = Some("Unsaved changes (add ! to override)".into());
    } else {
        ed.should_quit = true;
    }
}

pub(super) fn typed_write(ed: &mut Editor, arg: Option<&str>, force: bool) {
    if force {
        ed.status_msg = Some("Error: w! is not supported".into());
    } else {
        write_file(ed, arg);
    }
}

pub(super) fn typed_write_quit(ed: &mut Editor, arg: Option<&str>, force: bool) {
    // force applies to the quit part: quit even if the write fails.
    if write_file(ed, arg) || force {
        ed.should_quit = true;
    }
}

pub(super) fn typed_toggle_soft_wrap(ed: &mut Editor, _arg: Option<&str>, _force: bool) {
    use engine::pane::WrapMode;
    let currently_wrapping = ed.doc.overrides.wrap_mode(&ed.settings).is_wrapping();
    if currently_wrapping {
        ed.doc.overrides.wrap_mode = Some(WrapMode::None);
        // Horizontal offset is now meaningful; scroll stays where it is.
    } else {
        // Estimate gutter width (line numbers + separator). The engine will
        // compute the exact width at render time; this just needs to be close
        // enough for a reasonable default wrap column.
        const GUTTER_WIDTH_ESTIMATE: u16 = 4;
        let content_w = ed.viewport().width.saturating_sub(GUTTER_WIDTH_ESTIMATE).max(1);
        ed.doc.overrides.wrap_mode = Some(WrapMode::Indent { width: content_w });
        ed.viewport_mut().horizontal_offset = 0;
        ed.viewport_mut().top_row_offset = 0;
    }
    let state = if currently_wrapping { "off" } else { "on" };
    ed.status_msg = Some(format!("Soft wrap {state}"));
}

pub(super) fn typed_set(ed: &mut Editor, arg: Option<&str>, _force: bool) {
    const USAGE: &str = "Usage: :set global|buffer key=value";
    let Some(arg) = arg else {
        ed.status_msg = Some(USAGE.into());
        return;
    };
    let Some((scope, rest)) = arg.split_once(' ') else {
        ed.status_msg = Some(USAGE.into());
        return;
    };
    let Some((key, value)) = rest.split_once('=') else {
        ed.status_msg = Some("Expected key=value".into());
        return;
    };
    let result = match scope {
        "global" => crate::settings::apply_setting(
            crate::settings::SettingScope::Global,
            key, value, &mut ed.settings, &mut ed.doc.overrides,
        ),
        "buffer" => crate::settings::apply_setting(
            crate::settings::SettingScope::Buffer,
            key, value, &mut ed.settings, &mut ed.doc.overrides,
        ),
        _ => Err(format!("unknown scope '{scope}': expected 'global' or 'buffer'")),
    };
    if let Err(msg) = result {
        ed.status_msg = Some(msg);
    }
}

/// Serialize the buffer and write it to disk.
///
/// If `arg` is `Some(path)`, performs a save-as: writes to the specified
/// path and updates `ed.file_path` / `ed.file_meta` so that subsequent
/// `:w` (no argument) targets the same path.
///
/// If `arg` is `None`, writes to the current file. Errors with
/// "no file name" if the buffer is a scratch buffer with no path.
///
/// On success, calls `ed.doc.mark_saved()` and sets a status message.
/// Returns `true` on success, `false` on any error.
fn write_file(ed: &mut Editor, arg: Option<&str>) -> bool {
    let (content, line_count) = {
        let buf = ed.doc.buf();
        // The rope is always stored LF-normalized; restore CRLF for files that
        // originally used it so we don't silently change line endings on save.
        let content = if buf.line_ending() == crate::core::buffer::LineEnding::CrLf {
            buf.to_string().replace('\n', "\r\n")
        } else {
            buf.to_string()
        };
        // The buffer always ends with a structural '\n', so len_lines() returns
        // one more than the number of visible lines (ropey counts the empty
        // string after the final newline as an extra line).
        let line_count = buf.len_lines().saturating_sub(1);
        (content, line_count)
    };

    if let Some(path_str) = arg {
        // Save-as: write to the specified path.
        let path = std::path::Path::new(path_str);
        // Try to preserve existing file's permissions; if the file doesn't
        // exist yet, write_file_new creates it with default permissions.
        let result = match crate::io::read_file_meta(path) {
            Ok(meta) => crate::io::write_file_atomic(&content, &meta).map(|()| meta),
            Err(_)   => crate::io::write_file_new(&content, path),
        };
        match result {
            Ok(meta) => {
                // Store the canonicalized path so file_path and file_meta.resolved_path
                // always agree, even when the user supplied a relative or symlink path.
                ed.file_path = Some(Arc::new(meta.resolved_path.clone()));
                ed.file_meta = Some(meta);
                ed.doc.mark_saved();
                ed.status_msg = Some(format!("Written {line_count} lines"));
                true
            }
            Err(e) => {
                ed.status_msg = Some(format!("Error: {e}"));
                false
            }
        }
    } else {
        // Write to the current file.
        let Some(meta) = ed.file_meta.as_ref() else {
            ed.status_msg = Some("Error: no file name".into());
            return false;
        };
        match crate::io::write_file_atomic(&content, meta) {
            Ok(()) => {
                ed.doc.mark_saved();
                ed.status_msg = Some(format!("Written {line_count} lines"));
                true
            }
            Err(e) => {
                ed.status_msg = Some(format!("Error: {e}"));
                false
            }
        }
    }
}

// ── Jump list navigation ─────────────────────────────────────────────────────

pub(super) fn cmd_jump_backward(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    let current = crate::core::jump_list::JumpEntry::new(ed.doc.sels().clone(), ed.doc.buf());
    if let Some(entry) = ed.jump_list.backward(current) {
        let sels = entry.selections.clone();
        ed.doc.set_selections(sels);
    }
}

pub(super) fn cmd_jump_forward(ed: &mut Editor, _count: usize, _mode: MotionMode) {
    if let Some(entry) = ed.jump_list.forward() {
        let sels = entry.selections.clone();
        ed.doc.set_selections(sels);
    }
}
