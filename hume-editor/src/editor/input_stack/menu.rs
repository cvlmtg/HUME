//! The selection-menu layer: `(show-menu! items on-select)`'s raw state.

use termina::event::KeyCode;

use hume_core::{HostToken, NonEmpty};
use hume_engine::pipeline::{EngineView, ScreenLayout};
use hume_engine::types::EditorMode;

use super::super::mouse::is_fresh_gesture;
use super::super::replay::{Answer, Choice, InputOpener, PendingAnswer, WaitingInput};
use super::super::{Editor, EditorState};
use super::base::BaseLayer;
use super::paste::ignore_paste;
use super::placement::{focused_cursor, popup_placement};
use super::stack::{
    InputEvent, Layer, LayerHandler, LayerRef, PasteHandler, Removal, RemovalScope,
};

/// `(show-menu! items on-select)`'s raw content, held on `EditorState`
/// until the next frame's `Editor::sync_menu_view` resolves it into a
/// positioned `hume_ui::popup::PopupState` with `selected` set. `callback`
/// fires exactly once (one per selection or dismissal), then the whole
/// model is dropped.
///
/// `rows` is built once at construction, not rebuilt per frame: labels
/// never change during a menu's lifetime (only `selected` does), so
/// `sync_menu_view` slices it to the visible window every frame the menu
/// stays open. Measuring is `resolve_menu`'s own job, done fresh each call
/// from that slice alone.
pub(in crate::editor) struct MenuLayer {
    pub(in crate::editor) rows: NonEmpty<hume_ui::popup::MenuRow>,
    /// One per row: what `callback` gets when that row is chosen.
    payloads: Vec<hume_scripting::Rooted>,
    pub(in crate::editor) selected: usize,
    pub(in crate::editor) callback: hume_scripting::Callback,
    /// Identifies which `show-menu!` call opened this menu; see
    /// [`Self::token`]'s doc.
    token: HostToken,
    /// The check this menu passed as it opened, holding where its choice
    /// records when it belongs to a recorded change.
    input: WaitingInput,
}

/// `show-menu!`'s funnel: opens a menu over `items`, `(label, payload)`
/// rows, whose `callback` gets the chosen row's payload, or `#f`, and
/// returns its token. `Ok(None)` when the request is stale (see
/// `EditorState::async_opener_stale`).
///
/// During `.` the menu never opens: the recorded [`Answer::Menu`] answers
/// it, its callback queued to run after the replayed body with the payload
/// of the item its [`Choice`] names in this run's `items`, refusing when
/// the next answer is not a menu's or no item fits the choice, and running
/// nothing for a menu that never resolved live. Returns `Ok(None)` then,
/// naming no open menu.
pub(in crate::editor) fn open_menu(
    state: &mut EditorState,
    view: &EngineView,
    items: NonEmpty<(String, hume_scripting::Rooted)>,
    callback: hume_scripting::Callback,
) -> Result<Option<HostToken>, String> {
    if let Some(replay) = state.dot_replay.as_mut() {
        replay
            .answer_call(
                |a| match a {
                    Answer::Menu(choice) => Some(choice.clone()),
                    _ => None,
                },
                |choice| {
                    let index = choice
                        .map(|choice| {
                            choice
                                .find(&items, |(label, _)| label.as_str())
                                .ok_or_else(|| choice.missing())
                        })
                        .transpose()?;
                    Ok(PendingAnswer::Call {
                        proc: callback,
                        arg: menu_arg(index.map(|i| &items[i].1)),
                    })
                },
            )
            .map_err(|refusal| refusal.to_string())?;
        return Ok(None);
    }
    // A `Menu` on top is the self-replace exception: a second
    // `lsp-code-action` response while the first menu is still open
    // replaces it rather than being read as stale.
    if state.async_opener_stale::<BaseLayer, MenuLayer>("show-menu!") {
        return Ok(None);
    }
    let input = state
        .waiting_input(InputOpener::Code)
        .map_err(|refusal| refusal.to_string())?;
    // Retires a prior `Menu` on the self-replace path, firing its callback
    // with `#f` explicitly (`take_firing_false`, shared with the drawer):
    // only this path, a genuine refresh, should fire one. Any open popup is
    // `MenuLayer::setup`'s concern, run by `push_layer` below.
    if let Some(r) = state.input.ref_of::<MenuLayer>() {
        state.take_firing_false::<MenuLayer>(view, r);
    }
    let menu = MenuLayer::new(input, items, callback);
    let token = menu.token();
    state.push_layer(view, menu);
    Ok(Some(token))
}

/// The argument a menu's callback gets: the chosen row's payload, or `#f`.
fn menu_arg(payload: Option<&hume_scripting::Rooted>) -> hume_scripting::Rooted {
    hume_scripting::Rooted::new(
        payload.map_or(steel::rvals::SteelVal::BoolV(false), |p| p.value().clone()),
    )
}

