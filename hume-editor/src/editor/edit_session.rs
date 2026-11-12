//! The one Insert or Paste session live in the editor, if any: an
//! [`InsertSession`], or a [`PasteSession`] in
//! [`EditorState::paste_session`](crate::editor::EditorState::paste_session).
//! At most one is open at a time ([`open_session`]), which every open checks
//! ([`check_can_open`]).
//!
//! Every session records its own owner `(pane, buffer)`, so
//! `focus::end_focus_sessions` (run by every focus change and buffer switch)
//! commits or tears it down by reading that owner directly rather than
//! current focus; ordering relative to the focus write doesn't matter.
//! Recording the owner as one value also makes "is there a conflicting
//! session elsewhere" a single field comparison rather than a scan over
//! every pane.

use hume_core::{BufferId, PaneHandle, PaneId};
use hume_editing::changeset::{Assoc, ChangeSet};
use hume_editing::edit::TextChange;
use hume_editing::selection::SelectionSet;
use hume_editing::state::EditState;
use hume_editing::tracked::Tracked;
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::editor::buffer::store::BufferStore;
use crate::editor::error::CommandError;
use crate::editor::pane_state::PaneEntries;
use crate::editor::replay::DotCapture;
use crate::editor::replay::{AnswerPlace, SourceTicket};

/// An open `p`/`P` + `[`/`]` ring-cycle session. Fields are private:
/// [`open_paste`] is the only way to create one.
pub(in crate::editor) struct PasteSession {
    /// The pane that opened this session, always the focused pane, since
    /// paste sessions only ever open there.
    pane: PaneId,
    /// The buffer this session is editing.
    buffer: BufferId,
    /// The direction the session was opened with (`true` = `P`/paste-before),
    /// read by `[`/`]` so cycling re-pastes in the same direction as the
    /// opening `p`/`P`.
    before: bool,
    /// Where the opening paste recorded what it pasted for `.`, which `[`/`]`
    /// replace with the entry they cycle to; `None` when no change recorded
    /// it.
    answer: Option<AnswerPlace>,
    group: EditGroup,
}

impl PasteSession {
    pub(in crate::editor) fn pane(&self) -> PaneId {
        self.pane
    }

    pub(in crate::editor) fn buffer(&self) -> BufferId {
        self.buffer
    }

    pub(in crate::editor) fn before(&self) -> bool {
        self.before
    }

    pub(in crate::editor) fn answer(&self) -> Option<&AnswerPlace> {
        self.answer.as_ref()
    }

    pub(in crate::editor) fn group(&self) -> &EditGroup {
        &self.group
    }

    pub(in crate::editor) fn group_mut(&mut self) -> &mut EditGroup {
        &mut self.group
    }

    /// Consumes the session, handing its [`EditGroup`] to the commit
    /// (`commit_paste_group`).
    pub(in crate::editor) fn into_group(self) -> EditGroup {
        self.group
    }

    /// `true` if this session belongs to `(pane, buffer)`.
    pub(in crate::editor) fn owned_by(&self, pane: PaneId, buffer: BufferId) -> bool {
        self.pane == pane && self.buffer == buffer
    }
}

/// The one session open in the editor, of either kind. At most one is open
/// at a time.
#[derive(Clone, Copy)]
pub(in crate::editor) enum OpenSession<'a> {
    Insert(&'a InsertSession),
    Paste(&'a PasteSession),
}

impl<'a> OpenSession<'a> {
    pub(in crate::editor) fn pane(self) -> PaneId {
        match self {
            OpenSession::Insert(s) => s.pane,
            OpenSession::Paste(s) => s.pane,
        }
    }

    pub(in crate::editor) fn buffer(self) -> BufferId {
        match self {
            OpenSession::Insert(s) => s.buffer,
            OpenSession::Paste(s) => s.buffer,
        }
    }

    fn group(self) -> &'a EditGroup {
        match self {
            OpenSession::Insert(s) => &s.group,
            OpenSession::Paste(s) => &s.group,
        }
    }

    /// `true` when committing this session now would record a revision.
    pub(in crate::editor) fn has_edits(self) -> bool {
        self.group().edits().is_some()
    }

    /// The changeset that maps the live text back to the text the session
    /// opened on, which is the text of the history's current revision.
    /// `None` while the session has no edits.
    pub(in crate::editor) fn unrecorded_inverse(self) -> Option<ChangeSet> {
        self.group().inverse()
    }
}

