//! The one Insert, Paste, or dot-repeat-replay session live in the editor,
//! if any.
//!
//! [`EditorState::active_session`](crate::editor::EditorState::active_session) relies on the invariant that at most one
//! pane is ever mid-Insert, has a paste session open, or holds a dot-repeat
//! replay's own placeholder. Every session records its own owner `(pane,
//! buffer)`, so `focus::end_focus_sessions` (run by
//! every focus change and buffer switch) commits or tears it down by
//! reading that owner directly rather than current focus; ordering relative
//! to the focus write doesn't matter. Recording the owner as one value also
//! makes "is there a conflicting session elsewhere" a single field
//! comparison rather than a scan over every pane, and makes a stale session
//! left open by a caller that forgot to tear it down impossible to
//! represent silently: there is exactly one `Option` to get right, not one
//! per (pane, buffer) pair. Mirrors the pattern completion sessions already
//! use (`BufferSession` stores its `pane_id` directly on the session, not
//! keyed by a per-pane map).

use hume_editing::changeset::ChangeSet;
use hume_editing::selection::SelectionSet;
use hume_editing::state::EditState;
use hume_engine::pipeline::{BufferId, PaneId};

use crate::editor::dot_chain::DotChain;
use crate::editor::error::CommandError;
use crate::editor::replay::SourceTicket;

/// The editor's one live Insert or paste session, if any.
///
/// Fields are private: [`open_or_retarget`] is the only way to create one or
/// change its `kind`, which is what makes the `Replay` retarget rule (see
/// [`EditSessionKind::Replay`]) a compile-time guarantee rather than a
/// convention every call site has to honor on its own.
pub(in crate::editor) struct EditSession {
    /// The pane that opened this session, always the focused pane, since
    /// Insert/paste sessions only ever open there.
    pane: PaneId,
    /// The buffer this session is editing.
    buffer: BufferId,
    kind: EditSessionKind,
    group: EditGroup,
    /// Armed around an Insert-key binding dispatch or a completion-popup
    /// Enter, so a completion accept made there is recorded as its net
    /// cursor edit ([`DotCapture`]). `None` outside such a dispatch, and
    /// always on a non-`Insert` session (every armer requires an open Insert
    /// session first).
    dot_capture: Option<DotCapture>,
    /// The source this Insert session holds on the change whose code opened
    /// it, released by `EditorState::end_insert_recording`. `None` for a
    /// session opened outside a recorded change's code, and always on a
    /// non-`Insert` session.
    recording: Option<SourceTicket>,
}

impl EditSession {
    pub(in crate::editor) fn pane(&self) -> PaneId {
        self.pane
    }

    pub(in crate::editor) fn buffer(&self) -> BufferId {
        self.buffer
    }

    pub(in crate::editor) fn kind(&self) -> EditSessionKind {
        self.kind
    }

    /// `true` when committing this session now would record a revision.
    pub(in crate::editor) fn has_edits(&self) -> bool {
        self.group.edits().is_some()
    }

    /// The changeset that maps the live text back to the text the session
    /// opened on, which is the text of the history's current revision.
    /// `None` while the session has no edits.
    pub(in crate::editor) fn unrecorded_inverse(&self) -> Option<ChangeSet> {
        self.group.inverse()
    }

    pub(in crate::editor) fn group_mut(&mut self) -> &mut EditGroup {
        &mut self.group
    }

    pub(in crate::editor) fn dot_capture_mut(&mut self) -> Option<&mut DotCapture> {
        self.dot_capture.as_mut()
    }

    /// Arms `cap`. Never nests: an Insert-key binding that itself calls
    /// `completion-accept!` marks the binding's capture interactive
    /// (`EditorState::interactive_input`) instead of arming its own, so a
    /// capture is always taken back out (by `Editor::with_dot_capture`, or by
    /// `EditorState::end_insert_recording`) before the next arm.
    pub(in crate::editor) fn arm_dot_capture(&mut self, cap: DotCapture) {
        debug_assert!(
            self.dot_capture.is_none(),
            "arm_dot_capture: a capture was already armed and never resolved"
        );
        self.dot_capture = Some(cap);
    }

    /// Takes the armed capture, if any: `Editor::with_dot_capture` once its
    /// dispatch returns, and `EditorState::end_insert_recording` when the
    /// session ends first.
    pub(in crate::editor) fn take_dot_capture(&mut self) -> Option<DotCapture> {
        self.dot_capture.take()
    }

    /// `begin_insert_session`'s half of the recording-source hand-off.
    pub(in crate::editor) fn hold_recording(&mut self, ticket: SourceTicket) {
        debug_assert!(
            self.recording.is_none(),
            "hold_recording: the session already holds a recording source"
        );
        self.recording = Some(ticket);
    }

    /// `EditorState::end_insert_recording`'s half of the recording-source
    /// hand-off.
    pub(in crate::editor) fn release_recording(&mut self) -> Option<SourceTicket> {
        self.recording.take()
    }

    /// The source this session holds on the change whose code opened it.
    pub(in crate::editor) fn recording_ticket(&self) -> Option<&SourceTicket> {
        self.recording.as_ref()
    }

