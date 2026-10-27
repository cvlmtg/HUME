//! The fuzzy-picker layer, the last of the twelve to get its own file (see
//! `input_stack/stack.rs`'s own doc). `session.rs` is the Rust-side data
//! store (query, ranked items, streaming state); this file is the layer
//! wrapping it: [`PickerLayer`], its `Layer` impl, [`picker_input`]'s
//! key/mouse handling, its paste handler, and the open/close chokepoints
//! every opener and Steel builtin goes through.

mod session;

use termina::event::{KeyCode, Modifiers};

use hume_core::HostToken;
use hume_engine::pipeline::{EngineView, ScreenLayout};
use hume_engine::types::EditorMode;
use hume_scripting::host::token_to_steel;
use steel::rvals::SteelVal;

use super::super::commands::half_page;
use super::super::error::CommandError;
use super::super::keymap::CanonicalKey;
use super::super::replay::{
    Answer, InputOpener, PendingAnswer, Pick, Picked, ReplayRefusal, WaitingInput,
};
use super::super::{Editor, EditorState};
use super::paste::PastedText;
use super::popup::half_page_key;
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, PasteHandler, Removal};

#[cfg(test)]
pub(in crate::editor) use session::item;
pub(in crate::editor) use session::{PickerItem, PickerSession, picker_items};

pub(in crate::editor) struct PickerLayer {
    session: PickerSession,
    /// The check this picker passed as it opened, holding where its pick
    /// records when it belongs to a recorded change.
    input: WaitingInput,
}

impl Layer for PickerLayer {
    fn snapshot_mut(&mut self) -> Option<&mut super::PaneSnapshot> {
        None
    }

    fn handler(&self) -> LayerHandler {
        picker_input
    }
    fn paste_handler(&self) -> PasteHandler {
        picker_paste
    }
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    /// Allowed from any mode and over any other open layer, key-triggered
    /// like every synchronous opener: dispatch order already proves the
    /// stack is wherever the key path left it, so there is nothing to gate.
    /// Landing above a menu or drawer simply suspends it: its own
    /// stray-input policy (`input_stack/{menu,drawer}.rs`) hands control
    /// back to it once the picker retires, same as `Insert` suspending one
    /// today. Three entry rules, in order: the first two here, the third
    /// via the trait's default `popup_eviction` (`PopupEviction::Both`),
    /// run automatically by `push_layer` right after this returns:
    ///
    /// - Dismisses any open completion session first. A picker opening
    ///   mid-completion (e.g. `picker!` bound to a key pressed while
    ///   completing) must not leave a stale session behind it.
    /// - Retires an already-open picker with `#f` before this one lands:
    ///   the exactly-once callback contract must never have a window where
    ///   a session can be silently dropped without firing.
    /// - Clears any open popup: unlike a menu or drawer, which stay open
    ///   underneath and simply stop seeing input, a `Popup` layer left in
    ///   place would be sandwiched between whatever was below it and the
    ///   picker landing on top: the one case `PopupLayer`'s "never buried"
    ///   invariant requires every opener that could land above it to close
    ///   off itself.
    fn setup(&mut self, state: &mut EditorState, view: &EngineView) {
        state.dismiss_completion(view);
        close_picker(state, view);
    }
    /// Fires `on_select` with `#f` unconditionally, ignoring `why`, unlike
    /// every other layer's `tear_down`, which never fires a Steel callback
    /// (teardown *is* cancel, but the callback itself is queued only from
    /// an explicit accept/cancel arm before the truncate that reaches
    /// here). The picker's "fires exactly once" contract has no such arm to
    /// rely on when it's removed incidentally (buried under an `Insert`/
    /// `Prompt` session that a close-*!`/Rust-internal retirement then
    /// truncates through), so this is the one `tear_down` that fires,
    /// making the contract structural rather than dependent on every
    /// caller routing through `close_picker`/`close_picker_with`. Today
    /// `why` is always `Removal::Incidental` here (nothing ever names a
    /// picker as `truncate_layers`/`retire`'s own target), but firing
    /// regardless of which one it is stays correct either way: reaching
    /// this at all already means nothing has fired the callback yet. The
    /// accept path (`Enter`, `picker-close!`) never runs this: it takes the
    /// layer *by value* via `EditorState::take_layer`, which tears down
    /// everything above the target but not the target itself, and fires
    /// its own callback explicitly instead. `truncate_to_base` (reload) is
    /// the one exit that still drops the callback: `EditorState::truncate_layers`
    /// (this method's only caller) never runs during a reload.
    fn tear_down(mut self: Box<Self>, state: &mut EditorState, _view: &EngineView, _why: Removal) {
        fire_callback(state, &mut self, PickerCallback::OnSelect, None);
    }
}

