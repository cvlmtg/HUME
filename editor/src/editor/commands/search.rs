use std::sync::Arc;

use crate::core::grapheme::next_grapheme_boundary;
use crate::core::search_state::SearchPattern;
use crate::core::selection::{Selection, SelectionSet};
use crate::helpers::is_word_boundary;
use crate::ops::MotionMode;
use crate::ops::register::SEARCH_REGISTER;
use crate::ops::search::{
    compile_search_regex, escape_regex, find_all_matches, find_match_from_cache, find_next_match,
};
use crate::ops::text_object::inner_word_impl;

use super::super::{MiniBuffer, Mode, SearchDirection};
use super::super::Editor;
use crate::core::error::CommandError;

// ── Search ────────────────────────────────────────────────────────────────────

/// Enter forward search mode.
///
/// Snapshots the current selections for cancel-restore, then opens the
/// mini-buffer with the `/` prompt.
pub fn cmd_search_forward(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let pre_sels = ed.current_selections().clone();
    let extend = ed.mode == engine::types::EditorMode::Extend;
    let pid = ed.focused_pane_id;
    ed.search.direction = SearchDirection::Forward;
    // Capture extend state before mode becomes Search — live search uses it.
    ed.pane_transient[pid].pre_search_sels = Some(pre_sels);
    ed.pane_transient[pid].search_extend = extend;
    ed.history.begin_session_all();
    ed.set_mode(Mode::Search);
    ed.minibuf = Some(MiniBuffer {
        prompt: '/',
        input: String::new(),
        cursor: 0,
    });
    Ok(())
}

/// Enter backward search mode.
pub fn cmd_search_backward(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let pre_sels = ed.current_selections().clone();
    let extend = ed.mode == engine::types::EditorMode::Extend;
    let pid = ed.focused_pane_id;
    ed.search.direction = SearchDirection::Backward;
    // Capture extend state before mode becomes Search — live search uses it.
    ed.pane_transient[pid].pre_search_sels = Some(pre_sels);
    ed.pane_transient[pid].search_extend = extend;
    ed.history.begin_session_all();
    ed.set_mode(Mode::Search);
    ed.minibuf = Some(MiniBuffer {
        prompt: '?',
        input: String::new(),
        cursor: 0,
    });
    Ok(())
}

/// Build the primary selection after a search match.
///
/// `anchor = Some(a)` — extend mode: keep the caller's anchor, move head to
/// the match edge that faces the search direction.
/// `anchor = None` — move mode: cover the matched text exactly.
pub fn search_sel(
    start: usize,
    end_incl: usize,
    anchor: Option<usize>,
    direction: SearchDirection,
) -> Selection {
    match anchor {
        Some(a) => Selection::new(
            a,
            match direction {
                SearchDirection::Forward => end_incl,
                SearchDirection::Backward => start,
            },
        ),
        None => Selection::new(start, end_incl),
    }
}

/// Ensure the focused buffer has an active search pattern, compiling from
/// `SEARCH_REGISTER` if needed. Returns `true` if a usable pattern is now
/// in place, `false` otherwise.
fn ensure_search_regex(ed: &mut Editor) -> bool {
    if ed.search_pattern().is_some() {
        return true;
    }
    let pattern = ed
        .registers
        .read(SEARCH_REGISTER)
        .and_then(|r| r.as_text().and_then(|v| v.first()).cloned())
        .unwrap_or_default();
    if pattern.is_empty() {
        return false;
    }
    match compile_search_regex(&pattern) {
        Some(r) => {
            let bid = ed.focused_buffer_id();
            ed.buffers.get_mut(bid).search_pattern = Some(SearchPattern {
                regex: Arc::new(r),
                pattern_str: pattern,
            });
            true
        }
        None => false,
    }
}