impl MenuLayer {
    /// A fresh menu over single-column `items`, selection at row 0, with a
    /// freshly minted token (the only minting site, same as
    /// `DrawerLayer::new`).
    fn new(
        input: WaitingInput,
        items: NonEmpty<(String, hume_scripting::Rooted)>,
        callback: hume_scripting::Callback,
    ) -> Self {
        let mut payloads = Vec::with_capacity(items.len());
        let rows = items.map(|(label, payload)| {
            payloads.push(payload);
            hume_ui::popup::MenuRow::plain(label)
        });
        Self {
            rows,
            payloads,
            selected: 0,
            callback,
            token: HostToken::mint(),
            input,
        }
    }

    /// Queue this menu's callback with the chosen row's payload, or `#f` when
    /// it was dismissed, recording the chosen row's [`Choice`] under its
    /// answer ticket, if it has one. Every way a menu closes with a callback ends
    /// here.
    fn fire(&mut self, state: &mut EditorState, choice: Option<usize>) {
        let picked = choice.map(|i| Choice::of(&self.rows, i, |row| &row.main));
        state.queue_answered_call(
            self.input.take_answer(),
            Answer::Menu(picked),
            self.callback.clone(),
            menu_arg(choice.map(|i| &self.payloads[i])),
        );
    }

    /// A menu pushed directly, for a test of the stack's own behavior.
    #[cfg(test)]
    pub(in crate::editor) fn for_test(
        items: NonEmpty<(String, hume_scripting::Rooted)>,
        callback: hume_scripting::Callback,
    ) -> Self {
        Self::new(WaitingInput::for_test(), items, callback)
    }

    /// `show-menu!`'s return value, checked by `close-menu!` so a caller
    /// only ever closes the menu it opened. Mirrors `DrawerLayer::token`.
    pub(in crate::editor) fn token(&self) -> HostToken {
        self.token
    }
}

impl Layer for MenuLayer {
    fn snapshot_mut(&mut self) -> Option<&mut super::PaneSnapshot> {
        None
    }

    fn handler(&self) -> LayerHandler {
        menu_input
    }
    /// A paste is swallowed without closing the menu: the same "consumes
    /// stray input" treatment the menu gives any other key it doesn't
    /// recognize as a choice, minus the `#f` callback that arm fires: a
    /// paste was never a choice attempt.
    fn paste_handler(&self) -> PasteHandler {
        ignore_paste
    }
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    // No `setup`/`popup_eviction` override: the trait's own default
    // (`PopupEviction::Both`) is exactly right: a `Menu` can land directly
    // above a `Popup` (non-modal, so `is_settled_for` doesn't treat it as
    // the stack having moved), and `push_layer` evicts it on the way in,
    // keeping `PopupLayer`'s "never buried" invariant true. A prior `Menu`
    // on the self-replace path is retired separately (`take_firing_false`,
    // shared with the drawer) before pushing the new one.
    /// Fires `#f` when `why` is [`Removal::Incidental`], swept up as
    /// collateral above some other target. No concrete path reaches this
    /// today: a `Menu` only ever opens while the mode layer is `Base`
    /// (`show_menu`'s own `async_opener_stale::<BaseLayer, MenuLayer>`), and
    /// the only two things that can land above one without `is_settled_for`
    /// reading the stack as moved are a `Popup` (never buried: nothing
    /// lands above it either, so closing it never reaches below to a `Menu`)
    /// and a `Drawer`, which excises in place on close
    /// ([`RemovalScope::SelfOnly`], below) rather than taking a `Menu` above
    /// it along; see `close_drawer_leaves_a_menu_open_when_one_sits_above_it`.
    /// Still branches on `why`, matching `DrawerLayer`'s own fix for the same
    /// bug class, rather than leaving "silent unless swept as collateral"
    /// true only by that accident of what happens to land where today.
    /// Stays silent on [`Removal::Explicit`]: an explicit `close-menu!`
    /// (routed through `EditorState::retire`, which reaches this as the
    /// named target) must stay silent, not fire `#f` on a widget its caller
    /// may already be finishing its own way. `menu_input`'s own retirement
    /// arms (Enter/Escape/a stray key/a fresh mouse gesture) and
    /// `show_menu`'s self-replace path all take the layer *by value* via
    /// `EditorState::take_layer` instead and fire their own callback
    /// explicitly, so neither ever reaches this at all.
    fn tear_down(mut self: Box<Self>, state: &mut EditorState, _view: &EngineView, why: Removal) {
        if let Removal::Incidental = why {
            self.fire(state, None);
        }
    }
    /// A non-modal `Popup` can land directly above a `Menu` by design:
    /// closing the menu must not take it along.
    fn removal_scope(&self) -> RemovalScope {
        RemovalScope::SelfOnly
    }
}

