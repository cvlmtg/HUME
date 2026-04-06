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
    cmd_goto_first_nonblank, cmd_goto_line_end, cmd_goto_line_start, cmd_move_left, cmd_move_right,
    find_char_backward, find_char_forward, MotionMode,
};
use crate::ops::register::{yank_selections, DEFAULT_REGISTER, SEARCH_REGISTER};
use crate::ops::search::{compile_search_regex, escape_regex, find_match_from_cache, find_next_match};
use crate::ops::selection_cmd::cmd_collapse_selection;
use crate::ops::text_object::inner_word_impl;
use crate::helpers::is_word_boundary;

use engine::types::EditorMode;

use super::registry::MappableCommand;
use super::{Editor, FindChar, FindKind, MiniBuffer, Mode, SearchDirection};

// ── Mode transitions ──────────────────────────────────────────────────────────

pub(super) fn cmd_insert_before(ed: &mut Editor, _count: usize) {
    ed.apply_motion(|_b, sels| sels.map(|s| Selection::collapsed(s.start())));
    ed.begin_insert_session();
}

pub(super) fn cmd_insert_after(ed: &mut Editor, _count: usize) {
    ed.apply_motion(|b, s| cmd_move_right(b, s, 1));
    ed.begin_insert_session();
}

pub(super) fn cmd_insert_at_line_start(ed: &mut Editor, _count: usize) {
    ed.apply_motion(|b, s| cmd_goto_first_nonblank(b, s, 1));
    ed.begin_insert_session();
}

pub(super) fn cmd_insert_at_line_end(ed: &mut Editor, _count: usize) {
    ed.apply_motion(|b, s| cmd_goto_line_end(b, s, 1));
    ed.apply_motion(|b, s| cmd_move_right(b, s, 1));
    ed.begin_insert_session();
}

/// Open a new line below the cursor and enter insert mode.
///
/// `begin_insert_session` opens the edit group so the structural `\n` and
/// everything typed before Esc form one undo step — the same pattern as
/// `cmd_change`.
pub(super) fn cmd_open_line_below(ed: &mut Editor, _count: usize) {
    ed.begin_insert_session();
    ed.apply_motion(|b, s| cmd_goto_line_end(b, s, 1));
    ed.apply_motion(|b, s| cmd_move_right(b, s, 1));
    ed.doc.apply_edit_grouped(|b, s| insert_char(b, s, '\n'));
}

/// Open a new line above the cursor and enter insert mode.
pub(super) fn cmd_open_line_above(ed: &mut Editor, _count: usize) {
    ed.begin_insert_session();
    ed.apply_motion(|b, s| cmd_goto_line_start(b, s, 1));
    ed.doc.apply_edit_grouped(|b, s| insert_char(b, s, '\n'));
    ed.apply_motion(|b, s| cmd_move_left(b, s, 1));
}

pub(super) fn cmd_command_mode(ed: &mut Editor, _count: usize) {
    ed.set_mode(Mode::Command);
    ed.minibuf = Some(MiniBuffer { prompt: ':', input: String::new(), cursor: 0 });
}

pub(super) fn cmd_exit_insert(ed: &mut Editor, _count: usize) {
    ed.end_insert_session();
}

// ── Edit composites ───────────────────────────────────────────────────────────

/// Yank selections into the default register, then delete them.
pub(super) fn cmd_delete(ed: &mut Editor, _count: usize) {
    let yanked = yank_selections(ed.doc.buf(), ed.doc.sels());
    ed.doc.apply_edit(delete_selection);
    ed.registers.write(DEFAULT_REGISTER, yanked);
}

/// Yank, delete, then enter insert mode — all in one undo group.
///
/// `begin_insert_session` opens the group so the delete and everything typed
/// before Esc form one undo step.
pub(super) fn cmd_change(ed: &mut Editor, _count: usize) {
    let yanked = yank_selections(ed.doc.buf(), ed.doc.sels());
    ed.begin_insert_session();
    ed.doc.apply_edit_grouped(delete_selection);
    ed.registers.write(DEFAULT_REGISTER, yanked);
}

