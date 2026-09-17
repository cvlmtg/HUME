use std::sync::Arc;

use super::super::search::SearchPattern;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::word::{CharClass, is_word_boundary};
use hume_engine::pipeline::EngineView;
use hume_ops::MotionMode;
use hume_ops::search::{
    MatchScan, MatchSeed, SearchDirection, SearchFlags, find_all_matches, render_search_input,
    word_search_pattern,
};
use hume_ops::text_object::inner_word_impl;

use super::super::input_stack::{PaneSnapshot, SearchLayer, SiftLayer};
use super::super::{EditorState, MiniBuffer};
use super::{
    current_selections, doc, effective_word_chars, focused_buffer_id, search_pattern,
    set_current_selections, set_primary_selection,
};
use crate::editor::error::CommandError;

// ── Search ────────────────────────────────────────────────────────────────────

/// Shared body of `cmd_search_forward`/`cmd_search_backward` — opens the
/// mini-buffer with `prompt` (`/` or `?`); `SearchLayer::setup` snapshots
/// the current selections for cancel-restore once the layer lands (see its
/// own doc for why that capture happens there rather than here).
fn begin_search(
    state: &mut EditorState,
    view: &mut EngineView,
    direction: SearchDirection,
    prompt: &str,
) {
    let extend = state.mode() == hume_engine::types::EditorMode::Extend;
    let pane = state.focus.id();
    state.search.direction = direction;
    state.history.begin_session_all();
    state.push_mode_layer(
        view,
        SearchLayer {
            minibuf: MiniBuffer::new(prompt),
            snap: PaneSnapshot::new(pane),
            extend,
        },
    );
}

/// Enter forward search mode.
pub(in crate::editor) fn cmd_search_forward(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    begin_search(state, view, SearchDirection::Forward, "/");
    Ok(())
}

/// Enter backward search mode.
pub(in crate::editor) fn cmd_search_backward(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    begin_search(state, view, SearchDirection::Backward, "?");
    Ok(())
}

/// Ensure the focused buffer has an active search pattern.
fn ensure_search_regex(state: &mut EditorState, view: &EngineView) -> bool {
    if search_pattern(state, view).is_some() {
        return true;
    }
    let Some(pattern) = state.registers.search_register().filter(|p| !p.is_empty()) else {
        return false;
    };
    let Some(sp) = SearchPattern::compile(pattern) else {
        return false;
    };
    let bid = focused_buffer_id(state, view);
    state.buffers.get_mut(bid).search_pattern = Some(sp);
    true
}

/// Shared body for `search-next` / `search-prev` / extend variants.
///
/// Reads the cached `search_regex` (compiled during the search session), or
/// recompiles from the `'s'` register if the cache is empty. Repeats `count`
/// times (e.g. `3n` jumps 3 matches forward). Moves or extends the primary
/// selection depending on `extend` — or, when the pattern's `m` flag is set,
/// every selection independently. Both are `MatchScan::advance`/`advance_all`,
/// which also back live search's preview (`update_live_search` in
/// `input_stack/search.rs`) — this is the `PastSelection` seed, that one is
/// `AtSelection`.
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
    let sp = match state.buffers.get(bid).search_pattern.as_ref() {
        Some(sp) => sp,
        None => return Ok(()),
    };
    let regex = Arc::clone(&sp.regex);
    let multi = sp.multi();
    let matches = &state.buffers.get(bid).search_matches.matches;
    let scan = MatchScan {
        text: doc(state, view).text(),
        regex: &regex,
        cached: (!matches.is_empty()).then_some(matches.as_slice()),
        direction,
        mode,
        seed: MatchSeed::PastSelection,
    };

    if multi {
        let sels = current_selections(state, view).clone();
        let Some((new_sels, primary_wrapped)) = scan.advance_all(sels, count) else {
            return Err(CommandError::transient("no match"));
        };
        let pid = state.focus.id();
        state.panes.state[pid][bid].search_cursor.wrapped = primary_wrapped;
        set_current_selections(state, view, new_sels);
        return Ok(());
    }

    let primary = current_selections(state, view).primary();
    match scan.advance(primary, count) {
        Some((new_sel, wrapped)) => {
            let pid = state.focus.id();
            state.panes.state[pid][bid].search_cursor.wrapped = wrapped;
            set_primary_selection(state, view, new_sel);
            Ok(())
        }
        None => Err(CommandError::transient("no match")),
    }
}

/// Clear the active search regex and dismiss all match highlights.
pub(in crate::editor) fn cmd_clear_search(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let bid = focused_buffer_id(state, view);
    super::super::search::ops::clear_buffer_search(&mut state.buffers, &mut state.panes.state, bid);
    Ok(())
}

