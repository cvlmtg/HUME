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

use hume_editing::changeset::{Assoc, ChangeSet};
use hume_editing::edit::TextChange;
use hume_editing::selection::SelectionSet;
use hume_editing::state::EditState;
use hume_editing::tracked::Tracked;
use hume_engine::pipeline::{BufferId, PaneId};
use hume_rope::offset::{CharOffset, ExclusiveRange};

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
}

impl EditSession {
    pub(in crate::editor) fn pane(&self) -> PaneId {
        self.pane
    }

    pub(in crate::editor) fn buffer(&self) -> BufferId {
        self.buffer
    }

    pub(in crate::editor) fn kind(&self) -> &EditSessionKind {
        &self.kind
    }

    /// The Insert-only state, `None` on a Paste or Replay session.
    pub(in crate::editor) fn insert(&self) -> Option<&InsertState> {
        match &self.kind {
            EditSessionKind::Insert(insert) => Some(insert),
            EditSessionKind::Paste { .. } | EditSessionKind::Replay => None,
        }
    }

    /// [`Self::insert`], mutably.
    pub(in crate::editor) fn insert_mut(&mut self) -> Option<&mut InsertState> {
        match &mut self.kind {
            EditSessionKind::Insert(insert) => Some(insert),
            EditSessionKind::Paste { .. } | EditSessionKind::Replay => None,
        }
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

    pub(in crate::editor) fn group(&self) -> &EditGroup {
        &self.group
    }

    pub(in crate::editor) fn group_mut(&mut self) -> &mut EditGroup {
        &mut self.group
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
            self.insert()
                .is_none_or(|insert| insert.dot_capture.is_none()),
            "into_group: a capture was still armed; end_insert_recording should have taken it \
             first"
        );
        self.group
    }

    /// `true` if this is an Insert-kind session on `(pane, buffer)`: the
    /// check a caller composing an edit into an Insert session makes, since
    /// a Paste session also has a `group` but commits differently.
    pub(in crate::editor) fn is_insert_at(&self, pane: PaneId, buffer: BufferId) -> bool {
        self.insert_at(pane, buffer).is_some()
    }

    /// The Insert session's own state, when this is an Insert-kind session
    /// on `(pane, buffer)`.
    pub(in crate::editor) fn insert_at(
        &self,
        pane: PaneId,
        buffer: BufferId,
    ) -> Option<&InsertState> {
        if self.owned_by(pane, buffer) {
            self.insert()
        } else {
            None
        }
    }

    /// [`Self::insert_at`], mutably.
    pub(in crate::editor) fn insert_at_mut(
        &mut self,
        pane: PaneId,
        buffer: BufferId,
    ) -> Option<&mut InsertState> {
        if self.owned_by(pane, buffer) {
            self.insert_mut()
        } else {
            None
        }
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

/// Proof that an Insert session was opened, from which an `InsertLayer` is
/// built. Minted only by [`open_insert`], so an Insert layer exists only over
/// an open Insert session, and committing one takes the proof its layer
/// holds.
#[must_use]
pub(in crate::editor) struct InsertOpened(());

/// Open an Insert session on `(pane, buffer)` holding `recording`, its
/// source on the change whose code opens it, or retarget the
/// [`EditSessionKind::Replay`] placeholder open there to one. The only way to
/// create an Insert-kind session: [`InsertState`] has no other constructor.
/// See [`open_or_retarget`] for when it refuses.
pub(in crate::editor) fn open_insert(
    active_session: &mut Option<EditSession>,
    pane: PaneId,
    buffer: BufferId,
    recording: Option<SourceTicket>,
    mint_group: impl FnOnce() -> EditGroup,
) -> Result<InsertOpened, CommandError> {
    open_or_retarget(
        active_session,
        pane,
        buffer,
        EditSessionKind::Insert(InsertState::new(recording)),
        mint_group,
    )?;
    Ok(InsertOpened(()))
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
            });
        }
    }
    Ok(())
}