impl Editor {
    /// Write the open picker session into the shared `PickerViewState` Arc
    /// so `PickerOverlay` can paint it this frame. Synced with the other
    /// overlays in `prepare_frame`, but, unlike the cursor-anchored ones, it
    /// centers in the panes region, so it needs no cursor cell.
    ///
    /// Takes `&mut self`: before snapshotting the visible window, it calls
    /// `PickerSession::move_selection(0, geo.list_rows)`. A delta-0 move is
    /// a pure scroll-clamp against the *current* geometry, so a terminal
    /// resize between the last keystroke and this frame self-heals here
    /// rather than leaving a stale scroll offset from a taller frame.
    pub(in crate::editor) fn sync_picker_view(&mut self, screen: &ScreenLayout) {
        if self.state.input.picker().is_none() && self.state.views.picker.read().is_none() {
            return;
        }

        let geo = hume_ui::picker_panel::panel_geometry(screen.pane_area());
        let resolved = match (self.state.input.picker_mut(), geo) {
            (Some(session), Some(geo)) => {
                session.move_selection(0, geo.list_rows);
                let rows: Vec<String> = session.window(geo.list_rows).map(str::to_string).collect();
                let selected_row =
                    (!rows.is_empty()).then(|| session.selected() - session.scroll());
                Some(hume_ui::picker_panel::PickerViewState {
                    prompt: session.prompt().to_string(),
                    query: session.query().to_string(),
                    rows,
                    selected_row,
                    matched: session.matched_len(),
                    total: session.total_len(),
                    pending: session.is_pending(),
                    rect: geo.rect,
                    list_rows: geo.list_rows,
                    border: self.state.settings.popup_border,
                    truncate: session.truncate(),
                })
            }
            _ => None,
        };

        self.state.views.picker.set(resolved);
    }
}

/// Named sugar over the generic lookup; `stack.rs` stays agnostic.
impl super::stack::InputStack {
    pub(in crate::editor) fn picker(&self) -> Option<&PickerSession> {
        self.find::<PickerLayer>().map(|l| &l.session)
    }

    pub(in crate::editor) fn picker_mut(&mut self) -> Option<&mut PickerSession> {
        self.find_mut::<PickerLayer>().map(|l| &mut l.session)
    }
}

/// The open picker's session, but only if `token` may act on the session
/// itself: the guard for `picker-source-stop!` and a scoped `picker-close!`.
/// A mismatch, or no picker open at all, is expected-normal (a late callback
/// racing a picker the user already closed or replaced), so callers treat
/// `None` as a silent no-op, never an error; none of `PickerSession`'s own
/// mutators re-check the token themselves once a caller has reached one
/// through here or through [`session_for_feed`].
pub(in crate::editor) fn session_for_token(
    state: &mut super::super::EditorState,
    token: HostToken,
) -> Option<&mut PickerSession> {
    state
        .input
        .picker_mut()
        .filter(|session| session.accepts(token))
}

/// The open picker's session, but only if `token` is the one feeds for its
/// current query carry: the guard for `picker-push!`, `picker-replace!` and
/// `picker-source-spawn!`. A live session mints a new feed token on every
/// query change, so a source answering an earlier query is refused here with
/// the same silent `None` as any stale token.
pub(in crate::editor) fn session_for_feed(
    state: &mut super::super::EditorState,
    token: HostToken,
) -> Option<&mut PickerSession> {
    state
        .input
        .picker_mut()
        .filter(|session| session.feed_token() == token)
}