/// The open session, from the Insert session and the Paste slot that hold
/// them.
pub(in crate::editor) fn open_session<'a>(
    insert: Option<&'a InsertSession>,
    paste: &'a Option<PasteSession>,
) -> Option<OpenSession<'a>> {
    debug_assert!(
        insert.is_none() || paste.is_none(),
        "an Insert session and a Paste session are open at once"
    );
    insert
        .map(OpenSession::Insert)
        .or(paste.as_ref().map(OpenSession::Paste))
}

/// `Err` if `open` would refuse a fresh [`open_insert`]/[`open_paste`] call:
/// any open session does, even one that is still empty. Exposed so a caller
/// that needs to know the outcome *before* mutating anything else
/// (`commands::paste`'s `do_normal_paste`/`do_smart_paste`, which must not
/// take the live selection only to discover paste would have refused) can
/// check first.
pub(in crate::editor) fn check_can_open(open: Option<OpenSession<'_>>) -> Result<(), CommandError> {
    if open.is_some() {
        return Err(CommandError::transient(
            "buffer has an open insert/paste session already",
        ));
    }
    Ok(())
}

/// Open an Insert session on `(pane, buffer)`, always the focused pane,
/// since Insert only ever opens there, for the `InsertLayer` that will own
/// it. `insert` is the Insert session already open, if any. The session
/// holds `recording`, its source on the change whose code opens it, if any,
/// until it ends.
///
/// The session snapshots `entered_with`, the selections Insert mode was
/// entered with before they were placed where typing starts, as its
/// `undo_sels`: what undo restores, and the revision's pre-edit selections.
/// `Err` when a session is open ([`check_can_open`]). The only way to create
/// an [`InsertSession`].
pub(in crate::editor) fn open_insert(
    buffers: &BufferStore,
    entered_with: SelectionSet,
    insert: Option<&InsertSession>,
    paste: &Option<PasteSession>,
    pane: PaneId,
    buffer: BufferId,
    recording: Option<SourceTicket>,
) -> Result<InsertSession, CommandError> {
    check_can_open(open_session(insert, paste))?;
    let group = buffers
        .get(buffer)
        .begin_edit_group(entered_with.clone(), entered_with);
    Ok(InsertSession {
        pane,
        buffer,
        group,
        dot_capture: None,
        recording,
        typed_run: None,
        autoindent: None,
        step_back_on_exit: false,
    })
}

/// Open a Paste session on `(pane, buffer)` over the group `mint_group`
/// builds; `before` and `answer` are its [`PasteSession`] fields. `Err` when
/// a session is open ([`check_can_open`]), and then nothing is minted.
pub(in crate::editor) fn open_paste(
    insert: Option<&InsertSession>,
    paste: &mut Option<PasteSession>,
    pane: PaneId,
    buffer: BufferId,
    before: bool,
    answer: Option<AnswerPlace>,
    mint_group: impl FnOnce() -> EditGroup,
) -> Result<(), CommandError> {
    check_can_open(open_session(insert, paste))?;
    *paste = Some(PasteSession {
        pane,
        buffer,
        before,
        answer,
        group: mint_group(),
    });
    Ok(())
}