    /// Consumes the session, handing its [`EditGroup`] to the caller that's
    /// about to commit it (`doc_ops::commit_open_session`), the one place
    /// that needs the group by value rather than by reference.
    ///
    /// A live `dot_capture` here would be dropped along with the rest of the
    /// session. `EditorState::end_insert_recording` is what must take it out
    /// first, so reaching this with one still armed is a bug in that
    /// ordering, not a normal exit.
    pub(in crate::editor) fn into_group(self) -> EditGroup {
        debug_assert!(
            self.dot_capture.is_none(),
            "into_group: a capture was still armed; end_insert_recording should have taken it \
             first"
        );
        self.group
    }

    /// `true` if this is an Insert-kind session on `(pane, buffer)`: the
    /// check every caller composing an edit into an *Insert* session (as
    /// opposed to a Paste one, which has its own `group` but a different
    /// commit lifecycle) must make before touching it. A caller that only
    /// checked `pane`/`buffer` and skipped `kind` could silently compose an
    /// Insert-mode edit into an open Paste session's group instead of
    /// panicking on the mismatch. This is the one place that three-way
    /// check is spelled out, so it can't be forgotten at a new call site.
    pub(in crate::editor) fn is_insert_at(&self, pane: PaneId, buffer: BufferId) -> bool {
        self.owned_by(pane, buffer) && matches!(self.kind, EditSessionKind::Insert)
    }

    /// `true` if this is a Paste-kind session on `(pane, buffer)`: the
    /// `is_insert_at` counterpart for the other kind [`EditSessionKind`]
    /// can hold.
    pub(in crate::editor) fn is_paste_at(&self, pane: PaneId, buffer: BufferId) -> bool {
        self.owned_by(pane, buffer) && matches!(self.kind, EditSessionKind::Paste { .. })
    }

    /// `true` if this session belongs to `(pane, buffer)`, regardless of
    /// kind: the check [`open_or_retarget`] and every generic teardown
    /// (`focus::end_focus_sessions`'s own remaining-shape handling) needs,
    /// as opposed to `is_insert_at`'s kind-specific one.
    pub(in crate::editor) fn owned_by(&self, pane: PaneId, buffer: BufferId) -> bool {
        self.pane == pane && self.buffer == buffer
    }

    /// `true` if this is `Editor::replay_dot`'s own still-unclaimed
    /// placeholder on `(pane, buffer)`, the one kind [`open_or_retarget`]
    /// may retarget in place. See [`EditSessionKind::Replay`]'s own doc for
    /// why a real, empty `Insert`/`Paste` session must not also satisfy
    /// this.
    pub(in crate::editor) fn is_replay_at(&self, pane: PaneId, buffer: BufferId) -> bool {
        self.owned_by(pane, buffer) && matches!(self.kind, EditSessionKind::Replay)
    }

    /// `true` if this session would block a fresh [`open_or_retarget`]/
    /// [`check_can_open`] call at `(pane, buffer)`: anything except the
    /// still-unclaimed replay placeholder sitting there is a real conflict.
    fn blocks_open(&self, pane: PaneId, buffer: BufferId) -> bool {
        !self.is_replay_at(pane, buffer)
    }
}

/// `Err` if a session open right now would refuse a fresh
/// [`open_or_retarget`] call at `(pane, buffer)`: the exact rule that
/// function applies, exposed so a caller that needs to know the outcome
/// *before* mutating anything else (`commands::paste`'s
/// `do_normal_paste`/`do_smart_paste`, which must not take the live
/// selection only to discover paste would have refused) can check first.
pub(in crate::editor) fn check_can_open(
    active_session: &Option<EditSession>,
    pane: PaneId,
    buffer: BufferId,
) -> Result<(), CommandError> {
    if active_session
        .as_ref()
        .is_some_and(|s| s.blocks_open(pane, buffer))
    {
        return Err(CommandError::transient(
            "buffer has an open insert/paste session already",
        ));
    }
    Ok(())
}

/// Open a session of `kind` on `(pane, buffer)`, or take over the
/// [`EditSessionKind::Replay`] placeholder already open there.
///
/// `Ok` when: no session is open (opens fresh via `mint_group`), or the one
/// open is on this exact `(pane, buffer)` and is still the Replay
/// placeholder. Nothing has composed into it (nothing ever can, see that
/// variant's own doc), so retargeting its `kind` in place costs nothing and
/// keeps its already-correct `snapshot`/`undo_sels`, both captured
/// before *any* dispatch in the current action ran, exactly what a fresh
/// open right now would capture too, since nothing has touched the buffer
/// in between. This is what lets a replayed Steel body that pre-opens the
/// placeholder (`Editor::replay_dot`'s default) and then calls a native
/// paste command succeed instead of colliding with it: the paste takes
/// over the still-open placeholder rather than finding it already occupied.
///
/// `Err` otherwise: a real, already-open `Insert` or `Paste` session on
/// this `(pane, buffer)` (even one that's still empty) or any session on
/// a *different* one. All three are genuine conflicts that must never be
/// silently overwritten (see this module's own doc and
/// [`EditSessionKind::Replay`]'s own); see [`check_can_open`] for the exact
/// rule.
pub(in crate::editor) fn open_or_retarget(
    active_session: &mut Option<EditSession>,
    pane: PaneId,
    buffer: BufferId,
    kind: EditSessionKind,
    mint_group: impl FnOnce() -> EditGroup,
) -> Result<(), CommandError> {
    check_can_open(active_session, pane, buffer)?;
    match active_session {
        Some(existing) => existing.kind = kind,
        None => {
            *active_session = Some(EditSession {
                pane,
                buffer,
                kind,
                group: mint_group(),
                dot_capture: None,
                recording: None,
            });
        }
    }
    Ok(())
}