/// Single open chokepoint for the picker, `picker!` and `live-picker!`
/// alike. Entry policy (dismiss-completion,
/// replace-a-live-picker, clear-popups) lives on [`PickerLayer::setup`],
/// run by [`EditorState::push_layer`]. This function is the named door to
/// it, plus the one place the picker passes `EditorState::waiting_input`,
/// whose [`WaitingInput`] the layer holds.
///
/// During `.` the picker never opens: the recorded [`Answer::Pick`] answers
/// it, its callback queued to run after the replayed body with what the
/// recorded [`Picked`] names: the payload of the item with the recorded
/// text among this picker's items (see [`PickerSession::payload_for`]), or
/// the recorded payload. It refuses when the next answer is not a pick, when
/// no item has the recorded text, when a reload forgot the payload, or when
/// the recorded `#:actions` key is not bound here, and runs nothing for a
/// pick that never resolved live. The returned token names no open session,
/// so a later feed through it is a no-op.
///
/// Takes `state`/`view` rather than `&mut Editor` because its production
/// caller, `EditorHostImpl::open_picker`, holds those as disjoint borrows,
/// not a whole `Editor`: it can never reach an `&mut Editor`.
pub(in crate::editor) fn open_picker(
    state: &mut super::super::EditorState,
    view: &EngineView,
    session: PickerSession,
) -> Result<HostToken, String> {
    let token = session.token();
    if let Some(replay) = state.dot_replay.as_mut() {
        replay
            .answer_call(
                |a| match a {
                    Answer::Pick(pick) => Some(pick.clone()),
                    _ => None,
                },
                |pick: Pick| {
                    let proc = match pick.action {
                        None => session.on_select().clone(),
                        Some(key) => session
                            .action_for(key)
                            .ok_or(ReplayRefusal::UnboundAction)?
                            .clone(),
                    };
                    let arg = match pick.picked {
                        None => hume_scripting::Rooted::new(SteelVal::BoolV(false)),
                        Some(Picked::Choice(choice)) => session
                            .payload_for(&choice)
                            .ok_or_else(|| choice.missing())?
                            .clone(),
                        Some(Picked::Payload(payload)) => payload,
                        Some(Picked::PayloadForgotten) => {
                            return Err(ReplayRefusal::ForgottenPayload);
                        }
                    };
                    Ok(PendingAnswer::Call { proc, arg })
                },
            )
            .map_err(|refusal| refusal.to_string())?;
        return Ok(token);
    }
    let input = state
        .waiting_input(InputOpener::Code)
        .map_err(|refusal| refusal.to_string())?;
    state.push_layer(view, PickerLayer { session, input });
    Ok(token)
}

/// Which of a picker's callbacks a close fires.
pub(in crate::editor) enum PickerCallback {
    OnSelect,
    /// The `#:actions` `proc` bound to `key`.
    Action {
        key: CanonicalKey,
        proc: hume_scripting::Callback,
    },
}

/// Fire `callback` for a picker that just closed, with the payload of the
/// item at `choice` (an index into the session's items) or `#f` when nothing
/// was chosen: records the pick under the picker's answer ticket, if it has
/// one, and hands the ticket to the queued call.
fn fire_callback(
    state: &mut super::super::EditorState,
    PickerLayer { session, input }: &mut PickerLayer,
    callback: PickerCallback,
    choice: Option<usize>,
) {
    let (proc, action) = match callback {
        PickerCallback::OnSelect => (session.on_select().clone(), None),
        PickerCallback::Action { key, proc } => (proc, Some(key)),
    };
    let (picked, arg) = match choice {
        Some(i) => {
            let (picked, payload) = session.picked(i);
            (Some(picked), payload)
        }
        None => (None, hume_scripting::Rooted::new(SteelVal::BoolV(false))),
    };
    state.queue_answered_call(
        input.take_answer(),
        Answer::Pick(Pick { action, picked }),
        proc,
        arg,
    );
}

