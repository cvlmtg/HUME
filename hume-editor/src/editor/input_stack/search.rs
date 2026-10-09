//! The `Search` layer: the `/`/`?`-prompt minibuffer mode.

use hume_editing::selection::SelectionSet;
use hume_editing::state::EditState;
use hume_engine::pipeline::{BufferId, EngineView, PaneId};
use hume_engine::types::EditorMode;
use hume_ops::MotionMode;
use hume_ops::search::{MatchScan, MatchSeed, SearchDirection};

use super::super::jump_list::JumpEntry;
use super::super::minibuf::history::{HistoryDir, HistoryKind, HistoryStore};
use super::super::minibuf::{self, MiniBuffer, MiniBufferEvent};
use super::super::search::SearchPattern;
use super::super::{Editor, EditorState, commands, search};
use super::snapshot::PaneSnapshot;
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, Removal};

pub(in crate::editor) struct SearchLayer {
    pub(in crate::editor) minibuf: MiniBuffer,
    /// This session's pane and its pre-entry selection snapshot. Restore
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
    /// (the true pre-search selections) instead of the mid-search preview
    /// a construction-time capture would have caught.
    fn setup(&mut self, state: &mut EditorState, view: &EngineView) {
        self.snap.capture(state, view);
    }
    fn tear_down(&mut self, state: &mut EditorState, _view: &EngineView, _why: Removal) {
        if let Some(bid) = self
            .snap
            .take_restore(&mut state.panes.state, &state.buffers)
        {
            search::ops::clear_buffer_search(&mut state.buffers, &mut state.panes.state, bid);
        }
        state.history.begin_session_all();
    }
    fn snapshot_mut(&mut self) -> Option<&mut PaneSnapshot> {
        Some(&mut self.snap)
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
            // The `Confirm` arm does its own accept work (taking the stash)
            // before truncating; teardown's `Search` arm restores it on
            // every *other* removal, so it's already gone here and would be
            // a no-op if left to teardown.
            let pre = ed.state.input.at_mut::<SearchLayer>(r).and_then(|s| {
                let (bid, sels) = s.snap.take_selections()?;
                Some(SearchOrigin {
                    pane: s.snap.pane(),
                    bid,
                    sels,
                })
            });
            let direction = ed.state.search.direction;
            commit_search(&mut ed.state, &ed.view, pattern, direction, pre);
            // search_pattern stays alive on the buffer for immediate n/N
            // without recompile. The stash is already taken above, so
            // teardown's `Search` arm (gated on the same stash) won't
            // clear it.
            ed.state.truncate_layers(&ed.view, r);
        }
        MiniBufferEvent::EmptiedByBackspace => {
            // First Backspace cleared the last character: restore position but
            // stay in Search mode. A second Backspace (BackspaceOnEmpty) dismisses.
            restore_search_snapshot(ed, r);
            let Some(search) = ed.state.input.at::<SearchLayer>(r) else {
                return;
            };
            let Some((bid, _)) = search.snap.selections() else {
                return;
            };
            search::ops::clear_buffer_search(&mut ed.state.buffers, &mut ed.state.panes.state, bid);
        }
        MiniBufferEvent::BackspaceOnEmpty => {
            // Input already empty: user pressed Backspace a second time to
            // dismiss. Teardown restores the snapshot, clears the search, and
            // begins a fresh history session.
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

/// Recalls the previous/next entry from whichever ring `dir` names (shared
/// by the `HistoryPrev`/`HistoryNext` arms above, which differ only in
/// `dir`), then refreshes the live preview against the recalled pattern.
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

/// Recompile the regex from the current mini-buffer input and preview it
/// from the pre-search position ([`preview_search`]). Called on every
/// keystroke while in Search mode. Targets the session's own pane and the
/// buffer its snapshot was read from, the only text the snapshot's
/// selections belong to, as cancel-restore/clear below do (see
/// [`PaneSnapshot`]'s own doc).
fn update_live_search(ed: &mut Editor, r: LayerRef) {
    let pattern = match ed.state.input.minibuf() {
        Some(mb) if !mb.input.is_empty() => mb.input.clone(),
        _ => return,
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
    let Some(origin) = search.snap.selections().map(|(bid, sels)| SearchOrigin {
        pane: search.snap.pane(),
        bid,
        sels: sels.clone(),
    }) else {
        return;
    };

    let bid = origin.bid;
    match preview_search(&mut ed.state, origin, direction, mode, &pattern) {
        Ok(true) => {}
        // No match: restore position to pre-search.
        Ok(false) => restore_search_snapshot(ed, r),
        // Invalid regex in progress: clear pattern so highlights disappear.
        Err(InvalidPattern) => {
            search::ops::clear_buffer_search(&mut ed.state.buffers, &mut ed.state.panes.state, bid);
        }
    }
}

/// `raw` doesn't compile to a search regex.
pub(in crate::editor) struct InvalidPattern;

/// Where a search starts: pane `pane`'s selections `sels` on buffer `bid`.
pub(in crate::editor) struct SearchOrigin {
    pub(in crate::editor) pane: PaneId,
    pub(in crate::editor) bid: BufferId,
    pub(in crate::editor) sels: SelectionSet,
}

/// Make `raw` the search pattern of `origin`'s buffer and select its first
/// match from `origin`'s selections: every selection independently when the
/// input's `m` flag is set, the primary alone otherwise. Both are
/// [`MatchScan::advance`]/[`MatchScan::advance_all`] with the `AtSelection`
/// seed; `search_jump` (`commands/search.rs`) is the same scan with
/// `PastSelection`, for `n`/`N`.
///
/// `Ok(false)` when nothing matched: the pattern is stored but the
/// selections are left as they were. `Err` touches nothing.
///
/// Warms the match cache ([`search::ops::update_buffer_matches`]) before
/// scanning so every selection's hop binary-searches it instead of running
/// its own full-buffer regex scan. The per-frame highlight rebuild would
/// warm the same cache moments later anyway, so this spends that scan once
/// instead of once per selection.
pub(in crate::editor) fn preview_search(
    state: &mut EditorState,
    origin: SearchOrigin,
    direction: SearchDirection,
    mode: MotionMode,
    raw: &str,
) -> Result<bool, InvalidPattern> {
    let SearchOrigin { pane, bid, sels } = origin;
    let sp = SearchPattern::compile(raw).ok_or(InvalidPattern)?;
    let multi = sp.multi();
    state.buffers.get_mut(bid).search_pattern = Some(sp);
    search::ops::update_buffer_matches(&mut state.buffers, bid);

    let buf = state.buffers.get(bid);
    let cached = buf.search_matches.matches(buf.text());
    let scan = MatchScan {
        text: buf.text(),
        regex: &buf.search_pattern.as_ref().expect("just set above").regex,
        cached: (!cached.is_empty()).then_some(cached),
        direction,
        mode,
        seed: MatchSeed::AtSelection,
    };

    let current = EditState::bind(buf.text(), sels);
    let moved = if multi {
        scan.advance_all(current, 1)
            .map(|(new_state, _wrapped)| new_state)
    } else {
        scan.advance(current.view().primary(), 1)
            .map(|(new_sel, _wrapped)| current.replace_primary(new_sel))
    };
    let Some(moved) = moved else {
        return Ok(false);
    };
    let text = moved.text().clone();
    state.panes.state[pane][bid].set_selections(moved.into_selections(), &text);
    Ok(true)
}

/// Accept `raw` as the confirmed search: record it in `direction`'s history
/// ring and the `s` register (for `n`/`N`), and record `pre`, the pane's
/// selections before the search, in its jump list unless the search left
/// them where they were (`record_if_moved`).
pub(in crate::editor) fn commit_search(
    state: &mut EditorState,
    view: &EngineView,
    raw: String,
    direction: SearchDirection,
    pre: Option<SearchOrigin>,
) {
    let kind = match direction {
        SearchDirection::Forward => HistoryKind::SearchForward,
        SearchDirection::Backward => HistoryKind::SearchBackward,
    };
    state.history.get_mut(kind).push(raw.clone());
    state.registers.set_search_register(raw);
    if let Some(SearchOrigin { pane, bid, sels }) = pre
        && let Some(t) = commands::CommandPane::existing(view, pane)
    {
        let entry = JumpEntry::new(sels, state.buffers.get(bid).text(), bid);
        crate::editor::jump_list::record_if_moved(state, view, t, entry);
    }
}

// ── Snapshot restore helpers ────────────────────────────────────────────────

/// Restore selections from the search-mode snapshot without consuming it,
/// always targets the session's own originating pane, not whatever's
/// currently focused (see [`PaneSnapshot`]'s own doc).
fn restore_search_snapshot(ed: &mut Editor, r: LayerRef) {
    let Some(search) = ed.state.input.at::<SearchLayer>(r) else {
        return;
    };
    search
        .snap
        .restore(&mut ed.state.panes.state, &ed.state.buffers);
}