/// Yank selections into the default register without deleting.
pub(super) fn cmd_yank(ed: &mut Editor, _count: usize) {
    let yanked = yank_selections(ed.doc.buf(), ed.doc.sels());
    ed.registers.write(DEFAULT_REGISTER, yanked);
}

/// Shared body for paste commands: read the default register, run `paste_fn`,
/// then write displaced text back if any selection was non-cursor (replace-and-swap).
fn do_paste(
    ed: &mut Editor,
    paste_fn: impl Fn(Buffer, SelectionSet, &[String]) -> (Buffer, SelectionSet, crate::core::changeset::ChangeSet, Vec<String>),
) {
    if let Some(reg) = ed.registers.read(DEFAULT_REGISTER) {
        let values = reg.values().to_vec();
        let displaced = ed.doc.apply_edit(|b, s| paste_fn(b, s, &values));
        if displaced.iter().any(|s| !s.is_empty()) {
            ed.registers.write(DEFAULT_REGISTER, displaced);
        }
    }
}

/// Paste after the selection; swap displaced text back into the register when
/// the selection was non-cursor (replace-and-swap semantics).
pub(super) fn cmd_paste_after(ed: &mut Editor, _count: usize) {
    use crate::ops::edit::paste_after;
    do_paste(ed, paste_after);
}

/// Paste before the selection; same replace-and-swap semantics as `cmd_paste_after`.
pub(super) fn cmd_paste_before(ed: &mut Editor, _count: usize) {
    use crate::ops::edit::paste_before;
    do_paste(ed, paste_before);
}

pub(super) fn cmd_undo(ed: &mut Editor, _count: usize) {
    ed.doc.undo();
}

pub(super) fn cmd_redo(ed: &mut Editor, _count: usize) {
    ed.doc.redo();
}

// ── Selection state ───────────────────────────────────────────────────────────

pub(super) fn cmd_toggle_extend(ed: &mut Editor, _count: usize) {
    ed.mode = if ed.mode == EditorMode::Extend {
        EditorMode::Normal
    } else {
        EditorMode::Extend
    };
}

