//! A paste whose text the `on-paste` handlers may rewrite before it lands.
//!
//! Every paste is planned where it is asked for (`p`/`P` and `[`/`]`, a
//! terminal paste, `paste-text!`) and handed back as a [`PlannedPaste`] the
//! code that asked for it must finish. The handlers run in Scheme, which a
//! native command cannot reach: outside a script [`Editor::finish_paste`]
//! finishes it, and inside one the call parks it on
//! `EditorState::paste_awaiting_handlers` for the `%finish-paste!`
//! (`bootstrap.scm`) after it. Both land it through
//! [`EditorState::apply_paste_plan`], on the pane, buffer and selections it
//! was planned for and only while that pane is open and that text is
//! unchanged, wherever focus or the cursor moved meanwhile. A `[`/`]` also
//! needs the paste session it cycles, which a plugin loading meanwhile can
//! end; the cycle is then dropped. A script cannot paste while a paste
//! waits ([`EditorState::refuse_paste_while_one_waits`]).

use std::rc::Rc;

use hume_core::{BufferId, PaneHandle, PaneId};
use hume_editing::selection::{EditView, SelectionSet};
use hume_editing::text::{BufferText, TextVersion};
use hume_engine::pipeline::EngineView;
use hume_ops::edit::{insert_placement, paste_placement, replaced_texts};
use hume_ops::register::{Piece, Placement};
use hume_scripting::Audience;
use hume_scripting::host::{PasteAudience, PasteQuestion};

use super::buffer::store::BufferStore;
use super::commands::RegisterPaste;
use super::error::CommandError;
use super::event::ON_PASTE;
use super::{Editor, EditorState, commands, doc_ops};

/// What a planned paste pastes, and how it lands.
pub(in crate::editor) enum PasteApply {
    /// `p`/`P` and `[`/`]`, inside a paste session.
    Register(RegisterPaste),
    /// A terminal paste or `paste-text!` over the selections it was planned
    /// on: inside the Insert session's group while one is open on the plan's
    /// pane and buffer, otherwise as one revision.
    Selections(Rc<[String]>),
}

impl PasteApply {
    /// No values: the paste asks its handlers nothing and lands as nothing.
    fn is_empty(&self) -> bool {
        match self {
            Self::Register(paste) => paste.values.is_empty(),
            Self::Selections(values) => values.is_empty(),
        }
    }

    /// The same paste with one value per selection: the handlers' text
    /// where `answers` has one, in the shape of the value it replaces, and
    /// the placed value elsewhere.
    fn answered(self, mut answers: Vec<Option<String>>, selections: usize) -> Self {
        let mut answer = |i: usize| answers.get_mut(i).and_then(Option::take);
        match self {
            Self::Register(mut paste) => {
                let values = &paste.values;
                let answered = per_selection(values, selections, paste_placement, |i, piece| {
                    answer(i).map_or_else(|| piece.clone(), |text| Piece::new(text, piece.shape()))
                });
                paste.answered = Some(answered);
                Self::Register(paste)
            }
            Self::Selections(values) => {
                let values = per_selection(&values, selections, insert_placement, |i, value| {
                    answer(i).unwrap_or_else(|| value.clone())
                });
                Self::Selections(values.into())
            }
        }
    }
}

/// `each` applied to the value each of `selections` gets from `values`, as
/// `place` places them; empty for no values.
fn per_selection<'v, T: Clone, U>(
    values: &'v [T],
    selections: usize,
    place: impl FnOnce(&'v [T], usize) -> Placement<'v, T>,
    mut each: impl FnMut(usize, &T) -> U,
) -> Vec<U> {
    if values.is_empty() {
        return Vec::new();
    }
    let placement = place(values, selections);
    (0..selections).map(|i| each(i, placement.get(i))).collect()
}

/// What a paste's selections cover before it lands: the text and the
/// selections it was planned on, read out only for the handlers.
pub(in crate::editor) struct Replaced {
    text: BufferText,
    sels: SelectionSet,
}

impl Replaced {
    /// `sels` over `text`, the text they were computed for.
    pub(in crate::editor) fn new(text: &BufferText, sels: SelectionSet) -> Self {
        Self {
            text: text.clone(),
            sels,
        }
    }

    /// The selections the paste was planned on.
    pub(in crate::editor) fn sels(&self) -> &SelectionSet {
        &self.sels
    }

    fn len(&self) -> usize {
        EditView::bind(&self.text, &self.sels).len()
    }

    /// What each selection replaces, in document order (`None` at a cursor).
    fn texts(&self) -> Vec<Option<String>> {
        replaced_texts(EditView::bind(&self.text, &self.sels))
    }
}

/// A planned paste, waiting to land.
pub(in crate::editor) struct PastePlan {
    pane: PaneId,
    buffer: BufferId,
    /// The text the plan was made on; the paste lands only on it.
    version: TextVersion,
    replaced: Rc<Replaced>,
    apply: PasteApply,
}

