use std::sync::Arc;

use super::super::search_state::SearchPattern;
use crate::ops::MotionMode;
use crate::ops::register::SEARCH_REGISTER;
use crate::ops::search::{
    compile_search_regex, escape_regex, find_all_matches, find_match_from_cache, find_next_match,
};
use crate::ops::text_object::inner_word_impl;
use hume_editing::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::word::{CharClass, classify_char, is_word_boundary};
use hume_engine::pipeline::EngineView;

use super::super::{EditorState, MiniBuffer, Mode, SearchDirection};
use super::{
    current_selections, doc, focused_buffer_id, search_pattern, set_current_selections,
    set_primary_selection,
};
use crate::editor::error::CommandError;

// ── Search ────────────────────────────────────────────────────────────────────

/// Enter forward search mode.
///
/// Snapshots the current selections for cancel-restore, then opens the
/// mini-buffer with the `/` prompt.
pub fn cmd_search_forward(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let pre_sels = current_selections(state, view).clone();
    let extend = state.mode() == hume_engine::types::EditorMode::Extend;
    let pid = state.focused_pane_id;
    state.search.direction = SearchDirection::Forward;
    state.panes.transient[pid].pre_search_sels = Some(pre_sels);
    state.panes.transient[pid].search_extend = extend;
    state.history.begin_session_all();
    state.set_mode(Mode::Search);
    state.minibuf = Some(MiniBuffer {
        prompt: '/',
        input: String::new(),
        cursor: 0,
    });
    Ok(())
}

