//! Dot capture: the net cursor edit an interactive completion makes inside
//! an Insert-key binding, recorded in place of the binding so `.` replays
//! the edit rather than the completion.

use hume_editing::changeset::ChangeSet;
use hume_editing::edit::TextChange;
use hume_editing::text::{BufferText, TextVersion};
use hume_rope::cluster::ClusterStart;
use hume_rope::offset::CharOffset;

use super::{BindingId, CursorReplacement, Input, Recording, session_feed};
use crate::editor::edit_session::InsertSession;
use crate::editor::{Editor, Severity, commands};

/// A run of edits that compose into one, plus the primary head in the text
/// before the first of them.
///
/// `origin` is a position in the first edit's old text, not in the
/// buffer's current text, which is why it is not `Tracked`: it is read
/// against the composed edit at `finish`. The chain holds only while every
/// edit starts from the text the previous one produced. `version` is that
/// text's version, and an edit starting anywhere else (one the chain never
/// saw landed in between) breaks the chain for good.
#[derive(Debug)]
pub(in crate::editor) struct DotChain {
    links: Option<Links>,
}

#[derive(Debug)]
struct Links {
    origin: ClusterStart,
    edits: Vec<ChangeSet>,
    version: TextVersion,
}

/// A chain an unrecorded edit broke: its edits no longer compose.
#[derive(Debug)]
pub(in crate::editor) struct ChainBroken;

impl DotChain {
    /// An empty chain measured from `head` in `text`.
    pub(in crate::editor) fn new(head: ClusterStart, text: &BufferText) -> Self {
        Self {
            links: Some(Links {
                origin: head,
                edits: Vec::new(),
                version: text.version(),
            }),
        }
    }

    /// Record `change`. Breaks the chain unless `change` starts from the
    /// text the chain last saw.
    pub(in crate::editor) fn push(&mut self, change: &TextChange<'_>) {
        self.links = self.links.take().and_then(|mut links| {
            (links.version == change.before().version()).then(|| {
                links.edits.push(change.changes().clone());
                links.version = change.after().version();
                links
            })
        });
    }

    /// The composed edit's replacement at `origin`, or `None` when it did
    /// not touch it.
    pub(in crate::editor) fn finish(self) -> Result<Option<CursorReplacement>, ChainBroken> {
        let links = self.links.ok_or(ChainBroken)?;
        Ok(ChangeSet::compose_all(links.edits)
            .and_then(|delta| cursor_replacement_at(&delta, links.origin.offset())))
    }
}

/// Extracts the edited region of `delta` that contains `head` (a position
/// in `delta`'s *old* document), as a cursor-relative replacement, or `None`
/// if `delta` is identity, or every edited region lands away from `head`.
/// `delta` can hold more than one region (a completion's own
/// `additionalTextEdits`, or a second cursor's own edit under a
/// multi-cursor accept), and any region other than the one at `head` is
/// simply skipped: it's document-absolute, or belongs to a different
/// cursor, the same reasoning [`CursorReplacement`]'s own doc gives for
/// excluding `additionalTextEdits` from the replacement it records.
///
/// [`ChangeSet::edited_regions`] already pairs a region's `Delete`/`Insert`
/// ops in either order, so this only has to pick the one at `head`.
fn cursor_replacement_at(delta: &ChangeSet, head: CharOffset) -> Option<CursorReplacement> {
    delta
        .edited_regions()
        .into_iter()
        .find(|r| head >= r.old.start && head <= r.old.end)
        .map(|r| CursorReplacement {
            back: head.chars_since(r.old.start),
            forward: r.old.end.chars_since(head),
            text: r.inserted.into_owned(),
        })
}

/// Armed around an Insert-key binding dispatch, or a completion-popup Enter,
/// that may accept a completion: the one interactive choice with no callback
/// to hand it back to, so `.` replays its net cursor edit instead.
///
/// Armed on the `InsertSession` it belongs to by `Editor::with_dot_capture`
/// for the dispatch it wraps, and taken back and finalized when that
/// dispatch returns. If the `InsertSession` ends first (an accept immediately
/// followed by `exit-insert` in the same dispatch),
/// `EditorState::end_insert_recording` finalizes it.
/// `InsertSession::into_group`'s `debug_assert` is the enforcement: nothing
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
    pub(in crate::editor) binding: Option<BindingId>,
}

/// Finalize a capture taken from `session` into the recording. Records
/// nothing unless a completion trigger or accept made it interactive;
/// then the entry of the binding it wrapped is replaced, so `.` does not
/// re-run a binding whose completion it cannot replay: by its net cursor
/// edit, an [`Input::Result`], when it changed text, otherwise by
/// nothing. A bare completion-popup Enter, which has no entry, appends
/// its net edit.
///
/// Takes the recording rather than the editor state, so `session` may be
/// borrowed from the input stack; a broken chain comes back as the warning
/// to report.
pub(in crate::editor) fn finalize_dot_capture(
    recording: &mut Option<Recording>,
    session: &InsertSession,
    cap: DotCapture,
) -> Result<(), String> {
    if !cap.interactive {
        return Ok(());
    }
    let (replacement, outcome) = match cap.chain.finish() {
        Ok(replacement) => (replacement, Ok(())),
        Err(ChainBroken) => (
            None,
            Err("`.` won't repeat this edit: the buffer changed while it ran".to_string()),
        ),
    };
    let Some(rec) = session_feed(recording, session) else {
        return outcome;
    };
    let Some(binding) = cap.binding else {
        if let Some(r) = replacement {
            rec.action.transcript.push(Input::Result(r));
        }
        return outcome;
    };
    if let Some(i) = rec.binding_index(binding) {
        match replacement {
            Some(r) => rec.action.transcript[i] = Input::Result(r),
            None => {
                rec.action.transcript.remove(i);
            }
        }
    }
    outcome
}

impl Editor {
    /// Runs `f` with a fresh [`DotCapture`] armed on the focused Insert
    /// session, wrapping an operation that may accept a completion: an
    /// Insert-key binding's dispatch (`binding` is its entry), or a bare
    /// completion-popup Enter (`binding` is `None`). Takes the capture back
    /// and finalizes it once `f` returns. With no Insert session there, `f`
    /// runs with nothing armed.
    pub(in crate::editor) fn with_dot_capture(
        &mut self,
        binding: Option<BindingId>,
        f: impl FnOnce(&mut Self),
    ) {
        let fp = commands::FocusedPane::current(&self.view);
        let (pane, buffer) = (fp.pid(), fp.bid(&self.view));
        let armed = self.state.input.inserting_into(pane, buffer);
        if armed {
            let head = commands::pane_view(&self.state, &self.view, fp.pane())
                .primary()
                .head();
            let chain = DotChain::new(head, self.state.buffers.get(buffer).text());
            if let Some(insert) = self.state.input.insert_session_mut() {
                insert.arm_dot_capture(DotCapture {
                    chain,
                    interactive: false,
                    binding,
                });
            }
        }
        f(self);
        if !armed {
            return;
        }
        let state = &mut self.state;
        let Some(session) = state
            .input
            .insert_session_mut()
            .filter(|s| s.owned_by(pane, buffer))
        else {
            return;
        };
        if let Some(cap) = session.take_dot_capture()
            && let Err(warning) = finalize_dot_capture(&mut state.recording, session, cap)
        {
            state.report(Severity::Warning, warning);
        }
    }
}

#[cfg(test)]
mod tests;
