//! The completion overlay: [`BufferCompletionLayer`] pushed above `Insert`,
//! [`MinibufCompletionLayer`] pushed above `Command`. Two concrete layer
//! types, not one: `completion/session.rs`'s module doc explains why a
//! `Buffer`/`Minibuf` session are different types rather than one enum,
//! and the same split carries into their layers, so `dispatch_at`
//! (`mappings/mod.rs`) routes by ordinary generic layer lookup
//! (`find`/`at`/`ref_of`) rather than a runtime target check. What the two
//! layers' `Layer` impls share (`mode`, `popup_eviction`, `removal_scope`)
//! and menu navigation stay written once, in the free functions below, and
//! called from both.

use termina::event::KeyCode;

use hume_engine::pipeline::{BufferId, EngineView, PaneId, ScreenLayout};
use hume_engine::theme::{ScopeRegistry, Theme};
use hume_engine::types::EditorMode;
use hume_treesitter::registry::LanguageRegistry;
use hume_ui::popup::SideSlot;

use super::super::completion::{
    BufferSession, CompletionItem, Documentation, ItemKey, MinibufSession,
};
use super::super::error::CommandError;
use super::super::keymap::{BindMode, WalkResult};
use super::super::popup_syntax::{build_markup_syntax, popup_content};
use super::super::{Editor, EditorState, Severity};
use super::paste::PastedText;
use super::placement::popup_placement;
use super::popup::{boxed_popup_rows, half_page_key, scroll_box};
use super::stack::{
    InputEvent, Layer, LayerHandler, LayerRef, PasteHandler, PopupEviction, Removal, RemovalScope,
};

/// An open Insert-mode completion session, pushed above whichever base
/// layer opened it: an overlay, not a mode layer (`mode()` returns `None`;
/// `InputStack::mode_layer()` skips it, reading the mode from the layer
/// beneath).
pub(in crate::editor) struct BufferCompletionLayer {
    pub(in crate::editor) session: BufferSession,
    /// The documentation box's content, built the first time the box is
    /// resolved (by a frame, or by Ctrl-d/Ctrl-u paging it), since its
    /// styled runs need the baked theme.
    doc: DocView,
}

impl BufferCompletionLayer {
    pub(in crate::editor) fn new(session: BufferSession) -> Self {
        Self {
            session,
            doc: DocView::default(),
        }
    }

    /// Whether the documentation box has room beside the menu, as last
    /// resolved; true before it is first resolved, when it is not known yet.
    pub(in crate::editor) fn doc_box_fits(&self) -> bool {
        self.doc.fits != Some(false)
    }

    #[cfg(test)]
    pub(in crate::editor) fn doc_builds(&self) -> usize {
        self.doc.built.len()
    }
}

/// The documentation box's per-session content cache, placement and scroll
/// position.
#[derive(Default)]
struct DocView {
    /// The item the box last showed, and whether its docs were known then:
    /// while both hold, resolving the box reuses `current` without asking
    /// the item again.
    shown: Option<(ItemKey, bool)>,
    /// The index in `built` of what the box shows for `shown`, `None` for
    /// an item with nothing to show.
    current: Option<usize>,
    /// Content per text this session: going back to an item, or meeting it
    /// again in its source's next answer, reuses it, markdown parse
    /// included. One entry per distinct text visited, so a linear search
    /// on a selection change is enough.
    built: Vec<(Documentation, hume_ui::popup::PopupContent)>,
    /// First visible wrapped row of `current`, back to 0 whenever `shown`
    /// changes. Rendering clamps a copy; [`scroll_box`] clamps
    /// it before each step.
    scroll: usize,
    /// Whether the box last found room beside the menu, `None` before it is
    /// first resolved.
    fits: Option<bool>,
}

impl DocView {
    /// The index in `built` of `text`'s content, built on first sight.
    fn content_for(
        &mut self,
        text: Documentation,
        languages: &LanguageRegistry,
        theme: &mut Theme,
        registry: &ScopeRegistry,
    ) -> usize {
        if let Some(i) = self.built.iter().position(|(built, _)| *built == text) {
            return i;
        }
        let syntax = text
            .markdown
            .then(|| build_markup_syntax(languages, "markdown", &text.text))
            .flatten();
        let content = popup_content(&text.text, syntax.as_ref(), theme, registry);
        self.built.push((text, content));
        self.built.len() - 1
    }
}

