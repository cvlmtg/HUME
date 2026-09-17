//! The `Search` layer — the `/`/`?`-prompt minibuffer mode.

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;
use hume_ops::MotionMode;
use hume_ops::search::{MatchScan, MatchSeed};

use super::super::jump_list::JumpEntry;
use super::super::minibuf::history::{HistoryDir, HistoryStore};
use super::super::minibuf::{self, MiniBuffer, MiniBufferEvent};
use super::super::search::SearchPattern;
use super::super::{Editor, EditorState, commands, search};
use super::snapshot::PaneSnapshot;
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, Removal};

pub(in crate::editor) struct SearchLayer {
    pub(in crate::editor) minibuf: MiniBuffer,
    /// This session's pane and its pre-entry selection snapshot — restore
    /// and clear always target `snap`'s own pane, never `state.focus.id()`,
    /// which may have moved on since: a mouse click always falls through
    /// under this layer (`minibuf_input`'s `Mouse` arm), so focus (and
    /// whatever it opens next) can move to a different pane while Search
    /// stays open. See [`PaneSnapshot`]'s own doc for the capture/restore
    /// rules.
    pub(in crate::editor) snap: PaneSnapshot,
    /// Whether Extend mode was active when this session opened. Captured so
    /// live-search can extend from the pre-search anchor even though `mode`
    /// is `Search` during the live preview.
    pub(in crate::editor) extend: bool,
}

impl Layer for SearchLayer {
    fn handler(&self) -> LayerHandler {
        search_input
    }
    fn mode(&self) -> Option<EditorMode> {
        Some(EditorMode::Search)
    }
    /// Captures the snapshot here rather than at construction: `setup` runs
    /// after the outgoing layer's own `tear_down` (see the `Layer::setup`
    /// doc), so a re-entrant `/`-search (`push_mode_layer` replaces rather
    /// than no-ops on same-kind re-entry for every mode layer but `Insert`)
    /// captures the state the outgoing session's `tear_down` just restored
    /// — the true pre-search selections — instead of the mid-search preview
    /// a construction-time capture would have caught.
    fn setup(&mut self, state: &mut EditorState, view: &EngineView) {
        self.snap.capture(state, view);
    }
    fn tear_down(&mut self, state: &mut EditorState, view: &EngineView, _why: Removal) {
        if let Some(bid) = self.snap.take_restore(&mut state.panes.state, view) {
            search::ops::clear_buffer_search(&mut state.buffers, &mut state.panes.state, bid);
        }
        state.history.begin_session_all();
    }
    fn minibuf(&self) -> Option<&MiniBuffer> {
        Some(&self.minibuf)
    }
    fn minibuf_mut(&mut self) -> Option<&mut MiniBuffer> {
        Some(&mut self.minibuf)
    }
}

pub(in crate::editor) fn search_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    if let Some(event) = minibuf::minibuf_input(ed, r, ev) {
        handle_search_event(ed, r, event);
    }
}

fn handle_search_event(ed: &mut Editor, r: LayerRef, event: MiniBufferEvent) {
    match event {
        MiniBufferEvent::Cancel | MiniBufferEvent::ConfirmEmpty => {
            ed.state.truncate_layers(&ed.view, r);
        }
        MiniBufferEvent::Confirm(pattern) => {
            // Record into the correct search ring before truncating.
            let kind = ed
                .state
                .input
                .minibuf()
                .and_then(|m| HistoryStore::kind_for_prompt(&m.prompt));
            if let Some(k) = kind {
                ed.state.history.get_mut(k).push(pattern.clone());
            }
            // Persist pattern in 's' register for future n/N.
            ed.state.registers.set_search_register(pattern);
            // Record the pre-search position in the jump list before
            // discarding it, unless the match confirmed is the position
            // search started from (record_jump_if_moved). The `Confirm`
            // arm does its own accept work — taking the stash — before
            // truncating; teardown's `Search` arm restores it on every
            // *other* removal, so it's already gone here and would be a
            // no-op if left to teardown.
            let pre_sels = ed
                .state
                .input
                .at_mut::<SearchLayer>(r)
                .and_then(|s| s.snap.take_selections());
            if let Some(sels) = pre_sels {
                let bid = ed.focused_buffer_id();
                let entry = JumpEntry::new(sels, ed.doc().text(), bid);
                commands::record_jump_if_moved(&mut ed.state, &ed.view, entry);
            }
            // search_pattern stays alive on the buffer for immediate n/N
            // without recompile — the stash is already taken above, so
            // teardown's `Search` arm (gated on the same stash) won't
            // clear it.
            ed.state.truncate_layers(&ed.view, r);
        }
        MiniBufferEvent::EmptiedByBackspace => {
            // First Backspace cleared the last character — restore position but
            // stay in Search mode. A second Backspace (BackspaceOnEmpty) dismisses.
            restore_search_snapshot(ed, r);
            let Some(search) = ed.state.input.at::<SearchLayer>(r) else {
                return;
            };
            let bid = search.snap.buffer_id(&ed.view);
            search::ops::clear_buffer_search(&mut ed.state.buffers, &mut ed.state.panes.state, bid);
        }
        MiniBufferEvent::BackspaceOnEmpty => {
            // Input already empty — user pressed Backspace a second time to
            // dismiss. Teardown restores the snapshot, clears the search, and
            // begins a fresh history session — the same as this arm's own
            // body used to.
            ed.state.truncate_layers(&ed.view, r);
        }
        MiniBufferEvent::Edited => {
            if let Some(k) = ed
                .state
                .input
                .minibuf()
                .and_then(|m| HistoryStore::kind_for_prompt(&m.prompt))
            {
                ed.state.history.get_mut(k).demote_to_scratch();
            }
            update_live_search(ed, r);
        }
        MiniBufferEvent::HistoryPrev => recall_search_history(ed, r, HistoryDir::Prev),
        MiniBufferEvent::HistoryNext => recall_search_history(ed, r, HistoryDir::Next),
        MiniBufferEvent::CursorMoved
        | MiniBufferEvent::Ignored
        | MiniBufferEvent::CompleteRequested { .. } => {}
    }
}