/// Which kind of session [`EditSession`] is: the three shapes HUME's one
/// session slot can hold.
pub(in crate::editor) enum EditSessionKind {
    /// An Insert-mode session: every keystroke since entry composes into
    /// [`EditSession::group`] until Esc/Ctrl-c/a mode change commits it as
    /// one undo step.
    Insert(InsertState),
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

/// The span typed since an open insert session's entry command positioned the
/// cursor: one (anchor, end) pair per selection, index-paired and always the
/// same length (both `Vec`s are seeded together by `begin_typed_run` and
/// carried together by [`InsertState::carry_marks`], the only writer after
/// seeding).
pub(crate) struct TypedRun {
    /// Start of each selection's typed span, kept in post-edit coordinates
    /// with `Assoc::Before`: a keystroke exactly at the anchor is typed
    /// content, so the anchor must stay left of it.
    pub anchors: Vec<CharOffset>,
    /// Exclusive end of each typed span, kept with `Assoc::After` (opposite
    /// of `anchors`) so it tracks what was written rather than where the
    /// cursor happens to sit: a real keystroke at the run's end pushes it
    /// forward, an auto-paired closer pushes it past both inserted chars, and
    /// a skip-close (which edits nothing) leaves it where it was.
    pub ends: Vec<CharOffset>,
}

/// The state only an Insert session has. Built only by [`open_insert`], so
/// a fresh session starts with none of a previous one's records.
pub(in crate::editor) struct InsertState {
    /// Armed around an Insert-key binding dispatch or a completion-popup
    /// Enter, so a completion accept made there is recorded as its net
    /// cursor edit ([`DotCapture`]). `None` outside such a dispatch.
    dot_capture: Option<DotCapture>,
    /// The source this session holds on the change whose code opened it,
    /// released by `EditorState::end_insert_recording`. `None` for a session
    /// opened outside a recorded change's code.
    recording: Option<SourceTicket>,
    /// The session's typed span, carried through every text change by
    /// [`InsertState::carry_marks`]. `Some` from the moment the session's
    /// entry command positions the cursor (`begin_typed_run`) until
    /// `tear_down_insert` consumes it on exit, for every insert entry
    /// (`i`/`a`/`o`/`O`/`A`/`I`/`c`/…).
    pub(in crate::editor) typed_run: Option<Tracked<TypedRun>>,
    /// Per-selection range of leading whitespace *this* session
    /// auto-inserted (`o`/`O`/Enter's auto-indent), carried the same way
    /// `typed_run` is. `None` when nothing has armed a record yet (`i`/`I`/`c`
    /// entry, or before the first Enter/`o`/`O`).
    ///
    /// Read on exit (`hume_ops::edit::owned_blank_indent`) to decide whether
    /// the cursor's current blank line is whitespace *this session itself*
    /// inserted, rather than pre-existing or hand-typed whitespace. The
    /// vacate-on-exit trim must never touch the latter. A positional record
    /// rather than a bool ("is some trim pending") so ownership is re-derived
    /// from the buffer at exit instead of relying on every cursor-motion key
    /// handler remembering to invalidate a flag: a motion off this line
    /// leaves the record pointing at a line the cursor no longer occupies,
    /// which the containment check rejects on its own.
    pub(in crate::editor) autoindent: Option<Tracked<Vec<ExclusiveRange<CharOffset>>>>,
    /// Set by `begin_typed_run` from its `ExitCursor` parameter for `a`/`A`/
    /// `o`/`O` entry (never for `i`/`I`/`c`). Decides where an *empty* typed
    /// run's cursor lands on exit: step one grapheme back (so `a<Esc>` is a
    /// round trip) rather than staying put.
    pub(in crate::editor) step_back_on_exit: bool,
    /// Whether the session was entered via a ring-capturing kill (bare or
    /// `"k`-prefixed `c`; an explicit-register change writes no stamp and
    /// must not set this). Set only by `cmd_change`. Read by
    /// `tear_down_insert`: every keystroke typed during the session bumps
    /// `BufferStore::edit_seq`, so the `PasteStamp` `cmd_change` wrote
    /// (pointing at the just-replaced text) goes stale by the time the
    /// session closes, and refreshing its `seq` there is what keeps
    /// `c <text> <Esc> p` reading the kill ring instead of the clipboard.
    pub(in crate::editor) kill_opened_session: bool,
}

impl InsertState {
    /// A session holding `recording` on the change whose code opens it.
    fn new(recording: Option<SourceTicket>) -> Self {
        Self {
            dot_capture: None,
            recording,
            typed_run: None,
            autoindent: None,
            step_back_on_exit: false,
            kill_opened_session: false,
        }
    }

    /// Carry the session's typed-run and autoindent records through `change`.
    pub(in crate::editor) fn carry_marks(&mut self, change: &TextChange<'_>) {
        // `ChangeSet::map_ranges` maps (start, end) pairs directly, but with
        // `Assoc::After` on starts and `Assoc::Before` on ends: it shrinks a
        // range around inserted text. A typed run needs the opposite: it must
        // grow to include what was just typed, so anchors and ends are mapped
        // separately with `Assoc` reversed from what `map_ranges` would use.
        if let Some(run) = self.typed_run.as_mut() {
            run.translate(change, |run| {
                change
                    .changes()
                    .map_positions(&mut run.anchors, Assoc::Before);
                change.changes().map_positions(&mut run.ends, Assoc::After);
            });
        }
        // Shrinks each record around any edit landing at its
        // start/end; see `map_ranges`' own doc. That is what makes ownership
        // self-revoking: text typed past a record's end, or a line split
        // before its start, falls outside the mapped range without any key
        // handler needing to clear it.
        if let Some(ranges) = self.autoindent.as_mut() {
            ranges.translate(change, |ranges| change.changes().map_ranges(ranges));
        }
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

    /// The source this session holds on the change whose code opened it.
    pub(in crate::editor) fn recording(&self) -> Option<&SourceTicket> {
        self.recording.as_ref()
    }

    /// Closes the session's source on its change:
    /// `EditorState::end_insert_recording`, as the session ends.
    pub(in crate::editor) fn release_recording(&mut self) -> Option<SourceTicket> {
        self.recording.take()
    }
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
    pub(in crate::editor) binding: Option<super::replay::BindingId>,
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

    /// Fold `cs`, an edit applied since, into the group. An identity `cs`
    /// leaves it untouched: a group whose every edit was identity commits
    /// nothing.
    pub(in crate::editor) fn compose(&mut self, cs: &ChangeSet) {
        if cs.is_identity() {
            return;
        }
        self.cs = Some(match self.cs.take() {
            None => cs.clone(),
            Some(acc) => acc.compose(cs.clone()),
        });
    }

    /// The changeset that maps the live text back to `snapshot`'s text.
    pub(in crate::editor) fn inverse(&self) -> Option<ChangeSet> {
        self.edits().map(|cs| cs.invert(self.snapshot.text()))
    }
}