/// [`BufferCompletionLayer`]'s `:` command-line counterpart.
pub(in crate::editor) struct MinibufCompletionLayer {
    pub(in crate::editor) session: MinibufSession,
}

impl Layer for BufferCompletionLayer {
    fn snapshot_mut(&mut self) -> Option<&mut super::PaneSnapshot> {
        None
    }

    fn handler(&self) -> LayerHandler {
        completion_input_buffer
    }
    fn paste_handler(&self) -> PasteHandler {
        dismiss_and_pass_paste
    }
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    /// A `Completion` menu can land directly above a `Popup` (hover, the
    /// `gn`/`gp` diagnostic overlay; both non-modal, so `is_settled_for`
    /// doesn't treat one as the stack having moved). `push_layer` evicts
    /// it on the way in, keeping `PopupLayer`'s "never buried" invariant
    /// true. `LayerOnly`, not the default `Both`: a completion session must
    /// coexist with a `Sticky` signature-help popup sitting in the same
    /// mode layer's slot; evicting `Both` here would silently kill sighelp
    /// on every completion open.
    fn popup_eviction(&self) -> PopupEviction {
        PopupEviction::LayerOnly
    }
    /// A non-modal `Popup` (hover, `gn`/`gp`, a `Sticky` signature-help
    /// popup) can land directly above a completion layer by design. See
    /// this type's own doc and `popup_eviction`'s `LayerOnly` override just
    /// above, which exists for the same coexistence. The default `Stack`
    /// scope would otherwise take that popup down along with the session on
    /// every dismiss, contradicting both.
    fn removal_scope(&self) -> RemovalScope {
        RemovalScope::SelfOnly
    }
    fn tear_down(self: Box<Self>, state: &mut EditorState, _view: &EngineView, _why: Removal) {
        state.close_buffer_completion_views();
    }
    fn theme_changed(
        &mut self,
        _theme: &mut hume_engine::theme::Theme,
        _registry: &hume_engine::theme::ScopeRegistry,
    ) {
        self.doc = DocView::default();
    }
}

impl Layer for MinibufCompletionLayer {
    fn snapshot_mut(&mut self) -> Option<&mut super::PaneSnapshot> {
        None
    }

    fn handler(&self) -> LayerHandler {
        completion_input_minibuf
    }
    fn paste_handler(&self) -> PasteHandler {
        dismiss_and_pass_paste
    }
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    fn popup_eviction(&self) -> PopupEviction {
        PopupEviction::LayerOnly
    }
    fn removal_scope(&self) -> RemovalScope {
        RemovalScope::SelfOnly
    }
    fn tear_down(self: Box<Self>, state: &mut EditorState, _view: &EngineView, _why: Removal) {
        state.views.minibuf_completion.set(None);
    }
}

impl Editor {
    /// Write the Insert-mode completion menu into the shared `PopupState`
    /// Arc: same widget as [`Self::sync_menu_view`] (unwrapped rows,
    /// selected-row styling), but anchored at the leftmost contributing
    /// token's start rather than the live cursor (which drifts as the user
    /// types further into the token). `Buffer`-target only; a
    /// `Minibuf`-target session renders through
    /// [`Editor::sync_minibuf_completion_view`] instead, into its own slot.
    ///
    /// Called every frame from `prepare_frame`'s overlay sync after its
    /// scroll step, with the frame's layout and the cursor cell that step
    /// resolved, same as [`Self::sync_popup_view`]/[`Self::sync_menu_view`].
    pub(in crate::editor) fn sync_completion_menu_view(
        &mut self,
        screen: &ScreenLayout,
        cursor_cell: Option<(u16, u16)>,
    ) {
        let buffer_session_open = self.state.input.buffer_completion().is_some();
        if !buffer_session_open && self.state.views.completion_menu.read().is_none() {
            return;
        }
        let (menu, doc) = self
            .resolve_completion_overlays(screen, cursor_cell)
            .unzip();
        self.state.views.completion_doc.set(doc.flatten());
        self.state.views.completion_menu.set(menu);
    }