/// Recalls the previous/next entry from whichever ring `dir` names — shared
/// by the `HistoryPrev`/`HistoryNext` arms above, which differ only in
/// `dir` — then refreshes the live preview against the recalled pattern.
fn recall_search_history(ed: &mut Editor, r: LayerRef, dir: HistoryDir) {
    let Some(prompt) = ed.state.input.minibuf().map(|m| m.prompt.clone()) else {
        return;
    };
    let Some(kind) = HistoryStore::kind_for_prompt(&prompt) else {
        return;
    };
    minibuf::recall_history(ed, kind, dir);
    update_live_search(ed, r);
}

/// Recompile the regex from the current mini-buffer input, warm the match
/// cache, and jump to the first match from the pre-search position — every
/// selection independently when the input's `m` flag is set, the primary
/// alone otherwise. Both are [`MatchScan::advance`]/[`MatchScan::advance_all`]
/// with the `AtSelection` seed; `search_jump` (`commands/search.rs`) is the
/// same scan with `PastSelection`, for `n`/`N`.
///
/// Called on every keystroke while in Search mode. Targets the *focused*
/// pane/buffer, same as before this session's stash moved onto
/// `SearchLayer` — a live preview while typing has always followed focus,
/// unlike cancel-restore/clear below, which must target the session's own
/// originating pane instead (see [`PaneSnapshot`]'s own doc).
///
/// Warms the match cache ([`search::ops::update_buffer_matches`]) before
/// scanning so every selection's hop binary-searches it instead of running
/// its own full-buffer regex scan — the per-frame highlight rebuild would
/// warm the same cache moments later anyway, so this spends that scan once
/// per keystroke instead of once per selection.
fn update_live_search(ed: &mut Editor, r: LayerRef) {
    let pattern = match ed.state.input.minibuf() {
        Some(mb) if !mb.input.is_empty() => mb.input.clone(),
        _ => return,
    };

    let Some(sp) = SearchPattern::compile(&pattern) else {
        // Invalid regex in progress — clear pattern so highlights disappear.
        let bid = ed.focused_buffer_id();
        search::ops::clear_buffer_search(&mut ed.state.buffers, &mut ed.state.panes.state, bid);
        return;
    };

    let direction = ed.state.search.direction;
    let Some(search) = ed.state.input.at::<SearchLayer>(r) else {
        return;
    };
    let mode = if search.extend {
        MotionMode::Extend
    } else {
        MotionMode::Move
    };
    let Some(sels) = search.snap.selections().cloned() else {
        return;
    };

    let bid = ed.focused_buffer_id();
    let multi = sp.multi();
    ed.state.buffers.get_mut(bid).search_pattern = Some(sp);
    search::ops::update_buffer_matches(&mut ed.state.buffers, bid);

    let buf = ed.state.buffers.get(bid);
    let cached = &buf.search_matches.matches;
    let scan = MatchScan {
        text: buf.text(),
        regex: &buf.search_pattern.as_ref().expect("just set above").regex,
        cached: (!cached.is_empty()).then_some(cached.as_slice()),
        direction,
        mode,
        seed: MatchSeed::AtSelection,
    };

    let matched = if multi {
        match scan.advance_all(sels, 1) {
            Some((new_sels, _wrapped)) => {
                ed.set_current_selections(new_sels);
                true
            }
            None => false,
        }
    } else {
        match scan.advance(sels.primary(), 1) {
            Some((new_sel, _wrapped)) => {
                ed.set_primary_selection(new_sel);
                true
            }
            None => false,
        }
    };
    if !matched {
        // No match — restore position to pre-search.
        restore_search_snapshot(ed, r);
    }
}

// ── Snapshot restore helpers ────────────────────────────────────────────────

/// Restore selections from the search-mode snapshot without consuming it —
/// always targets the session's own originating pane, not whatever's
/// currently focused (see [`PaneSnapshot`]'s own doc).
fn restore_search_snapshot(ed: &mut Editor, r: LayerRef) {
    let Some(search) = ed.state.input.at::<SearchLayer>(r) else {
        return;
    };
    search.snap.restore(&mut ed.state.panes.state, &ed.view);
}