impl super::stack::FiresFalseOnReplace for MenuLayer {
    fn close_firing_false(mut self: Box<Self>, state: &mut EditorState) {
        self.fire(state, None);
    }
}

impl Editor {
    /// Write the current menu content into the shared `PopupState` Arc so
    /// `PopupOverlay` can render it during this frame, with the same geometry rules
    /// as [`Self::sync_popup_view`], but items are shown one-per-line as-is
    /// (no word-wrap: menu entries are short labels, not prose) and
    /// `selected` marks the highlighted row. `screen` and `cursor_cell` are
    /// [`popup_placement`]'s.
    pub(in crate::editor) fn sync_menu_view(
        &mut self,
        screen: &ScreenLayout,
        cursor_cell: Option<(u16, u16)>,
    ) {
        if self.state.input.menu().is_none() {
            // Skip the write-lock when both sides are already None: the common
            // case while no menu is open.
            if self.state.views.menu.read().is_none() {
                return;
            }
            self.state.views.menu.set(None);
            return;
        }

        // Hoisted out of the `and_then` below: resolving the anchor takes
        // `&mut self` (it may walk the pane's display-line map), which cannot overlap
        // the `&self.state.input` that closure's receiver holds.
        let anchor = focused_cursor(self);
        let placement = popup_placement(self, screen, cursor_cell, anchor);
        let border = self.state.settings.popup_border;

        let resolved = placement.and_then(|placement| {
            let model = self.state.input.menu()?;
            let window = hume_ui::popup::menu_window(
                model.rows.len(),
                Some(model.selected),
                placement.pane_rect,
            );
            Some(hume_ui::popup::resolve_menu(
                &model.rows[window.range.clone()],
                window,
                placement,
                border,
            ))
        });

        self.state.views.menu.set(resolved);
    }
}

/// Named sugar over the generic lookup; `stack.rs` stays
/// agnostic. Read-only: every handler-side mutation now addresses its own
/// dispatched-to layer via `at_mut::<MenuLayer>(r)` instead, so there is no
/// `menu_mut()` sibling.
impl super::stack::InputStack {
    pub(in crate::editor) fn menu(&self) -> Option<&MenuLayer> {
        self.find()
    }
}

/// Handles one key while a selection menu is open. Movement is handled
/// in place; `Enter`/`Esc`/a stray key all retire the layer and fire the
/// callback exactly once (one-shot `.take()`-equivalent discipline via
/// `truncate`): `queue_steel_call` never invokes it inline, matching
/// every other Rust→Steel callback in this codebase. A stray key both
/// closes the menu (with a `#f` callback) *and* falls through to normal
/// dispatch this same call.
///
/// No mode gate of its own: `show-menu!` only pushes with the mode
/// layer at `Base` (its own async-staleness check, `host_impl/ui.rs`),
/// and `push_mode_layer` always pushes a new mode layer *above* whatever
/// overlay sits on `Base`, so a `Menu` layer is dispatch's top only
/// while the mode layer beneath it is still `Base`.
///
/// A fresh mouse press or wheel notch gets the same treatment as a
/// stray key (close with `#f`, then fall through); a release, drag, or
/// move is the tail of a gesture already in flight (see
/// [`is_fresh_gesture`]'s doc) and falls through untouched, leaving the
/// menu open.
pub(in crate::editor) fn menu_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let key = match ev {
        InputEvent::Key(key) => key,
        InputEvent::Mouse(mouse) => {
            if !is_fresh_gesture(mouse.kind) {
                ed.fall_through(r, InputEvent::Mouse(mouse));
                return;
            }
            take_menu(ed, r).fire(&mut ed.state, None);
            ed.fall_through(r, InputEvent::Mouse(mouse));
            return;
        }
    };
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            let menu = ed
                .state
                .input
                .at_mut::<MenuLayer>(r)
                .expect("dispatch_at already checked kind(r) == MenuLayer");
            if menu.selected + 1 < menu.rows.len() {
                menu.selected += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let menu = ed
                .state
                .input
                .at_mut::<MenuLayer>(r)
                .expect("dispatch_at already checked kind(r) == MenuLayer");
            menu.selected = menu.selected.saturating_sub(1);
        }
        KeyCode::Enter => {
            let mut menu = take_menu(ed, r);
            let selected = menu.selected;
            menu.fire(&mut ed.state, Some(selected));
        }
        KeyCode::Escape => take_menu(ed, r).fire(&mut ed.state, None),
        _ => {
            take_menu(ed, r).fire(&mut ed.state, None);
            ed.fall_through(r, InputEvent::Key(key));
        }
    }
}

/// Take the menu at `r` off the stack, handing back its model so the
/// caller can fire the one callback this layer owes
/// (`.take()`-equivalent one-shot discipline via `EditorState::take_layer`).
fn take_menu(ed: &mut Editor, r: LayerRef) -> MenuLayer {
    *ed.state.take_layer::<MenuLayer>(&ed.view, r)
}