impl PastePlan {
    /// Who handles the paste: the `on-paste` handlers that see the language
    /// of the buffer it lands in, or no language once that buffer is closed.
    pub(in crate::editor) fn audience(&self, buffers: &BufferStore) -> PasteAudience {
        PasteAudience {
            event: ON_PASTE,
            language: buffers
                .try_get(self.buffer)
                .and_then(|b| b.plugin_view.language.clone()),
        }
    }

    /// What the paste asks its handlers: per selection, the text it gets
    /// and the text it replaces.
    pub(in crate::editor) fn question(&self) -> PasteQuestion {
        let selections = self.replaced.len();
        let texts = match &self.apply {
            PasteApply::Register(paste) => {
                per_selection(&paste.values, selections, paste_placement, |_, p| {
                    p.text().to_owned()
                })
            }
            PasteApply::Selections(values) => {
                per_selection(values, selections, insert_placement, |_, v| v.clone())
            }
        };
        PasteQuestion {
            pane: PaneHandle::with_pane(self.buffer, self.pane),
            requests: texts.into_iter().zip(self.replaced.texts()).collect(),
        }
    }
}

/// The paste a planner hands back to the code that asked for it, which
/// finishes it: [`Editor::finish_paste`] outside a script,
/// [`EditorState::park_paste`] for `%finish-paste!` inside one.
#[must_use = "a planned paste lands only when its caller finishes it"]
pub(in crate::editor) struct PlannedPaste(Option<PastePlan>);

impl PlannedPaste {
    /// Nothing to paste.
    pub(in crate::editor) const NONE: Self = Self(None);

    /// A paste into `buffer`'s text as it is now, at selections that
    /// replace what `replaced` covers, placed as `p` places them.
    pub(in crate::editor) fn new(
        buffers: &BufferStore,
        pane: PaneId,
        buffer: BufferId,
        replaced: Rc<Replaced>,
        apply: PasteApply,
    ) -> Self {
        Self(Some(PastePlan {
            pane,
            buffer,
            version: buffers.get(buffer).text().version(),
            replaced,
            apply,
        }))
    }
}

/// The paste waiting for its `on-paste` handlers, if any. Only this
/// module's [`EditorState`] methods change it, so no paste is parked
/// beside another.
#[derive(Default)]
pub(in crate::editor) struct PasteWait(Option<AwaitingPaste>);

impl PasteWait {
    pub(in crate::editor) fn is_empty(&self) -> bool {
        self.0.is_none()
    }
}

/// A paste waiting for its `on-paste` handlers.
enum AwaitingPaste {
    /// Parked for the `%finish-paste!` that runs its handlers: the one
    /// after the script call that planned it, or the one
    /// [`Editor::finish_paste`] runs outside a script.
    Parked(PastePlan),
    /// Taken by a `%finish-paste!`, whose plugins activate and whose
    /// handlers run before it lands.
    Finishing(PastePlan),
}

impl EditorState {
    /// `Err` while a paste waits for its `on-paste` handlers: a plugin the
    /// paste loads cannot paste. Checked before a script's paste plans
    /// anything, so a refused paste leaves nothing behind.
    pub(in crate::editor) fn refuse_paste_while_one_waits(&self) -> Result<(), CommandError> {
        if !self.paste_awaiting_handlers.is_empty() {
            return Err(CommandError::new(
                "cannot paste while another paste waits for its on-paste handlers",
            ));
        }
        Ok(())
    }

    /// Park `planned` for the `%finish-paste!` after the script call that
    /// planned it. Every script paste is refused while another paste waits
    /// ([`Self::refuse_paste_while_one_waits`]), so none waits here.
    pub(in crate::editor) fn park_paste(&mut self, planned: PlannedPaste) {
        if let Some(plan) = planned.0 {
            self.park(plan);
        }
    }

    /// Park `plan`, which no other paste may be waiting beside.
    fn park(&mut self, plan: PastePlan) {
        assert!(
            self.paste_awaiting_handlers.is_empty(),
            "a paste was planned while another paste waits for its on-paste handlers"
        );
        self.paste_awaiting_handlers.0 = Some(AwaitingPaste::Parked(plan));
    }

    /// Start finishing the parked paste. `false` when none is parked, also
    /// while one is already finishing: a call the paste's plugins make while
    /// they activate finishes nothing of its own.
    pub(in crate::editor) fn start_paste_finish(&mut self) -> bool {
        let parked = self
            .paste_awaiting_handlers
            .0
            .take_if(|a| matches!(a, AwaitingPaste::Parked(_)));
        let Some(AwaitingPaste::Parked(plan)) = parked else {
            return false;
        };
        self.paste_awaiting_handlers.0 = Some(AwaitingPaste::Finishing(plan));
        true
    }

    /// The paste being finished, if any.
    pub(in crate::editor) fn finishing_paste(&self) -> Option<&PastePlan> {
        match &self.paste_awaiting_handlers.0 {
            Some(AwaitingPaste::Finishing(plan)) => Some(plan),
            _ => None,
        }
    }

    /// Take the paste being finished, to land it.
    pub(in crate::editor) fn take_finishing_paste(&mut self) -> Option<PastePlan> {
        let finishing = self
            .paste_awaiting_handlers
            .0
            .take_if(|a| matches!(a, AwaitingPaste::Finishing(_)));
        match finishing {
            Some(AwaitingPaste::Finishing(plan)) => Some(plan),
            _ => None,
        }
    }