    /// The Insert-mode completion menu and its documentation box resolved
    /// against the focused pane's current rect and viewport: what a frame
    /// paints and what Ctrl-d/Ctrl-u page the box against. Resolving the box
    /// also brings the box's model up to the selection (see
    /// [`Self::resolve_completion_doc`]), so paging after a selection move
    /// pages the new candidate's documentation. `None` with no session open
    /// or its anchor out of view. `screen` and `cursor_cell` are
    /// [`popup_placement`]'s.
    fn resolve_completion_overlays(
        &mut self,
        screen: &ScreenLayout,
        cursor_cell: Option<(u16, u16)>,
    ) -> Option<(
        hume_ui::popup::PopupState,
        Option<hume_ui::popup::PopupState>,
    )> {
        // The menu anchor is ranked against one text and reads as absent
        // once the text moves, until the next settle-time
        // `reconcile_completion` pass re-ranks (`scripting_setup.rs`). A pane
        // switch can leave the session on a buffer that isn't the one on
        // screen in that same window; the buffer check below covers it.
        //
        // Sequential borrows of `self.state.input`: the session's shared
        // borrow has to end before `popup_placement` takes `&mut self`.
        //
        // With `completion-docs` on, the rows leave `detail` to the box, and
        // keep it when the box finds no room beside the narrower menu.
        let border = self.state.settings.popup_border;
        let completion_docs = self.state.settings.completion_docs;
        let (_, bid) = self.state.input.completion_owner()?;
        let session = self.state.input.buffer_completion()?;
        let anchor = session.menu_anchor(self.state.buffers.get(bid).text())?;
        let placement = popup_placement(self, screen, cursor_cell, anchor)?;

        let session = self.state.input.buffer_completion()?;
        let menu = |with_detail| {
            let window = hume_ui::popup::menu_window(
                session.len(),
                Some(session.selected()),
                placement.pane_rect,
            );
            let rows = session.rows_in(window.range.clone(), with_detail);
            hume_ui::popup::resolve_menu(&rows, window, placement, border)
        };
        let (menu, slot) = if completion_docs {
            let lean = menu(false);
            match hume_ui::popup::side_slot(lean.rect, placement.anchor.1, placement.pane_rect) {
                Some(slot) => (lean, Some(slot)),
                None => (menu(true), None),
            }
        } else {
            (menu(true), None)
        };

        let doc = if completion_docs && !menu.lines.is_empty() {
            self.resolve_completion_doc(slot)
        } else {
            None
        };
        Some((menu, doc))
    }

    /// The documentation box in `slot` for the selected item, recording
    /// whether there was room for it: `None` with nothing to show or no
    /// room beside the menu.
    fn resolve_completion_doc(
        &mut self,
        slot: Option<SideSlot>,
    ) -> Option<hume_ui::popup::PopupState> {
        let languages = &self.state.config.languages;
        let (_, bid) = self.state.input.completion_owner()?;
        let BufferCompletionLayer { session, doc } =
            self.state.input.find_mut::<BufferCompletionLayer>()?;
        doc.fits = Some(slot.is_some());
        let slot = slot?;
        let (key, item) = session.selected_keyed()?;
        let shown = Some((key, item.docs_known()));
        if doc.shown != shown {
            let fence_lang = self
                .state
                .buffers
                .get(bid)
                .language
                .map_or("", |id| languages.name_of(id));
            doc.current = item.doc_box_text(fence_lang).map(|text| {
                doc.content_for(text, languages, &mut self.view.theme, &self.view.registry)
            });
            doc.shown = shown;
            doc.scroll = 0;
        }
        let (_, content) = doc.built.get_mut(doc.current?)?;
        Some(hume_ui::popup::resolve_side_popup(
            content,
            &slot,
            doc.scroll,
            self.state.settings.popup_border,
        ))
    }
}

impl super::stack::OtherLayers<'_> {
    /// [`super::stack::InputStack::buffer_completion_mut`], among these
    /// layers.
    pub(in crate::editor) fn buffer_completion_mut(&mut self) -> Option<&mut BufferSession> {
        self.find_mut::<BufferCompletionLayer>()
            .map(|l| &mut l.session)
    }
}

/// Named sugar over the generic lookup; `stack.rs` stays agnostic.
impl super::stack::InputStack {
    pub(in crate::editor) fn buffer_completion(&self) -> Option<&BufferSession> {
        self.find::<BufferCompletionLayer>().map(|l| &l.session)
    }

    /// The pane and buffer of the open `Buffer` completion session: those of
    /// the Insert session under it, the only layer that opens one. Every
    /// focus move or buffer switch ends that Insert session and everything
    /// above it.
    pub(in crate::editor) fn completion_owner(&self) -> Option<(PaneId, BufferId)> {
        self.buffer_completion()?;
        let insert = self
            .insert_session()
            .expect("a buffer completion layer sits above the Insert layer");
        Some((insert.pane(), insert.buffer()))
    }