/// Single close chokepoint for the picker: ends the session (if one is
/// open) and fires one callback, `callback`, with `choice` (see
/// [`fire_callback`]). Takes
/// the layer *by value* via `EditorState::take_layer` (tearing down
/// whatever was pushed above it, but not the picker itself) rather than
/// `truncate_layers`, since `PickerLayer::tear_down` would otherwise fire
/// its own `#f` callback right before this fires the real one.
///
/// `Editor::reset_config_state` is a second, deliberate exit from this
/// "fires exactly once" contract: its `reset_input` call drops
/// a still-open picker layer directly (never calling this function, and
/// running no teardown at all) along with the `pending_work` queue this
/// function would have pushed the callback onto. The outgoing engine that
/// owns the callback is seconds from being dropped, so firing it would be
/// observable to nothing.
pub(in crate::editor) fn close_picker_with(
    state: &mut super::super::EditorState,
    view: &EngineView,
    callback: PickerCallback,
    choice: Option<usize>,
) {
    let Some(r) = state.input.ref_of::<PickerLayer>() else {
        return;
    };
    let mut layer = state.take_layer::<PickerLayer>(view, r);
    fire_callback(state, &mut layer, callback, choice);
}

/// `close_picker_with`'s dismissal: fire `on_select` with `#f`.
pub(in crate::editor) fn close_picker(state: &mut super::super::EditorState, view: &EngineView) {
    close_picker_with(state, view, PickerCallback::OnSelect, None);
}

/// Handles one key while the picker is open. Always fully consumes,
/// unlike the menu (a stray key closes it and falls through) or the
/// drawer (a stray key falls through untouched), the picker is
/// full-modal: it owns the entire interaction, so an unrecognized key
/// (Left/Right/Home/Tab/…) is simply consumed and ignored rather than
/// leaking through to whatever mode sits underneath. `r` addresses this
/// layer's own session via [`picker_mut`]/[`queue_query_change`]/
/// [`close_picker_with_selection`], but not retirement: every retirement
/// path goes through [`close_picker`]/[`close_picker_with`], which finds
/// the picker's own ref itself rather than needing this call's.
///
/// `on_select` fires exactly once via `.take()`-equivalent truncate +
/// `queue_steel_call` (never invoked inline), same one-shot discipline
/// as the menu. `visible_rows` comes from the panel geometry of
/// `EditorState::screen_layout`, the layout the next frame's
/// `sync_picker_view` paints at the same terminal size, so a key pages by the
/// rows that frame shows (on a pane area too small for the panel, geometry
/// is `None` and paging is a no-op on the store). A mouse event (click, drag, or
/// wheel) is swallowed the same way: full-modal means the picker owns
/// the pointer too, not just the keyboard, so a click can't move the
/// cursor in the buffer underneath it.
pub(in crate::editor) fn picker_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let key = match ev {
        InputEvent::Key(key) => key,
        InputEvent::Mouse(_) => return,
    };
    let screen = ed.state.screen_layout(&ed.view);
    let visible_rows =
        hume_ui::picker_panel::panel_geometry(screen.pane_area()).map_or(0, |geo| geo.list_rows);

    // Every movement key differs only in the delta passed to
    // `move_selection`, collapsed to one borrow instead of one per key.
    let half_page_step = half_page(visible_rows) as isize;
    let step: Option<isize> = half_page_key(&key)
        .map(|down| {
            if down {
                half_page_step
            } else {
                -half_page_step
            }
        })
        .or_else(|| match key.code {
            KeyCode::Down => Some(1),
            KeyCode::Up => Some(-1),
            KeyCode::Char('n') if key.modifiers.contains(Modifiers::CONTROL) => Some(1),
            KeyCode::Char('p') if key.modifiers.contains(Modifiers::CONTROL) => Some(-1),
            KeyCode::PageDown => Some(visible_rows as isize),
            KeyCode::PageUp => Some(-(visible_rows as isize)),
            _ => None,
        });
    if let Some(delta) = step {
        picker_mut(&mut ed.state, r).move_selection(delta, visible_rows);
        return;
    }

    match key.code {
        KeyCode::Backspace => {
            // `None` (and so no fire) both on an already-empty query and
            // on a non-live session; see `pop_grapheme`'s doc.
            let cb = picker_mut(&mut ed.state, r).pop_grapheme();
            queue_query_change(&mut ed.state, r, cb);
        }
        KeyCode::Enter => {
            // No match (or nothing pushed yet) behaves like Esc: Enter
            // is always a terminal action, never a silent no-op.
            close_picker_with_selection(ed, r, PickerCallback::OnSelect);
        }
        KeyCode::Escape => {
            close_picker(&mut ed.state, &ed.view);
        }
        KeyCode::Char(ch)
            if !key
                .modifiers
                .intersects(Modifiers::CONTROL | Modifiers::ALT) =>
        {
            let cb = picker_mut(&mut ed.state, r).insert_char(ch);
            queue_query_change(&mut ed.state, r, cb);
        }
        _ => {
            // `#:actions`: tried only here, after every built-in key
            // above has already had first refusal, so a declared action
            // can never override movement/Backspace/Enter/Escape/query
            // input (see `PickerSession::action_for`'s doc).
            let key = CanonicalKey::from(key);
            if let Some(proc) = picker_mut(&mut ed.state, r).action_for(key).cloned() {
                close_picker_with_selection(ed, r, PickerCallback::Action { key, proc });
            }
        }
    }
}