/// Shared body for `search-next` / `search-prev` / extend variants.
///
/// Reads the cached `search_regex` (compiled during the search session), or
/// recompiles from the `'s'` register if the cache is empty. Repeats `count`
/// times (e.g. `3n` jumps 3 matches forward). Moves or extends the primary
/// selection depending on `extend`.
fn search_jump(
    ed: &mut Editor,
    count: usize,
    direction: SearchDirection,
    mode: MotionMode,
) -> Result<(), CommandError> {
    if !ensure_search_regex(ed) {
        return Ok(());
    }

    let regex = {
        let bid = ed.focused_buffer_id();
        match ed.buffers.get(bid).search_pattern.as_ref() {
            Some(sp) => Arc::clone(&sp.regex),
            None => return Ok(()),
        }
    };

    // Capture anchor before the loop (extend mode keeps the original anchor fixed).
    let (mut from_char, anchor) = {
        let buf = ed.doc().text();
        let primary = ed.current_selections().primary();
        let from = match direction {
            // Step past the current match so we don't re-find it on the first jump.
            SearchDirection::Forward => next_grapheme_boundary(buf, primary.end_inclusive(buf)),
            SearchDirection::Backward => primary.start(),
        };
        (
            from,
            if mode == MotionMode::Extend {
                Some(primary.anchor)
            } else {
                None
            },
        )
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
    let bid = ed.focused_buffer_id();

    if !ed.buffers.get(bid).search_matches.matches.is_empty() {
        let cached_matches = &ed.buffers.get(bid).search_matches.matches;
        for _ in 0..count {
            match find_match_from_cache(cached_matches, from_char, direction) {
                Some((start, end_incl, wrapped)) => {
                    any_wrapped |= wrapped;
                    last_match = Some((start, end_incl));
                    from_char = match direction {
                        SearchDirection::Forward => {
                            next_grapheme_boundary(ed.doc().text(), end_incl)
                        }
                        SearchDirection::Backward => start,
                    };
                }
                None => {
                    last_match = None;
                    break;
                }
            }
        }
    } else {
        for _ in 0..count {
            match find_next_match(ed.doc().text(), &regex, from_char, direction) {
                Some((start, end_incl, wrapped)) => {
                    any_wrapped |= wrapped;
                    last_match = Some((start, end_incl));
                    from_char = match direction {
                        SearchDirection::Forward => {
                            next_grapheme_boundary(ed.doc().text(), end_incl)
                        }
                        SearchDirection::Backward => start,
                    };
                }
                None => {
                    last_match = None;
                    break;
                }
            }
        }
    }

    match last_match {
        Some((start, end_incl)) => {
            ed.current_search_cursor_mut().wrapped = any_wrapped;
            let new_sel = search_sel(start, end_incl, anchor, direction);
            ed.set_primary_selection(new_sel);
            Ok(())
        }
        None => Err(CommandError("no match".into())),
    }
}

/// Clear the active search regex and dismiss all match highlights.
///
/// Also invocable as `:clear-search` / `:cs` in command mode.
pub fn cmd_clear_search(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let bid = ed.focused_buffer_id();
    super::super::search_ops::clear_buffer_search(&mut ed.buffers, &mut ed.pane_state, bid);
    Ok(())
}

pub fn cmd_search_next(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    search_jump(ed, count, SearchDirection::Forward, mode)
}
pub fn cmd_search_prev(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    search_jump(ed, count, SearchDirection::Backward, mode)
}

// ── Select all matches ────────────────────────────────────────────────────────

/// Turn every search match in the buffer into a selection.
///
/// Uses the active search regex, falling back to recompiling from the `'s'`
/// register (same as `n`/`N`). If there is no active search, does nothing.
/// The first match becomes primary.
pub fn cmd_select_all_matches(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if !ensure_search_regex(ed) {
        return Ok(());
    }
    let bid = ed.focused_buffer_id();
    let regex = match ed.buffers.get(bid).search_pattern.as_ref() {
        Some(sp) => Arc::clone(&sp.regex),
        None => return Ok(()),
    };

    let matches = find_all_matches(ed.doc().text(), &regex);
    if matches.is_empty() {
        return Err(CommandError("no matches".into()));
    }

    let sels: Vec<Selection> = matches
        .into_iter()
        .map(|(s, e)| Selection::new(s, e))
        .collect();
    ed.set_current_selections(SelectionSet::from_vec(sels, 0));
    Ok(())
}

// ── Select within (s) ────────────────────────────────────────────────────────

/// Enter Select mode.
///
/// Snapshots the current selections for cancel-restore, then opens the
/// mini-buffer with the `s` prompt. The user types a regex; all matches
/// within the current selections become new selections (live preview).
pub fn cmd_select_within(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    // Nothing meaningful to search within a single-char selection.
    if ed
        .current_selections()
        .iter_sorted()
        .all(Selection::is_collapsed)
    {
        return Ok(());
    }
    let pre_sels = ed.current_selections().clone();
    let pid = ed.focused_pane_id;
    ed.pane_transient[pid].pre_select_sels = Some(pre_sels);
    ed.set_mode(Mode::Select);
    ed.minibuf = Some(MiniBuffer {
        prompt: '⫽',
        input: String::new(),
        cursor: 0,
    });
    Ok(())
}

// ── Use selection as search (*) ──────────────────────────────────────────────

/// Use the primary selection text as the search pattern.
///
/// If the primary selection is a cursor (1-char), expands to the word under
/// the cursor first (same as Helix). The escaped text is compiled as a search
/// regex, stored in the `'s'` register, and search highlights appear immediately.
pub fn cmd_use_selection_as_search(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let buf = ed.doc().text();
    let primary = ed.current_selections().primary();

    // If cursor (1-char selection), expand to inner word first.
    let (text, new_sel): (String, Option<Selection>) = if primary.is_collapsed() {
        let Some((start, end)) = inner_word_impl(buf, primary.head, is_word_boundary) else {
            return Ok(()); // cursor on structural newline or similar — nothing to do
        };
        let word_text = buf.slice(start..end + 1).to_string();
        (word_text, Some(Selection::new(start, end)))
    } else {
        let text = buf
            .slice(primary.start()..primary.end_inclusive(buf) + 1)
            .to_string();
        (text, None)
    };

    if text.is_empty() {
        return Ok(());
    }

    // Update the primary selection to cover the word (for cursor expansion).
    if let Some(sel) = new_sel {
        ed.set_primary_selection(sel);
    }

    let escaped = escape_regex(&text);
    let Some(regex) = compile_search_regex(&escaped) else {
        return Ok(());
    };

    // Store in search register and set as active search.
    ed.registers
        .write_text(SEARCH_REGISTER, vec![escaped.clone()]);
    ed.search.direction = SearchDirection::Forward;
    let bid = ed.focused_buffer_id();
    ed.buffers.get_mut(bid).search_pattern = Some(SearchPattern {
        regex: Arc::new(regex),
        pattern_str: escaped,
    });
    Ok(())
}