    pub(in crate::editor) fn buffer_completion_mut(&mut self) -> Option<&mut BufferSession> {
        self.find_mut::<BufferCompletionLayer>()
            .map(|l| &mut l.session)
    }

    pub(in crate::editor) fn minibuf_completion(&self) -> Option<&MinibufSession> {
        self.find::<MinibufCompletionLayer>().map(|l| &l.session)
    }

    pub(in crate::editor) fn minibuf_completion_mut(&mut self) -> Option<&mut MinibufSession> {
        self.find_mut::<MinibufCompletionLayer>()
            .map(|l| &mut l.session)
    }

    /// The open completion menu's selected row: `None` when neither layer
    /// is open or a `:` line session has not picked a row yet.
    #[cfg(test)]
    pub(in crate::editor) fn completion_selected(&self) -> Option<usize> {
        match self.buffer_completion() {
            Some(session) => Some(session.selected()),
            None => self.minibuf_completion()?.selected(),
        }
    }
}

/// Both completion layers' paste handler. A paste bypasses per-char
/// refiltering the same way an Insert-mode paste bypasses trigger-char hooks
/// (see `EditorState::apply_insert_mode_paste`'s doc), and has no "cycle"
/// or "accept" reading, so the session is dismissed and the layer below
/// takes the text.
fn dismiss_and_pass_paste(
    state: &mut EditorState,
    view: &EngineView,
    r: LayerRef,
    text: PastedText,
) -> Result<(), CommandError> {
    state.dismiss_completion(view);
    state.fall_through_paste(view, r, text)
}

/// Handles one key while an Insert-mode completion session is open.
/// Tab/Down/BackTab/Up/Enter/Esc are claimed only while the session has
/// at least one visible match, and Ctrl-d/Ctrl-u only while the
/// documentation box also has content past what it shows. A session
/// narrowed to empty (continued typing, or a source still pending) shows
/// no menu, so nothing here should intercept a key: Esc must leave
/// Insert in one press and Enter must insert a newline, as if no session
/// were open. Every other key resolves through the Insert keymap:
/// a bound command (a motion or an edit command) dismisses the session
/// outright; anything else falls through and the session is reconciled
/// with the buffer's new state.
fn completion_input_buffer(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let key = match ev {
        InputEvent::Key(key) => key,
        // A click ends Insert outright via `focus_pane` and takes this layer
        // with it; any other mouse event leaves the cursor and text alone.
        InputEvent::Mouse(mouse) => {
            ed.fall_through(r, InputEvent::Mouse(mouse));
            return;
        }
    };
    let has_matches = ed
        .state
        .input
        .at::<BufferCompletionLayer>(r)
        .is_some_and(|l| !l.session.is_empty());
    if has_matches {
        if let Some(down) = half_page_key(&key) {
            let screen = ed.state.screen_layout(&ed.view);
            let rows = ed
                .resolve_completion_overlays(&screen, None)
                .and_then(|(_, doc)| doc)
                .as_ref()
                .map(boxed_popup_rows);
            if let Some(layer) = ed.state.input.at_mut::<BufferCompletionLayer>(r)
                && scroll_box(&mut layer.doc.scroll, down, rows)
            {
                return;
            }
        }
        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                move_buffer_completion_selection(ed, r, true);
                return;
            }
            KeyCode::BackTab | KeyCode::Up => {
                move_buffer_completion_selection(ed, r, false);
                return;
            }
            KeyCode::Enter => {
                accept_completion_selection(ed, r);
                return;
            }
            KeyCode::Escape => {
                ed.state.dismiss_completion(&ed.view);
                return;
            }
            _ => {}
        }
    }
    // Every key bound in the Insert keymap resolves to a cursor motion
    // or an edit command, neither of which can keep the session's
    // tokens correctly tracked. Walked before delegating, since the
    // command itself may reload the keymap.
    let is_command = matches!(
        ed.resolve_keys(BindMode::Insert, &[key]).walk,
        WalkResult::Leaf(_)
    );
    ed.fall_through(r, InputEvent::Key(key));
    // The callee may have taken this layer with it (a `completion-trigger`
    // re-pushing the session as a fresh layer, or Insert itself exiting and
    // tearing this layer down as part of the same truncate). `r`'s id check
    // catches both: skip rather than write through a stale ref.
    if !ed.state.input.is_live(r) {
        return;
    }
    if is_command {
        ed.state.dismiss_completion(&ed.view);
    } else {
        ed.state.reconcile_completion(&ed.view);
    }
}