pub(in crate::editor) fn cmd_search_next(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    search_jump(state, view, count, SearchDirection::Forward, mode)
}
pub(in crate::editor) fn cmd_search_prev(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    search_jump(state, view, count, SearchDirection::Backward, mode)
}

// ── Select all matches ────────────────────────────────────────────────────────

pub(in crate::editor) fn cmd_select_all_matches(
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
        return Err(CommandError::transient("no matches"));
    }

    let sels: Vec<Selection> = matches
        .into_iter()
        .map(|span| Selection::new(span.start, span.end))
        .collect();
    set_current_selections(state, view, SelectionSet::from_vec(sels, 0));
    Ok(())
}

// ── Sift within (s) ──────────────────────────────────────────────────────────

pub(in crate::editor) fn cmd_sift_within(
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
    let pane = state.focus.id();
    // `SiftLayer::setup` snapshots the current selections once the layer
    // lands — see `SearchLayer::setup`'s doc for why the capture happens
    // there rather than here.
    state.push_mode_layer(
        view,
        SiftLayer {
            minibuf: MiniBuffer::new("⫽"),
            snap: PaneSnapshot::new(pane),
        },
    );
    Ok(())
}

// ── Search word under cursor (*) ─────────────────────────────────────────────

pub(in crate::editor) fn cmd_search_word_under_cursor(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let buf_id = focused_buffer_id(state, view);
    let chars = effective_word_chars(state.buffers.get(buf_id), &state.settings);
    let text = doc(state, view).text();
    let primary = current_selections(state, view).primary();

    // Always search the word under the head, regardless of any existing selection
    // (matches Vim: `*` targets the word under the cursor, not the visual selection).
    //
    // No-op on \n or whitespace — no word to search for. On \n, inner_word_impl
    // would otherwise expand the cursor to the adjacent \n run and set a useless
    // newline regex; on whitespace, it would expand to the whitespace run itself
    // and set a bare-space pattern (Vim instead scans to the nearest word — HUME
    // deliberately no-ops rather than adding that scan).
    match chars.classify(text.char_at(primary.head()).unwrap_or('\n')) {
        CharClass::Eol | CharClass::Space => return Ok(()),
        _ => {}
    }
    let Some(range) = inner_word_impl(text, primary.head(), is_word_boundary, chars) else {
        return Ok(());
    };
    let (start, end_incl) = (range.start, range.end);
    // Computed here (before set_primary_selection) so the immutable `text`/
    // `chars` borrows end before we mutably borrow state.
    let word = text.slice(range.to_exclusive()).to_string();
    let pattern = word_search_pattern(&word, chars);

    set_primary_selection(state, view, Selection::new(start, end_incl));

    set_search_pattern(state, view, SearchFlags::default(), &pattern)
}

// ── Search selection (Ctrl-/) ────────────────────────────────────────────────

/// Use the primary selection's literal text as the search pattern — unlike
/// `*`, no whole-word anchors and no word expansion. Selects the exact text
/// the user already highlighted, so `n`/`N` cycle its other occurrences
/// (Helix's `search_selection`).
pub(in crate::editor) fn cmd_search_selection(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let text = doc(state, view).text();
    let primary = current_selections(state, view).primary();
    let selected = primary.slice(text).to_string();

    // No-op on a bare structural newline (a collapsed cursor sitting on one) —
    // a raw `\n` pattern would match every line end, the same "useless
    // newline regex" `*` avoids above. A multi-char selection that merely
    // *contains* a newline (e.g. a whole-line selection) keeps the literal
    // semantics this command promises — only the single-newline case is
    // guarded.
    if selected == "\n" {
        return Ok(());
    }

    let flags = SearchFlags {
        multi: false,
        verbatim: true,
    };
    set_search_pattern(state, view, flags, &selected)
}

/// Compile `pattern` under `flags`, write the rendered flagged form to the
/// search register, and set it as the focused buffer's active search pattern
/// (forward direction). Shared tail of `*` and Ctrl-/ — both set the same
/// (register, direction, pattern) triple that live search sets on confirm;
/// the match-cache/highlights are rebuilt lazily per-frame regardless of
/// which path set the pattern.
///
/// Renders through `render_search_input` rather than compiling `pattern`
/// directly, so `SearchPattern::compile` stays the crate's one compilation
/// path — see `render_search_input`'s own doc for why the round trip is
/// safe for both of this function's callers.
fn set_search_pattern(
    state: &mut EditorState,
    view: &EngineView,
    flags: SearchFlags,
    pattern: &str,
) -> Result<(), CommandError> {
    let raw = render_search_input(flags, pattern);
    let Some(sp) = SearchPattern::compile(&raw) else {
        return Ok(());
    };
    state.registers.set_search_register(raw);
    state.search.direction = SearchDirection::Forward;
    let bid = focused_buffer_id(state, view);
    state.buffers.get_mut(bid).search_pattern = Some(sp);
    Ok(())
}