/// Which kind of session [`EditSession`] is: the three shapes HUME's one
/// session slot can hold.
#[derive(Clone, Copy)]
pub(in crate::editor) enum EditSessionKind {
    /// An Insert-mode session: every keystroke since entry composes into
    /// [`EditSession::group`] until Esc/Ctrl-c/a mode change commits it as
    /// one undo step.
    Insert,
    /// An open `p`/`P` + `[`/`]` ring-cycle session. `before` is the
    /// direction the session was opened with (`true` = `P`/paste-before),
    /// meaningful only while the session is open; read by `[`/`]` so
    /// cycling re-pastes in the same direction as the opening `p`/`P`.
    Paste { before: bool },
    /// `Editor::replay_dot`'s own placeholder, pre-opened before the
    /// replayed body runs, to fold a recipe replay plus the main edit into
    /// one undo revision. Holds no data of its own: nothing ever composes
    /// into it directly, and no `InsertLayer` or paste-cycle state is
    /// attached while it's this kind. The replayed body resolves it to
    /// `Insert` or `Paste` in place (`open_or_retarget`'s retarget branch)
    /// if it claims it; otherwise it stays `Replay` for
    /// `finish_replay_session` to commit directly, as a plain edit.
    ///
    /// The *only* kind [`open_or_retarget`] may retarget away from. A real,
    /// already-open `Insert` or `Paste` session, even one that's still
    /// empty, must refuse a conflicting open rather than silently hand its
    /// group to whatever's asking. This variant exists to close exactly
    /// that bug: before it, "empty" alone stood in for "is this the replay
    /// placeholder," and a real Insert session, in the one keystroke
    /// between entry and its first typed character, matched that test just
    /// as well.
    Replay,
}

/// Armed around an Insert-key binding dispatch, or a completion-popup Enter,
/// that may accept a completion: the one interactive choice with no callback
/// to hand it back to, so `.` replays its net cursor edit instead.
///
/// Armed on the `EditSession` it belongs to by `Editor::with_dot_capture`
/// for the dispatch it wraps, and taken back and finalized when that
/// dispatch returns. If the `EditSession` ends first (an accept immediately
/// followed by `exit-insert` in the same dispatch),
/// `EditorState::end_insert_recording` finalizes it.
/// `EditSession::into_group`'s `debug_assert` is the enforcement: nothing
/// may reach it with a capture still attached.
#[derive(Debug)]
pub(in crate::editor) struct DotCapture {
    /// The edits recorded while this capture was armed and the head they are
    /// measured from.
    pub(in crate::editor) chain: DotChain,
    /// Set by `EditorState::interactive_input`.
    pub(in crate::editor) interactive: bool,
    /// The entry of the Insert-key binding this capture wraps, which its
    /// net edit takes the place of; `None` for a bare Enter-key accept
    /// (`accept_completion_selection`), which has no entry of its own.
    pub(in crate::editor) slot: Option<super::replay::BindingId>,
}

/// Accumulated state for an in-progress insert or paste session.
pub(in crate::editor) struct EditGroup {
    /// The text and selections at `begin_edit_group`: the text is inverted
    /// at commit to record a single history revision, and a paste cycle
    /// re-applies from this whole state.
    pub(in crate::editor) snapshot: EditState,
    /// The selections undo restores: the ones the command that opened the
    /// group was made from. Usually `snapshot`'s own; a smart paste repeat
    /// applies to collapsed cursors but undoes to the selection it saw.
    pub(in crate::editor) undo_sels: SelectionSet,
    /// Running composition of all forward ChangeSets applied since the group
    /// opened. `None` until the first keystroke (empty session = no revision
    /// recorded on commit).
    pub(in crate::editor) cs: Option<ChangeSet>,
}

impl EditGroup {
    /// The changes applied since the group opened, `None` before the first
    /// edit or once the edits cancel out.
    pub(in crate::editor) fn edits(&self) -> Option<&ChangeSet> {
        self.cs.as_ref().filter(|cs| !cs.is_identity())
    }

    /// The changeset that maps the live text back to `snapshot`'s text.
    pub(in crate::editor) fn inverse(&self) -> Option<ChangeSet> {
        self.edits().map(|cs| cs.invert(self.snapshot.text()))
    }
}