/// Joined and flattened to one line, same as a minibuffer paste, then
/// appended to the query in one bulk mutation: one rerank, at most one
/// `on_query_change` callback, instead of one per pasted char.
fn picker_paste(
    state: &mut EditorState,
    _view: &EngineView,
    r: LayerRef,
    text: PastedText,
) -> Result<(), CommandError> {
    let cb = picker_mut(state, r).insert_str(&text.single_line());
    queue_query_change(state, r, cb);
    Ok(())
}

/// Queues `cb`, the `on_query_change` callback a `PickerSession` query
/// mutator (`insert_char`/`pop_grapheme`) just returned, if any, with
/// `(token query)` as it stands right now, via `queue_steel_call`,
/// never invoked inline, same discipline as every other picker callback
/// (`close_picker`). The token is the session's feed token for this query,
/// so everything `live-picker!`'s Scheme wrapper (`bootstrap.scm`) feeds
/// through it is dropped once the query changes again; see
/// `builtins/mod.rs`'s `picker!`/`live-picker!` rationale block. A bare
/// `None` is a silent no-op: the mutator itself already decided there
/// was nothing to fire (a `picker!` session, or a backspace on an
/// already-empty query). Debouncing (a query changes on every
/// keystroke, but a live source shouldn't re-spawn on every one) is
/// composed once inside `live-picker!`'s own wrapper, not hand-wired by
/// every plugin author. Keystrokes are human-rate, unlike
/// `debounce_viewport_change`'s fire site (`timer_bridge.rs`), which is
/// every scroll step of every frame and so earns its own Rust-side
/// coalescer.
fn queue_query_change(state: &mut EditorState, r: LayerRef, cb: Option<hume_scripting::Callback>) {
    let Some(cb) = cb else {
        return;
    };
    let session = picker_mut(state, r);
    let token = session.feed_token();
    let query = session.query().to_string();
    state.queue_steel_call(hume_scripting::SteelCall::new(
        cb,
        vec![token_to_steel(token), SteelVal::StringV(query.into())],
    ));
}

/// The open picker session at `r`, the `Picker` layer that `picker_input`
/// or `picker_paste` was dispatched to.
fn picker_mut(state: &mut EditorState, r: LayerRef) -> &mut PickerSession {
    &mut state
        .input
        .at_mut::<PickerLayer>(r)
        .expect("r names the Picker layer dispatch reached")
        .session
}

/// Close the picker, firing `callback` with the selected item (`#f` when
/// nothing matches), so accepting is always a terminal action rather than a
/// silent no-op.
fn close_picker_with_selection(ed: &mut Editor, r: LayerRef, callback: PickerCallback) {
    let choice = picker_mut(&mut ed.state, r).selected_index();
    close_picker_with(&mut ed.state, &ed.view, callback, choice);
}