/// `Err` when `open`, the open session, is on `target`'s buffer and was
/// opened by some pane other than `target`'s (any pane, for a handle with
/// none): the exclusivity check every buffer-mutating chokepoint in
/// `doc_ops` runs before touching that buffer's text.
///
/// Remote dispatch (`(call! "cmd" pane)` targeting a pane other than the
/// focused one) means a *different* pane's own in-progress insert/paste
/// session can otherwise be mutated out from under it: its eventual commit
/// inverts the composed `ChangeSet` against a `snapshot` a concurrent
/// edit from another pane would make stale, and its next grouped edit panics
/// in `ChangeSet::compose`'s length assert. Scoped to "some *other* pane"
/// (not any open session at all) because the focused/target pane's own open
/// session is what routes an edit into
/// [`doc_ops::apply_doc_edit_grouped`](super::doc_ops::apply_doc_edit_grouped),
/// a distinct, unrelated case this check must not shadow.
///
/// `doc_ops`'s `admit` runs this right before an edit commits;
/// [`super::lsp::edits::apply_workspace_edit`] runs it for every file in its
/// plan, before any file's commit, so a conflict on file *k* of a multi-file
/// edit is caught before files `0..k` are touched rather than partway
/// through the commit loop.
pub(in crate::editor) fn check_no_conflicting_session(
    open: Option<OpenSession<'_>>,
    target: PaneHandle,
) -> Result<(), CommandError> {
    if open.is_some_and(|s| Some(s.pane()) != target.pane() && s.buffer() == target.buffer()) {
        return Err(CommandError::new(
            "buffer has an open insert/paste session on another pane",
        ));
    }
    Ok(())
}

/// Commit the open Paste session as one undo step, read from the session's
/// own `(pane, buffer)`, never from focus, so the commit lands correctly
/// however focus moved in the meantime. No-op if none is open.
pub(in crate::editor) fn commit_paste_group(
    buffers: &mut BufferStore,
    pane_state: &PaneEntries,
    paste: &mut Option<PasteSession>,
) {
    let Some(session) = paste.take() else {
        return;
    };
    let (pane, buffer) = (session.pane(), session.buffer());
    commit_group(buffers, pane_state, pane, buffer, session.into_group());
}

/// Record the Insert session `session`, which the leaving `InsertLayer`'s
/// teardown has closed, as one undo step.
pub(in crate::editor) fn commit_insert(
    buffers: &mut BufferStore,
    pane_state: &PaneEntries,
    session: InsertSession,
) {
    let (pane, buffer) = (session.pane(), session.buffer());
    commit_group(buffers, pane_state, pane, buffer, session.into_group());
}

/// Record `group`, the edits a session on `(pane, buffer)` made, as one undo
/// step, with the pane's selections now as the step's `post_sels`: cloned,
/// not taken, since the pane keeps them.
fn commit_group(
    buffers: &mut BufferStore,
    pane_state: &PaneEntries,
    pane: PaneId,
    buffer: BufferId,
    group: EditGroup,
) {
    let post_sels = pane_state.seeded(pane, buffer).selections().clone();
    buffers
        .get_mut(buffer)
        .commit_edit_group(group, post_sels, pane);
}

/// The whitespace an insert session auto-inserted for each selection
/// (`o`/`O`/Enter's auto-indent): the range it occupies, carried through
/// every text change, and the text that was inserted there. Index-paired
/// with the selections it was armed for; [`AutoIndent::new`] keeps the two
/// `Vec`s the same length.
pub(crate) struct AutoIndent {
    ranges: Vec<ExclusiveRange<CharOffset>>,
    armed: Vec<String>,
}

impl AutoIndent {
    pub(in crate::editor) fn new(
        entries: impl IntoIterator<Item = (ExclusiveRange<CharOffset>, String)>,
    ) -> Self {
        let (ranges, armed) = entries.into_iter().unzip();
        Self { ranges, armed }
    }

    pub(in crate::editor) fn ranges(&self) -> &[ExclusiveRange<CharOffset>] {
        &self.ranges
    }

    pub(in crate::editor) fn armed(&self) -> &[String] {
        &self.armed
    }
}

