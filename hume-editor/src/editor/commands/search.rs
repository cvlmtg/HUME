use std::sync::Arc;

use super::super::search::SearchPattern;
use hume_editing::grapheme::next_grapheme_boundary;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::word::{CharClass, classify_char, is_word_boundary};
use hume_engine::pipeline::EngineView;
use hume_ops::MotionMode;
use hume_ops::search::{
    SearchDirection, compile_search_regex, escape_regex, find_all_matches, find_match_from_cache,
    find_next_match,
};
use hume_ops::text_object::inner_word_impl;

use super::super::{EditorState, MiniBuffer, Mode};
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
pub(crate) fn cmd_search_forward(
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
        prompt: "/".to_string(),
        input: String::new(),
        cursor: 0,
    });
    Ok(())
}

/// Enter backward search mode.
pub(crate) fn cmd_search_backward(
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
        prompt: "?".to_string(),
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
pub(crate) fn search_sel(
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
    let Some(pattern) = state
        .registers
        .search_register()
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
    else {
        return false;
    };
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
pub(crate) fn cmd_clear_search(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let bid = focused_buffer_id(state, view);
    super::super::search::ops::clear_buffer_search(&mut state.buffers, &mut state.panes.state, bid);
    Ok(())
}

pub(crate) fn cmd_search_next(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    search_jump(state, view, count, SearchDirection::Forward, mode)
}
pub(crate) fn cmd_search_prev(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    search_jump(state, view, count, SearchDirection::Backward, mode)
}

// ── Select all matches ────────────────────────────────────────────────────────

pub(crate) fn cmd_select_all_matches(
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

pub(crate) fn cmd_select_within(
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
        prompt: "⫽".to_string(),
        input: String::new(),
        cursor: 0,
    });
    Ok(())
}

// ── Search word under cursor (*) ─────────────────────────────────────────────

pub(crate) fn cmd_search_word_under_cursor(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let buf = doc(state, view).text();
    let primary = current_selections(state, view).primary();

    // Always search the word under the head, regardless of any existing selection
    // (matches Vim: `*` targets the word under the cursor, not the visual selection).
    //
    // No-op on \n or whitespace — no word to search for. On \n, inner_word_impl
    // would otherwise expand the cursor to the adjacent \n run and set a useless
    // newline regex; on whitespace, it would expand to the whitespace run itself
    // and set a bare-space pattern (Vim instead scans to the nearest word — HUME
    // deliberately no-ops rather than adding that scan).
    match classify_char(buf.char_at(primary.head()).unwrap_or('\n')) {
        CharClass::Eol | CharClass::Space => return Ok(()),
        _ => {}
    }
    let Some((start, end_incl)) = inner_word_impl(buf, primary.head(), is_word_boundary) else {
        return Ok(());
    };
    let text = buf.slice(start..end_incl + 1).to_string();

    // Wrap in `\b…\b` when the run is Word-class, matching Vim's whole-word `*`
    // behaviour. inner_word_impl returns a uniform same-class run, so checking the
    // first character's class is sufficient — the last character is guaranteed to
    // share it. Punctuation runs stay literal.
    //
    // Computed here (before set_primary_selection) so the immutable `buf` borrow
    // ends before we mutably borrow state.
    let whole_word = classify_char(buf.char_at(start).unwrap_or('\n')) == CharClass::Word;

    set_primary_selection(state, view, Selection::new(start, end_incl));

    let escaped = escape_regex(&text);
    let pattern = if whole_word {
        format!(r"\b{escaped}\b")
    } else {
        escaped
    };
    set_search_pattern(state, view, pattern)
}

// ── Search selection (Ctrl+/) ────────────────────────────────────────────────

/// Use the primary selection's literal text as the search pattern — unlike
/// `*`, no whole-word anchors and no word expansion. Selects the exact text
/// the user already highlighted, so `n`/`N` cycle its other occurrences
/// (Helix's `search_selection`).
pub(crate) fn cmd_search_selection(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let buf = doc(state, view).text();
    let primary = current_selections(state, view).primary();
    let text = buf
        .slice(primary.start()..primary.end_inclusive(buf) + 1)
        .to_string();

    // No-op on a bare structural newline (a collapsed cursor sitting on one) —
    // a raw `\n` pattern would match every line end, the same "useless
    // newline regex" `*` avoids above. A multi-char selection that merely
    // *contains* a newline (e.g. a whole-line selection) keeps the literal
    // semantics this command promises — only the single-newline case is
    // guarded.
    if text == "\n" {
        return Ok(());
    }

    let pattern = escape_regex(&text);
    set_search_pattern(state, view, pattern)
}

/// Compile `pattern`, write it to the search register, and set it as the
/// focused buffer's active search pattern (forward direction). Shared tail
/// of `*` and Ctrl+/ — both set the same (register, direction, pattern)
/// triple that live search sets on confirm; the match-cache/highlights are
/// rebuilt lazily per-frame regardless of which path set the pattern.
fn set_search_pattern(
    state: &mut EditorState,
    view: &EngineView,
    pattern: String,
) -> Result<(), CommandError> {
    let Some(regex) = compile_search_regex(&pattern) else {
        return Ok(());
    };
    state.registers.set_search_register(pattern.clone());
    state.search.direction = SearchDirection::Forward;
    let bid = focused_buffer_id(state, view);
    state.buffers.get_mut(bid).search_pattern = Some(SearchPattern {
        regex: Arc::new(regex),
        pattern_str: pattern,
    });
    Ok(())
}