/// Handles one key while a minibuffer completion session is open.
/// Tab/Shift-Tab step the selection and immediately splice the proposed
/// edit into the minibuffer (the picked candidate, or the common prefix
/// while none is picked; there is no separate accept step, unlike the
/// Buffer-target's Enter);
/// every other key dismisses the popup first, then falls through
/// unchanged: the minibuffer's own always-eager-apply UX.
fn completion_input_minibuf(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let key = match ev {
        InputEvent::Key(key) => key,
        // A mouse click has no meaningful "cycle" or "accept" reading here,
        // so dismiss and let the minibuffer's own handler see it, same
        // discipline as a paste (`dismiss_and_pass_paste`).
        InputEvent::Mouse(mouse) => {
            ed.state.dismiss_completion(&ed.view);
            ed.fall_through(r, InputEvent::Mouse(mouse));
            return;
        }
    };
    match key.code {
        KeyCode::Tab => {
            move_minibuf_completion_selection(ed, r, true);
            ed.state.apply_minibuf_candidate(r);
            return;
        }
        KeyCode::BackTab => {
            move_minibuf_completion_selection(ed, r, false);
            ed.state.apply_minibuf_candidate(r);
            return;
        }
        KeyCode::Enter => {
            // If the selected candidate names a directory, Enter descends
            // into it instead of confirming the command line: the
            // candidate is already in the input (Tab applied it), so
            // dismiss this session and restart completion for the
            // directory's children, rather than falling through to
            // `Command`'s own Confirm handling. Gated on the item's own
            // declared kind, not its source's identity or its text: any
            // `'minibuf` source's item can opt in this way, and an
            // unrelated candidate that merely ends in `/` (a URL, a
            // namespaced tag) is never mistaken for one.
            let is_dir = ed
                .state
                .input
                .at::<MinibufCompletionLayer>(r)
                .is_some_and(|l| {
                    l.session
                        .selected()
                        .and_then(|row| l.session.selected_item(row))
                        .is_some_and(CompletionItem::is_folder)
                });
            if is_dir {
                ed.state.dismiss_completion(&ed.view);
                ed.start_minibuf_completion();
                return;
            }
        }
        _ => {}
    }
    ed.state.dismiss_completion(&ed.view);
    ed.fall_through(r, InputEvent::Key(key));
}

/// Moves the Insert-mode completion menu's selection by one row. The popup
/// scrolls to keep the selection visible, so the bound is the full ranked
/// candidate list, not just the visible window.
fn move_buffer_completion_selection(ed: &mut Editor, r: LayerRef, forward: bool) {
    if let Some(layer) = ed.state.input.at_mut::<BufferCompletionLayer>(r) {
        layer.session.step_selection(forward);
    }
}

/// [`move_buffer_completion_selection`]'s `Minibuf` counterpart, a no-op
/// on an empty session, reachable here since this key handler has no
/// non-empty guard the way `completion_input_buffer`'s does.
fn move_minibuf_completion_selection(ed: &mut Editor, r: LayerRef, forward: bool) {
    if let Some(layer) = ed.state.input.at_mut::<MinibufCompletionLayer>(r) {
        layer.session.step_selection(forward);
    }
}

/// Accepts the currently-selected completion item through the same
/// gen-checked edit path as `completion-accept!`. The session ends
/// either way (success or failure), matching `EditorHostImpl`'s own
/// `completion_accept`. `Buffer`-target only; a `Minibuf`-target session
/// has no separate accept step (see `completion_input_minibuf`'s doc).
///
/// Wrapped in the same [`Editor::with_dot_capture`] `handle_insert` uses
/// around a bound key's own dispatch: this Enter keypress is outside any
/// Insert-key binding, so there's no `Binding` entry to fall back to
/// (`fallback: None`), and an accept is an interactive input, so it is
/// recorded as its own net edit, or as nothing when it changed no text.
fn accept_completion_selection(ed: &mut Editor, r: LayerRef) {
    let selected = ed
        .state
        .input
        .at::<BufferCompletionLayer>(r)
        .map_or(0, |l| l.session.selected());
    let Some(session) = ed.state.take_buffer_completion(&ed.view) else {
        return;
    };

    ed.with_dot_capture(None, |ed| {
        let accepted = ed
            .state
            .interactive_input("completion accept")
            .and_then(|input| session.accept(&mut ed.state, &ed.view, selected, input));
        if let Err(msg) = accepted {
            ed.report(Severity::Error, msg);
        }
    });
}
