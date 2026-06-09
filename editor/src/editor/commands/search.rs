use std::sync::Arc;

use editing::grapheme::next_grapheme_boundary;
use super::super::search_state::SearchPattern;
use editing::selection::{Selection, SelectionSet};
use editing::word::{CharClass, classify_char, is_word_boundary};
use engine::pipeline::EngineView;
use crate::ops::MotionMode;
use crate::ops::register::SEARCH_REGISTER;
use crate::ops::search::{
    compile_search_regex, escape_regex, find_all_matches, find_match_from_cache, find_next_match,
};
use crate::ops::text_object::inner_word_impl;

use super::super::{MiniBuffer, Mode, SearchDirection, EditorState};
use crate::editor::error::CommandError;
use super::{
    current_selections, doc, enqueue_mode_change, focused_buffer_id,
    search_pattern, set_current_selections, set_primary_selection,
};

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
    let extend = state.mode == engine::types::EditorMode::Extend;
    let pid = state.focused_pane_id;
    state.search.direction = SearchDirection::Forward;
    state.panes.transient[pid].pre_search_sels = Some(pre_sels);
    state.panes.transient[pid].search_extend = extend;
    state.history.begin_session_all();
    let old_mode = state.mode;
    state.mode = Mode::Search;
    enqueue_mode_change(state, old_mode, Mode::Search);
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
    let extend = state.mode == engine::types::EditorMode::Extend;
    let pid = state.focused_pane_id;
    state.search.direction = SearchDirection::Backward;
    state.panes.transient[pid].pre_search_sels = Some(pre_sels);
    state.panes.transient[pid].search_extend = extend;
    state.history.begin_session_all();
    let old_mode = state.mode;
    state.mode = Mode::Search;
    enqueue_mode_change(state, old_mode, Mode::Search);
    state.minibuf = Some(MiniBuffer {
        prompt: '?',
        input: String::new(),
        cursor: 0,
    });
    Ok(())
}

/// Build the primary selection after a search match.
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

    let (mut from_char, anchor) = {
        let buf = doc(state, view).text();
        let primary = current_selections(state, view).primary();
        let from = match direction {
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
    let old_mode = state.mode;
    state.mode = Mode::Select;
    enqueue_mode_change(state, old_mode, Mode::Select);
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

    let (text, new_sel): (String, Option<Selection>) = if primary.is_collapsed() {
        if classify_char(buf.char_at(primary.head()).unwrap_or('\n')) == CharClass::Eol {
            return Ok(());
        }
        let Some((start, end)) = inner_word_impl(buf, primary.head(), is_word_boundary) else {
            return Ok(());
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

    if let Some(sel) = new_sel {
        set_primary_selection(state, view, sel);
    }

    let escaped = escape_regex(&text);
    let Some(regex) = compile_search_regex(&escaped) else {
        return Ok(());
    };

    state.registers.write_text(SEARCH_REGISTER, vec![escaped.clone()]);
    state.search.direction = SearchDirection::Forward;
    let bid = focused_buffer_id(state, view);
    state.buffers.get_mut(bid).search_pattern = Some(SearchPattern {
        regex: Arc::new(regex),
        pattern_str: escaped,
    });
    Ok(())
}