/// The span typed since an open insert session's entry command positioned the
/// cursor: one (anchor, end) pair per selection, index-paired and always the
/// same length (both `Vec`s are seeded together by `begin_typed_run` and
/// carried together by [`InsertSession::carry_marks`], the only writer after
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

/// The open Insert session: every keystroke since entry composes into its
/// `group` until Esc/Ctrl-c/a mode change commits it as one undo step.
/// Built only by [`open_insert`], so a fresh session starts with none of a
/// previous one's records.
pub(in crate::editor) struct InsertSession {
    /// The pane that opened this session, always the focused pane.
    pane: PaneId,
    /// The buffer this session is editing.
    buffer: BufferId,
    group: EditGroup,
    /// Armed around an Insert-key binding dispatch or a completion-popup
    /// Enter, so a completion accept made there is recorded as its net
    /// cursor edit ([`DotCapture`]). `None` outside such a dispatch.
    dot_capture: Option<DotCapture>,
    /// The source this session holds on the change whose code opened it,
    /// released by `EditorState::end_insert_recording`. `None` for a session
    /// opened outside a recorded change's code.
    recording: Option<SourceTicket>,
    /// The session's typed span, carried through every text change by
    /// [`InsertSession::carry_marks`]. `Some` from the moment the session's
    /// entry command positions the cursor (`begin_typed_run`) until
    /// `tear_down_insert` consumes it on exit, for every insert entry
    /// (`i`/`a`/`o`/`O`/`A`/`I`/`c`/…).
    pub(in crate::editor) typed_run: Option<Tracked<TypedRun>>,
    /// Per-selection leading whitespace *this* session auto-inserted
    /// (`o`/`O`/Enter's auto-indent), its ranges carried the same way
    /// `typed_run` is. `None` when nothing has armed a record yet (`i`/`I`/`c`
    /// entry, or before the first Enter/`o`/`O`).
    ///
    /// The armed text is what electric dedent compares a line's whitespace
    /// with: a line still holding it is one the user has not re-indented.
    ///
    /// The ranges are read on exit (`hume_ops::edit::owned_blank_indent`)
    /// to decide whether the cursor's current blank line is whitespace *this
    /// session itself* inserted, rather than pre-existing or hand-typed
    /// whitespace. The vacate-on-exit trim must never touch the latter. A
    /// positional record rather than a bool ("is some trim pending") so
    /// ownership is re-derived from the buffer at exit instead of relying on
    /// every cursor-motion key handler remembering to invalidate a flag: a
    /// motion off this line leaves the record pointing at a line the cursor
    /// has left, which the containment check rejects on its own.
    pub(in crate::editor) autoindent: Option<Tracked<AutoIndent>>,
    /// Set by `begin_typed_run` from its `ExitCursor` parameter for `a`/`A`/
    /// `o`/`O` entry (never for `i`/`I`/`c`). Decides where an *empty* typed
    /// run's cursor lands on exit: step one grapheme back (so `a<Esc>` is a
    /// round trip) rather than staying put.
    pub(in crate::editor) step_back_on_exit: bool,
}

impl InsertSession {
    pub(in crate::editor) fn pane(&self) -> PaneId {
        self.pane
    }

    pub(in crate::editor) fn buffer(&self) -> BufferId {
        self.buffer
    }

    /// `true` if this session belongs to `(pane, buffer)`.
    pub(in crate::editor) fn owned_by(&self, pane: PaneId, buffer: BufferId) -> bool {
        self.pane == pane && self.buffer == buffer
    }

    pub(in crate::editor) fn group_mut(&mut self) -> &mut EditGroup {
        &mut self.group
    }

    /// Consumes the session, handing its [`EditGroup`] to its commit
    /// (`commit_insert`).
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
        if let Some(record) = self.autoindent.as_mut() {
            record.translate(change, |record| {
                change.changes().map_ranges(&mut record.ranges)
            });
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