/// Enter backward search mode.
pub fn cmd_search_backward(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let pre_sels = current_selections(state, view).clone();
    let extend = state.mode() == hume_engine::types::EditorMode::Extend;
    let pid = state.focused_pane_id;
    state.search.direction = SearchDirection::Backward;
    state.panes.transient[pid].pre_search_sels = Some(pre_sels);
    state.panes.transient[pid].search_extend = extend;
    state.history.begin_session_all();
    state.set_mode(Mode::Search);
    state.minibuf = Some(MiniBuffer {
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

/// Ensure the focused buffer has an active search pattern.
fn ensure_search_regex(state: &mut EditorState, view: &EngineView) -> bool {
    if search_pattern(state, view).is_some() {
        return true;
    }
    let pattern = state
        .registers
        .read(SEARCH_REGISTER)
        .and_then(|r| r.as_text().and_then(|v| v.first()).cloned())
        .unwrap_or_default();
    if pattern.is_empty() {
        return false;
    }
    match compile_search_regex(&pattern) {
        Some(r) => {
            let bid = focused_buffer_id(state, view);
            state.buffers.get_mut(bid).search_pattern = Some(SearchPattern {
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
    state: &mut EditorState,
    view: &EngineView,
    count: usize,
    direction: SearchDirection,
    mode: MotionMode,
) -> Result<(), CommandError> {
    if !ensure_search_regex(state, view) {
        return Ok(());
    }

    let bid = focused_buffer_id(state, view);
    let regex = match state.buffers.get(bid).search_pattern.as_ref() {
        Some(sp) => Arc::clone(&sp.regex),
        None => return Ok(()),
    };

    // Capture anchor before the loop (extend mode keeps the original anchor fixed).
    let (mut from_char, anchor) = {
        let buf = doc(state, view).text();
        let primary = current_selections(state, view).primary();
        let from = match direction {
            // Step past the current match so we don't re-find it on the first jump.
            SearchDirection::Forward => next_grapheme_boundary(buf, primary.end_inclusive(buf)),
            SearchDirection::Backward => primary.start(),
        };
        (
            from,
            if mode == MotionMode::Extend {
                Some(primary.anchor())
            } else {
                None
            },
        )
    };

    let count = count.max(1);
    let mut last_match: Option<(usize, usize)> = None;
    let mut any_wrapped = false;

    // When the match cache is populated we binary-search it (O(log M) per
    // jump). When it is empty — e.g. the very first `n` after startup before
    // the cache is warmed — we fall back to the O(buffer) regex-scan path.
    if !state.buffers.get(bid).search_matches.matches.is_empty() {
        let cached_matches = &state.buffers.get(bid).search_matches.matches;
        for _ in 0..count {
            match find_match_from_cache(cached_matches, from_char, direction) {
                Some((start, end_incl, wrapped)) => {
                    any_wrapped |= wrapped;
                    last_match = Some((start, end_incl));
                    from_char = match direction {
                        SearchDirection::Forward => {
                            next_grapheme_boundary(doc(state, view).text(), end_incl)
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
            match find_next_match(doc(state, view).text(), &regex, from_char, direction) {
                Some((start, end_incl, wrapped)) => {
                    any_wrapped |= wrapped;
                    last_match = Some((start, end_incl));
                    from_char = match direction {
                        SearchDirection::Forward => {
                            next_grapheme_boundary(doc(state, view).text(), end_incl)
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
            let pid = state.focused_pane_id;
            state.panes.state[pid][bid].search_cursor.wrapped = any_wrapped;
            let new_sel = search_sel(start, end_incl, anchor, direction);
            set_primary_selection(state, view, new_sel);
            Ok(())
        }
        None => Err(CommandError::new("no match")),
    }
}

/// Clear the active search regex and dismiss all match highlights.
pub fn cmd_clear_search(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let bid = focused_buffer_id(state, view);
    super::super::search_ops::clear_buffer_search(&mut state.buffers, &mut state.panes.state, bid);
    Ok(())
}

pub fn cmd_search_next(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    search_jump(state, view, count, SearchDirection::Forward, mode)
}
pub fn cmd_search_prev(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    search_jump(state, view, count, SearchDirection::Backward, mode)
}

// ── Select all matches ────────────────────────────────────────────────────────

pub fn cmd_select_all_matches(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if !ensure_search_regex(state, view) {
        return Ok(());
    }
    let bid = focused_buffer_id(state, view);
    let regex = match state.buffers.get(bid).search_pattern.as_ref() {
        Some(sp) => Arc::clone(&sp.regex),
        None => return Ok(()),
    };

    let matches = find_all_matches(doc(state, view).text(), &regex);
    if matches.is_empty() {
        return Err(CommandError::new("no matches"));
    }

    let sels: Vec<Selection> = matches
        .into_iter()
        .map(|(s, e)| Selection::new(s, e))
        .collect();
    set_current_selections(state, view, SelectionSet::from_vec(sels, 0));
    Ok(())
}

// ── Select within (s) ────────────────────────────────────────────────────────

pub fn cmd_select_within(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if current_selections(state, view)
        .iter_sorted()
        .all(Selection::is_collapsed)
    {
        return Ok(());
    }
    let pre_sels = current_selections(state, view).clone();
    let pid = state.focused_pane_id;
    state.panes.transient[pid].pre_select_sels = Some(pre_sels);
    state.set_mode(Mode::Select);
    state.minibuf = Some(MiniBuffer {
        prompt: '⫽',
        input: String::new(),
        cursor: 0,
    });
    Ok(())
}

// ── Use selection as search (*) ──────────────────────────────────────────────

pub fn cmd_use_selection_as_search(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let buf = doc(state, view).text();
    let primary = current_selections(state, view).primary();

    // If cursor (1-char selection), expand to inner word first.
    let (text, start, end_incl, new_sel): (String, usize, usize, Option<Selection>) =
        if primary.is_collapsed() {
            // Noop on \n — no word to search for (matches Vim/Helix behaviour).
            // inner_word_impl would otherwise expand the cursor to the adjacent \n
            // run and set a useless newline regex.
            if classify_char(buf.char_at(primary.head()).unwrap_or('\n')) == CharClass::Eol {
                return Ok(());
            }
            let Some((start, end)) = inner_word_impl(buf, primary.head(), is_word_boundary) else {
                return Ok(());
            };
            let word_text = buf.slice(start..end + 1).to_string();
            (word_text, start, end, Some(Selection::new(start, end)))
        } else {
            let start = primary.start();
            let end_incl = primary.end_inclusive(buf);
            let text = buf.slice(start..end_incl + 1).to_string();
            (text, start, end_incl, None)
        };

    if text.is_empty() {
        return Ok(());
    }

    // Wrap in `\b…\b` when both edges are Word-class characters, matching Vim's
    // whole-word `*` behaviour. Classifying via grapheme bases (not raw chars) keeps
    // combining sequences (e.g. e + U+0301) correct: prev_grapheme_boundary gives
    // the start of the final grapheme so we classify its base codepoint, not a
    // combining mark. Punctuation runs and mixed-class selections stay literal.
    //
    // Computed here (before set_primary_selection) so the immutable `buf` borrow
    // ends before we mutably borrow state.
    let first_class = classify_char(buf.char_at(start).unwrap_or('\n'));
    let last_base = prev_grapheme_boundary(buf, end_incl + 1);
    let last_class = classify_char(buf.char_at(last_base).unwrap_or('\n'));
    let whole_word = first_class == CharClass::Word && last_class == CharClass::Word;

    if let Some(sel) = new_sel {
        set_primary_selection(state, view, sel);
    }

    let escaped = escape_regex(&text);
    let pattern = if whole_word {
        format!(r"\b{escaped}\b")
    } else {
        escaped
    };
    let Some(regex) = compile_search_regex(&pattern) else {
        return Ok(());
    };

    state
        .registers
        .write_text(SEARCH_REGISTER, vec![pattern.clone()]);
    state.search.direction = SearchDirection::Forward;
    let bid = focused_buffer_id(state, view);
    state.buffers.get_mut(bid).search_pattern = Some(SearchPattern {
        regex: Arc::new(regex),
        pattern_str: pattern,
    });
    Ok(())
}