/// Collapse each selection to its cursor AND exit extend mode.
///
/// Collapsing is a "done selecting" signal, so extend mode is always cleared.
pub(super) fn cmd_collapse_and_exit_extend(ed: &mut Editor, _count: usize) {
    // Mode is SSOT for extend state; setting Normal implicitly clears Extend.
    ed.mode = EditorMode::Normal;
    ed.apply_motion(cmd_collapse_selection);
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

pub(super) fn cmd_find_forward(ed: &mut Editor, count: usize) {
    find_char(ed, count, MotionMode::Move, FindKind::Inclusive, find_char_forward);
}
pub(super) fn cmd_extend_find_forward(ed: &mut Editor, count: usize) {
    find_char(ed, count, MotionMode::Extend, FindKind::Inclusive, find_char_forward);
}
pub(super) fn cmd_find_backward(ed: &mut Editor, count: usize) {
    find_char(ed, count, MotionMode::Move, FindKind::Inclusive, find_char_backward);
}
pub(super) fn cmd_extend_find_backward(ed: &mut Editor, count: usize) {
    find_char(ed, count, MotionMode::Extend, FindKind::Inclusive, find_char_backward);
}
pub(super) fn cmd_till_forward(ed: &mut Editor, count: usize) {
    find_char(ed, count, MotionMode::Move, FindKind::Exclusive, find_char_forward);
}
pub(super) fn cmd_extend_till_forward(ed: &mut Editor, count: usize) {
    find_char(ed, count, MotionMode::Extend, FindKind::Exclusive, find_char_forward);
}
pub(super) fn cmd_till_backward(ed: &mut Editor, count: usize) {
    find_char(ed, count, MotionMode::Move, FindKind::Exclusive, find_char_backward);
}
pub(super) fn cmd_extend_till_backward(ed: &mut Editor, count: usize) {
    find_char(ed, count, MotionMode::Extend, FindKind::Exclusive, find_char_backward);
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

pub(super) fn cmd_repeat_find_forward(ed: &mut Editor, count: usize) {
    repeat_find(ed, count, MotionMode::Move, find_char_forward);
}
pub(super) fn cmd_extend_repeat_find_forward(ed: &mut Editor, count: usize) {
    repeat_find(ed, count, MotionMode::Extend, find_char_forward);
}
pub(super) fn cmd_repeat_find_backward(ed: &mut Editor, count: usize) {
    repeat_find(ed, count, MotionMode::Move, find_char_backward);
}
pub(super) fn cmd_extend_repeat_find_backward(ed: &mut Editor, count: usize) {
    repeat_find(ed, count, MotionMode::Extend, find_char_backward);
}

// ── Replace ───────────────────────────────────────────────────────────────────

/// Replace every character in each selection with the next typed character.
///
/// Reads the replacement character from `ed.pending_char`.
pub(super) fn cmd_replace(ed: &mut Editor, _count: usize) {
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
pub(super) fn cmd_repeat(ed: &mut Editor, count: usize) {
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
    ed.execute_keymap_command(action.command, effective_count, false);

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

// ── Page scroll ───────────────────────────────────────────────────────────────
//
// Uses `view.height` as the move count rather than the user's numeric prefix.

/// Shared implementation for the four page-scroll commands.
fn page_scroll(ed: &mut Editor, motion_name: &str) {
    let page = ed.viewport().height as usize;
    let Some(MappableCommand::Motion { fun, .. }) = ed.registry.get(motion_name).copied() else {
        unreachable!("page_scroll: motion '{}' not in registry", motion_name);
    };
    ed.apply_motion(|b, s| fun(b, s, page));
}

pub(super) fn cmd_page_down(ed: &mut Editor, _count: usize) {
    page_scroll(ed, "move-down");
}
pub(super) fn cmd_extend_page_down(ed: &mut Editor, _count: usize) {
    page_scroll(ed, "extend-down");
}
pub(super) fn cmd_page_up(ed: &mut Editor, _count: usize) {
    page_scroll(ed, "move-up");
}
pub(super) fn cmd_extend_page_up(ed: &mut Editor, _count: usize) {
    page_scroll(ed, "extend-up");
}

// ── Half-page scroll ─────────────────────────────────────────────────────────
//
// Like page scroll but uses `view.height / 2` as the move count.

/// Shared implementation for the four half-page-scroll commands.
fn half_page_scroll(ed: &mut Editor, motion_name: &str) {
    let half = (ed.viewport().height as usize / 2).max(1);
    let Some(MappableCommand::Motion { fun, .. }) = ed.registry.get(motion_name).copied() else {
        unreachable!("half_page_scroll: motion '{}' not in registry", motion_name);
    };
    ed.apply_motion(|b, s| fun(b, s, half));
}

pub(super) fn cmd_half_page_down(ed: &mut Editor, _count: usize) {
    half_page_scroll(ed, "move-down");
}
pub(super) fn cmd_extend_half_page_down(ed: &mut Editor, _count: usize) {
    half_page_scroll(ed, "extend-down");
}
pub(super) fn cmd_half_page_up(ed: &mut Editor, _count: usize) {
    half_page_scroll(ed, "move-up");
}
pub(super) fn cmd_extend_half_page_up(ed: &mut Editor, _count: usize) {
    half_page_scroll(ed, "extend-up");
}

// ── Search ────────────────────────────────────────────────────────────────────

/// Enter forward search mode.
///
/// Snapshots the current selections for cancel-restore, then opens the
/// mini-buffer with the `/` prompt.
pub(super) fn cmd_search_forward(ed: &mut Editor, _count: usize) {
    ed.search.pre_search_sels = Some(ed.doc.sels().clone());
    ed.search.direction = SearchDirection::Forward;
    // Capture extend state before mode becomes Search — live search uses it.
    ed.search.extend = ed.mode == EditorMode::Extend;
    ed.set_mode(Mode::Search);
    ed.minibuf = Some(MiniBuffer { prompt: '/', input: String::new(), cursor: 0 });
}

/// Enter backward search mode.
pub(super) fn cmd_search_backward(ed: &mut Editor, _count: usize) {
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

/// Shared body for `search-next` / `search-prev` / extend variants.
///
/// Reads the cached `search_regex` (compiled during the search session), or
/// recompiles from the `'s'` register if the cache is empty. Repeats `count`
/// times (e.g. `3n` jumps 3 matches forward). Moves or extends the primary
/// selection depending on `extend`.
fn search_jump(ed: &mut Editor, count: usize, direction: SearchDirection, extend: bool) {
    // Ensure search.regex is populated — compile from the 's' register if needed.
    if ed.search.regex.is_none() {
        let pattern = ed
            .registers
            .read(SEARCH_REGISTER)
            .and_then(|r| r.values().first().cloned())
            .unwrap_or_default();
        if pattern.is_empty() {
            return;
        }
        match compile_search_regex(&pattern) {
            Some(r) => ed.search.set_regex(Some(r)),
            None => return,
        }
    }
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
        (from, if extend { Some(primary.anchor) } else { None })
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
pub(super) fn cmd_clear_search(ed: &mut Editor, _count: usize) {
    ed.search.clear();
    // update_search_cache() is called by the event loop after handle_key returns,
    // but search.clear() already zeroes the cache fields directly, so the render
    // path sees a clean state immediately regardless of event-loop ordering.
}

pub(super) fn cmd_search_next(ed: &mut Editor, count: usize) {
    search_jump(ed, count, SearchDirection::Forward, false);
}
pub(super) fn cmd_extend_search_next(ed: &mut Editor, count: usize) {
    search_jump(ed, count, SearchDirection::Forward, true);
}
pub(super) fn cmd_search_prev(ed: &mut Editor, count: usize) {
    search_jump(ed, count, SearchDirection::Backward, false);
}
pub(super) fn cmd_extend_search_prev(ed: &mut Editor, count: usize) {
    search_jump(ed, count, SearchDirection::Backward, true);
}

// ── Select within (s) ────────────────────────────────────────────────────────

/// Enter Select mode.
///
/// Snapshots the current selections for cancel-restore, then opens the
/// mini-buffer with the `s` prompt. The user types a regex; all matches
/// within the current selections become new selections (live preview).
pub(super) fn cmd_select_within(ed: &mut Editor, _count: usize) {
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
pub(super) fn cmd_use_selection_as_search(ed: &mut Editor, _count: usize) {
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
    ed.registers.write(SEARCH_REGISTER, vec![escaped]);
    ed.search.direction = SearchDirection::Forward;
    ed.search.set_regex(Some(regex));
}

// ── Misc ──────────────────────────────────────────────────────────────────────

pub(super) fn cmd_quit(ed: &mut Editor, _count: usize) {
    ed.should_quit = true;
}

// ── Typed commands (command-mode `:` dispatch) ───────────────────────────────
//
// Each typed command has a canonical name and zero or more short aliases.
// Follows the Helix model: no prefix matching, exact match on name or alias.
// Tab-completion (future) will filter by prefix for discoverability.

/// A command-mode (`:`) command with a canonical name and optional short aliases.
pub(super) struct TypedCommand {
    /// Canonical name, e.g. `"write"`.
    pub name: &'static str,
    /// Short aliases, e.g. `&["w"]`. Empty slice for commands with no alias.
    pub aliases: &'static [&'static str],
    /// One-line description shown in help/completion (unused until tab-completion is built).
    #[allow(dead_code)]
    pub doc: &'static str,
    /// The function to execute. Receives the editor, an optional argument
    /// (e.g. file path for `:w`), and whether `!` was appended.
    pub fun: fn(&mut Editor, Option<&str>, bool),
}

/// Static table of all typed commands.
pub(super) static TYPED_COMMANDS: &[TypedCommand] = &[
    TypedCommand {
        name: "quit",
        aliases: &["q"],
        doc: "Close the editor",
        fun: typed_quit,
    },
    TypedCommand {
        name: "write",
        aliases: &["w"],
        doc: "Write changes to disk",
        fun: typed_write,
    },
    TypedCommand {
        name: "write-quit",
        aliases: &["wq"],
        doc: "Write changes and quit",
        fun: typed_write_quit,
    },
    TypedCommand {
        name: "clear-search",
        aliases: &["cs"],
        doc: "Clear search highlights",
        fun: typed_clear_search,
    },
    TypedCommand {
        name: "toggle-soft-wrap",
        aliases: &["wrap"],
        doc: "Toggle soft line wrapping",
        fun: typed_toggle_soft_wrap,
    },
];

/// Look up a typed command by exact name or alias.
pub(super) fn find_typed_command(name: &str) -> Option<&'static TypedCommand> {
    TYPED_COMMANDS.iter().find(|tc| tc.name == name || tc.aliases.contains(&name))
}

// ── Typed command implementations ────────────────────────────────────────────

fn typed_quit(ed: &mut Editor, _arg: Option<&str>, force: bool) {
    if !force && ed.doc.is_dirty() {
        ed.status_msg = Some("Unsaved changes (add ! to override)".into());
    } else {
        ed.should_quit = true;
    }
}

fn typed_write(ed: &mut Editor, arg: Option<&str>, force: bool) {
    if force {
        ed.status_msg = Some("Error: w! is not supported".into());
    } else {
        write_file(ed, arg);
    }
}

fn typed_write_quit(ed: &mut Editor, arg: Option<&str>, force: bool) {
    // force applies to the quit part: quit even if the write fails.
    if write_file(ed, arg) || force {
        ed.should_quit = true;
    }
}

fn typed_clear_search(ed: &mut Editor, _arg: Option<&str>, _force: bool) {
    cmd_clear_search(ed, 0);
}

fn typed_toggle_soft_wrap(ed: &mut Editor, _arg: Option<&str>, _force: bool) {
    use engine::pane::WrapMode;
    let pane = ed.pane_mut();
    let currently_wrapping = pane.wrap_mode.is_wrapping();
    if currently_wrapping {
        pane.wrap_mode = WrapMode::None;
        // Horizontal offset is now meaningful; scroll stays where it is.
    } else {
        // Estimate gutter width (line numbers + separator). The engine will
        // compute the exact width at render time; this just needs to be close
        // enough for a reasonable default wrap column.
        const GUTTER_WIDTH_ESTIMATE: u16 = 4;
        let content_w = pane.viewport.width.saturating_sub(GUTTER_WIDTH_ESTIMATE).max(1);
        pane.wrap_mode = WrapMode::Indent { width: content_w };
        pane.viewport.horizontal_offset = 0;
        pane.viewport.top_row_offset = 0;
    }
    let state = if currently_wrapping { "off" } else { "on" };
    ed.status_msg = Some(format!("Soft wrap {state}"));
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

pub(super) fn cmd_jump_backward(ed: &mut Editor, _count: usize) {
    let current = crate::core::jump_list::JumpEntry::new(ed.doc.sels().clone(), ed.doc.buf());
    if let Some(entry) = ed.jump_list.backward(current) {
        let sels = entry.selections.clone();
        ed.doc.set_selections(sels);
    }
}

pub(super) fn cmd_jump_forward(ed: &mut Editor, _count: usize) {
    if let Some(entry) = ed.jump_list.forward() {
        let sels = entry.selections.clone();
        ed.doc.set_selections(sels);
    }
}