    /// Plan the paste of `values` at the focused pane's selections, placed
    /// as `p` places them. `Err` when the buffer's text may not change.
    pub(in crate::editor) fn plan_selections_paste(
        &mut self,
        view: &EngineView,
        values: Rc<[String]>,
    ) -> Result<PlannedPaste, CommandError> {
        let pane = view.focused_pane();
        let buf = view.panes[pane].buffer_id();
        doc_ops::text_gate(&self.buffers, buf)?;
        let sels = self.panes.entries.seeded(pane, buf).selections().clone();
        let replaced = Rc::new(Replaced::new(self.buffers.get(buf).text(), sels));
        Ok(PlannedPaste::new(
            &self.buffers,
            pane,
            buf,
            replaced,
            PasteApply::Selections(values),
        ))
    }

    /// Land `plan` with `answers`, the handlers' text for each selection in
    /// order (`None`, or no entry at all: its own). `Err` when the answers
    /// outnumber the selections the handlers were asked about, when the
    /// buffer is gone or its text changed since the plan, when its pane
    /// closed, and when a register paste cannot land
    /// ([`commands::land_register_paste`]); the paste is dropped either way.
    pub(in crate::editor) fn apply_paste_plan(
        &mut self,
        view: &EngineView,
        plan: PastePlan,
        answers: Vec<Option<String>>,
    ) -> Result<(), CommandError> {
        let PastePlan {
            pane,
            buffer,
            version,
            replaced,
            apply,
        } = plan;
        let requests = if apply.is_empty() { 0 } else { replaced.len() };
        if answers.len() > requests {
            return Err(CommandError::new(format!(
                "on-paste: {} answers for {requests} selections",
                answers.len(),
            )));
        }
        let unchanged = self
            .buffers
            .try_get(buffer)
            .is_some_and(|b| b.text().version() == version);
        if !unchanged {
            return Err(CommandError::new(
                "paste dropped: its buffer changed while it waited for its on-paste handlers",
            ));
        }
        if self.panes.entries.pair_mut(pane, buffer).is_none() {
            return Err(CommandError::new(
                "paste dropped: its pane closed while it waited for its on-paste handlers",
            ));
        }
        let apply = if answers.iter().all(Option::is_none) {
            apply
        } else {
            apply.answered(answers, replaced.len())
        };
        match apply {
            PasteApply::Register(paste) => {
                commands::land_register_paste(self, view, pane, buffer, &replaced, paste)
            }
            PasteApply::Selections(values) => {
                let sels = replaced.sels().clone();
                let (buffers, mut stores) = self.edit_parts();
                doc_ops::apply_doc_edit_on(buffers, &mut stores, pane, buffer, sels, |s| {
                    hume_ops::edit::insert_values(s, &values)
                })
            }
        }
    }
}

impl Editor {
    /// Finish the paste `planned` asks for, or report why it was refused.
    pub(in crate::editor) fn finish_planned_paste(
        &mut self,
        planned: Result<PlannedPaste, CommandError>,
    ) {
        match planned {
            Ok(planned) => {
                self.finish_paste(planned);
            }
            Err(e) => self.state.report_error(&e),
        }
    }

    /// Run the `on-paste` handlers for `planned`, if it is a paste, and land
    /// it: outside a script, the finish of every paste a command, a key or
    /// the terminal asked for. `false` when the paste was dropped or its
    /// handlers' script failed, either reported.
    pub(in crate::editor) fn finish_paste(&mut self, planned: PlannedPaste) -> bool {
        let Some(plan) = planned.0 else {
            return true;
        };
        let audience = plan.audience(&self.state.buffers);
        let activates = self
            .scripting
            .as_ref()
            .is_some_and(|host| !host.activation_event_plugins(audience.event).is_empty());
        let seeing = Audience::Seeing(audience.language.as_deref());
        if !activates && !self.has_event_handlers(audience.event, None, seeing) {
            return self.land_paste(plan);
        }
        self.state.park(plan);
        let result = {
            let (host, mut impl_host) = self.scripting_with_host().expect("checked above");
            host.finish_paste(&mut impl_host)
        };
        let finished = result.is_ok();
        self.apply_script_result(result, "on-paste: ");
        finished
    }

    /// Land `plan` with no handler's text. `false` when it was dropped,
    /// which is reported.
    fn land_paste(&mut self, plan: PastePlan) -> bool {
        match self.state.apply_paste_plan(&self.view, plan, Vec::new()) {
            Ok(()) => true,
            Err(e) => {
                self.state.report_error(&e);
                false
            }
        }
    }

    /// Land the paste a script left parked or finishing, if any, with no
    /// handler's text: the script was stopped before it finished it. A
    /// dropped paste is reported.
    pub(in crate::editor) fn land_parked_paste(&mut self) {
        if let Some(AwaitingPaste::Parked(plan) | AwaitingPaste::Finishing(plan)) =
            self.state.paste_awaiting_handlers.0.take()
        {
            self.land_paste(plan);
        }
    }
}
