//! The fuzzy-picker layer — the last of the twelve to get its own file (see
//! `input_stack/stack.rs`'s own doc). `session.rs` is the Rust-side data
//! store (query, ranked items, streaming state); this file is the layer
//! wrapping it: [`PickerLayer`], its `Layer` impl, [`picker_input`]'s
//! key/paste/mouse handling, and the open/close chokepoints every opener
//! and Steel builtin goes through.

mod session;

use termina::event::{KeyCode, Modifiers};

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;
use steel::rvals::SteelVal;

use super::super::commands::half_page;
use super::super::minibuf::flatten_single_line;
use super::super::{Editor, EditorState};
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, Removal};

#[cfg(test)]
pub(in crate::editor) use session::item;
pub(in crate::editor) use session::{PickerItem, PickerSession, picker_items};

/// A fuzzy-picker layer. Wraps `PickerSession` directly — unlike the closed
/// `enum` this crate's stack replaced, `Box<dyn Layer>` already indirects
/// every layer uniformly, so there's no need to box the session a second
/// time just to keep this layer's own footprint from sizing anything else
/// (the reason the old `enum` boxed it: `clippy::large_enum_variant`, a
/// concern only a shared inline `enum` has).
pub(in crate::editor) struct PickerLayer(pub(in crate::editor) PickerSession);

impl Layer for PickerLayer {
    fn handler(&self) -> LayerHandler {
        picker_input
    }
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    /// Allowed from any mode and over any other open layer, key-triggered
    /// like every synchronous opener: dispatch order already proves the
    /// stack is wherever the key path left it, so there is nothing to gate.
    /// Landing above a menu or drawer simply suspends it — its own
    /// stray-input policy (`input_stack/{menu,drawer}.rs`) hands control
    /// back to it once the picker retires, same as `Insert` suspending one
    /// today. Three entry rules, in order — the first two here, the third
    /// via the trait's default `popup_eviction` (`PopupEviction::Both`),
    /// run automatically by `push_layer` right after this returns:
    ///
    /// - Dismisses any open completion session first — a picker opening
    ///   mid-completion (e.g. `picker!` bound to a key pressed while
    ///   completing) must not leave a stale session behind it.
    /// - Retires an already-open picker with `#f` before this one lands —
    ///   the exactly-once callback contract must never have a window where
    ///   a session can be silently dropped without firing.
    /// - Clears any open popup: unlike a menu or drawer, which stay open
    ///   underneath and simply stop seeing input, a `Popup` layer left in
    ///   place would be sandwiched between whatever was below it and the
    ///   picker landing on top — the one case `PopupLayer`'s "never buried"
    ///   invariant requires every opener that could land above it to close
    ///   off itself.
    fn setup(&mut self, state: &mut EditorState, view: &EngineView) {
        state.dismiss_completion(view);
        close_picker(state, view, SteelVal::BoolV(false));
    }
    /// Fires `on_select` with `#f` unconditionally, ignoring `why` — unlike
    /// every other layer's `tear_down`, which never fires a Steel callback
    /// (teardown *is* cancel, but the callback itself is queued only from
    /// an explicit accept/cancel arm before the truncate that reaches
    /// here). The picker's "fires exactly once" contract has no such arm to
    /// rely on when it's removed incidentally — buried under an `Insert`/
    /// `Prompt` session that a close-*!`/Rust-internal retirement then
    /// truncates through — so this is the one `tear_down` that fires,
    /// making the contract structural rather than dependent on every
    /// caller routing through `close_picker`/`close_picker_with`. Today
    /// `why` is always `Removal::Incidental` here — nothing ever names a
    /// picker as `truncate_layers`/`retire`'s own target — but firing
    /// regardless of which one it is stays correct either way: reaching
    /// this at all already means nothing has fired the callback yet. The
    /// accept path (`Enter`, `picker-close!`) never runs this: it takes the
    /// layer *by value* via `EditorState::take_layer`, which tears down
    /// everything above the target but not the target itself, and fires
    /// its own callback explicitly instead. `truncate_to_base` (reload) is
    /// the one exit that still drops the callback: `EditorState::truncate_layers`
    /// — this method's only caller — never runs during a reload.
    fn tear_down(&mut self, state: &mut EditorState, _view: &EngineView, _why: Removal) {
        state.queue_steel_call(self.0.on_select().clone(), vec![SteelVal::BoolV(false)]);
    }
}

impl Editor {
    /// Write the open picker session into the shared `PickerViewState` Arc
    /// so `PickerOverlay` can paint it this frame. Same step-10 timing as
    /// `sync_popup_view`/`sync_menu_view`/`sync_completion_menu_view` (needs
    /// `last_pane_area`, set in step 9) but, unlike them, centers in the
    /// panes region rather than anchoring at the cursor — no `RenderContext`
    /// needed.
    ///
    /// Takes `&mut self`: before snapshotting the visible window, it calls
    /// `PickerSession::move_selection(0, geo.list_rows)` — a delta-0 move is
    /// a pure scroll-clamp against the *current* geometry, so a terminal
    /// resize between the last keystroke and this frame self-heals here
    /// rather than leaving a stale scroll offset from a taller frame.
    pub(in crate::editor) fn sync_picker_view(&mut self) {
        if self.state.input.picker().is_none() && self.state.views.picker.read().is_none() {
            return;
        }

        let geo = hume_ui::picker_panel::panel_geometry(self.view.last_pane_area);
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

/// Named sugar over the generic lookup — the ~350 existing call sites
/// (`ed.state.input.picker()`) stay as they are, and `stack.rs` stays agnostic.
impl super::stack::InputStack {
    pub(in crate::editor) fn picker(&self) -> Option<&PickerSession> {
        self.find::<PickerLayer>().map(|l| &l.0)
    }

    pub(in crate::editor) fn picker_mut(&mut self) -> Option<&mut PickerSession> {
        self.find_mut::<PickerLayer>().map(|l| &mut l.0)
    }
}

/// The open picker's session, but only if its token is `token` — the shared
/// guard for every token-scoped picker mutation (`picker-push!`,
/// `picker-replace!`, `picker-source-spawn!`, `picker-source-stop!`, a
/// scoped `picker-close!`). A mismatch, or no picker open at all, is
/// expected-normal — a late callback racing a picker the user already
/// closed or replaced — so callers treat `None` as a silent no-op, never an
/// error; none of `PickerSession`'s own mutators re-check the token
/// themselves once a caller has reached one through here.
pub(in crate::editor) fn session_for_token(
    state: &mut super::super::EditorState,
    token: u64,
) -> Option<&mut PickerSession> {
    state
        .input
        .picker_mut()
        .filter(|session| session.token() == token)
}

/// Single open chokepoint for the picker — `hume-scripting`'s `picker!`
/// builtin (`ui::picker`) calls this via `EditorHostImpl`. Entry policy
/// (dismiss-completion, replace-a-live-picker, clear-popups) lives on
/// [`PickerLayer::setup`], run by [`EditorState::push_layer`] — this
/// function is just the named door to it. Takes `state`/`view` rather than
/// `&mut Editor` because its production caller, `EditorHostImpl::open_picker`,
/// holds those as disjoint borrows, not a whole `Editor` — it can never
/// reach an `&mut Editor`.
pub(in crate::editor) fn open_picker(
    state: &mut super::super::EditorState,
    view: &EngineView,
    session: PickerSession,
) {
    state.push_layer(view, PickerLayer(session));
}

/// Single close chokepoint for the picker: ends the session (if one is
/// open) and fires exactly one callback with `payload` — `on_select` unless
/// `callback` overrides it. Shared by `Esc`, `Enter` (with the selected
/// payload), a bound `#:actions` key, `picker-close!`, and `open_picker`'s
/// replace-on-open path — one chokepoint, not one copy per caller. Takes
/// the layer *by value* via `EditorState::take_layer` (tearing down
/// whatever was pushed above it, but not the picker itself) rather than
/// `truncate_layers`, since `PickerLayer::tear_down` would otherwise fire
/// its own `#f` callback right before this fires the real one.
///
/// `Editor::reset_config_state` is a second, deliberate exit from this
/// "fires exactly once" contract: its `input.truncate_to_base()` call drops
/// a still-open picker layer directly (never calling this function, and
/// running no teardown at all) along with the `pending_work` queue this
/// function would have pushed the callback onto — the outgoing engine that
/// owns the callback is seconds from being dropped, so firing it would be
/// observable to nothing.
pub(in crate::editor) fn close_picker_with(
    state: &mut super::super::EditorState,
    view: &EngineView,
    callback: Option<SteelVal>,
    payload: SteelVal,
) {
    let Some(r) = state.input.ref_of::<PickerLayer>() else {
        return;
    };
    let session = state.take_layer::<PickerLayer>(view, r).0;
    let callback = callback.unwrap_or_else(|| session.on_select().clone());
    state.queue_steel_call(callback, vec![payload]);
}

/// `close_picker_with`'s common case: fire `on_select` itself.
pub(in crate::editor) fn close_picker(
    state: &mut super::super::EditorState,
    view: &EngineView,
    payload: SteelVal,
) {
    close_picker_with(state, view, None, payload);
}

/// Handles one key while the picker is open. Always fully consumes —
/// unlike the menu (a stray key closes it and falls through) or the
/// drawer (a stray key falls through untouched), the picker is
/// full-modal: it owns the entire interaction, so an unrecognized key
/// (Left/Right/Home/Tab/…) is simply consumed and ignored rather than
/// leaking through to whatever mode sits underneath. `r` addresses this
/// layer's own session via [`picker_mut`]/[`queue_query_change`]/
/// [`close_picker_with_selection`] — but not retirement: every retirement
/// path goes through [`close_picker`]/[`close_picker_with`], which finds
/// the picker's own ref itself rather than needing this call's.
///
/// `on_select` fires exactly once via `.take()`-equivalent truncate +
/// `queue_steel_call` (never invoked inline) — same one-shot discipline
/// as the menu. `visible_rows` comes from `panel_geometry` against the
/// same `last_pane_area` the next frame's `sync_picker_view` will use,
/// so a keystroke and the following paint always agree on how many rows
/// are visible (before the first frame, geometry is `None` and paging is
/// a documented no-op on the store). A mouse event — click, drag, or
/// wheel — is swallowed the same way: full-modal means the picker owns
/// the pointer too, not just the keyboard, so a click can't move the
/// cursor in the buffer underneath it.
pub(in crate::editor) fn picker_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let key = match ev {
        InputEvent::Key(key) => key,
        // Flattened to one line, same as a minibuffer paste, then
        // appended to the query in one bulk mutation — one rerank, at
        // most one `on_query_change` callback, instead of one per
        // pasted char.
        InputEvent::Paste(text) => {
            let cb = picker_mut(ed, r).insert_str(&flatten_single_line(&text));
            queue_query_change(ed, r, cb);
            return;
        }
        InputEvent::Mouse(_) => return,
    };
    let visible_rows = hume_ui::picker_panel::panel_geometry(ed.view.last_pane_area)
        .map_or(0, |geo| geo.list_rows);

    // Every movement key differs only in the delta passed to
    // `move_selection` — collapsed to one borrow instead of one per key.
    // `half_page_step` reads `visible_rows` once up front rather than
    // recomputing it in each of the two Ctrl-d/Ctrl-u arms.
    let half_page_step = half_page(visible_rows) as isize;
    let step: Option<isize> = match key.code {
        KeyCode::Down => Some(1),
        KeyCode::Up => Some(-1),
        KeyCode::Char('n') if key.modifiers.contains(Modifiers::CONTROL) => Some(1),
        KeyCode::Char('p') if key.modifiers.contains(Modifiers::CONTROL) => Some(-1),
        KeyCode::PageDown => Some(visible_rows as isize),
        KeyCode::PageUp => Some(-(visible_rows as isize)),
        KeyCode::Char('d') if key.modifiers.contains(Modifiers::CONTROL) => Some(half_page_step),
        KeyCode::Char('u') if key.modifiers.contains(Modifiers::CONTROL) => Some(-half_page_step),
        _ => None,
    };
    if let Some(delta) = step {
        picker_mut(ed, r).move_selection(delta, visible_rows);
        return;
    }

    match key.code {
        KeyCode::Backspace => {
            // `None` (and so no fire) both on an already-empty query and
            // on a non-live session — see `pop_grapheme`'s doc.
            let cb = picker_mut(ed, r).pop_grapheme();
            queue_query_change(ed, r, cb);
        }
        KeyCode::Enter => {
            // No match (or nothing pushed yet) behaves like Esc — Enter
            // is always a terminal action, never a silent no-op.
            close_picker_with_selection(ed, r, None);
        }
        KeyCode::Escape => {
            close_picker(&mut ed.state, &ed.view, SteelVal::BoolV(false));
        }
        KeyCode::Char(ch)
            if !key
                .modifiers
                .intersects(Modifiers::CONTROL | Modifiers::ALT) =>
        {
            let cb = picker_mut(ed, r).insert_char(ch);
            queue_query_change(ed, r, cb);
        }
        _ => {
            // `#:actions` — tried only here, after every built-in key
            // above has already had first refusal, so a declared action
            // can never override movement/Backspace/Enter/Escape/query
            // input (see `PickerSession::action_for`'s doc).
            if let Some(proc) = picker_mut(ed, r).action_for(key).cloned() {
                close_picker_with_selection(ed, r, Some(proc));
            }
        }
    }
}

/// Queues `cb` — the `on_query_change` callback a `PickerSession` query
/// mutator (`insert_char`/`pop_grapheme`) just returned, if any — with
/// `(token query)` as it stands right now, via `queue_steel_call`,
/// never invoked inline, same discipline as every other picker callback
/// (`close_picker`). The token lets `live-picker!`'s Scheme wrapper
/// (`bootstrap.scm`) spawn/stop/replace against the right session
/// without closing over a not-yet-bound `define` — see
/// `builtins/mod.rs`'s `picker!`/`live-picker!` rationale block. A bare
/// `None` is a silent no-op: the mutator itself already decided there
/// was nothing to fire (a `picker!` session, or a backspace on an
/// already-empty query). Debouncing (a query changes on every
/// keystroke, but a live source shouldn't re-spawn on every one) is
/// composed once inside `live-picker!`'s own wrapper, not hand-wired by
/// every plugin author — keystrokes are human-rate, unlike
/// `debounce_viewport_change`'s fire site (`timer_bridge.rs`), which is
/// every scroll step of every frame and so earns its own Rust-side
/// coalescer.
fn queue_query_change(ed: &mut Editor, r: LayerRef, cb: Option<SteelVal>) {
    let Some(cb) = cb else {
        return;
    };
    let session = picker_mut(ed, r);
    let token = session.token();
    let query = session.query().to_string();
    ed.state.queue_steel_call(
        cb,
        vec![
            SteelVal::IntV(token as isize),
            SteelVal::StringV(query.into()),
        ],
    );
}

/// The open picker session at `r` — only ever called from `picker_input`,
/// which `dispatch_at` only reaches while a `Picker` layer is on top, so
/// `r` always names it. Address-based, not the `InputStack::picker_mut`
/// sugar (`find_mut`-based, and still used as-is by render-sync): every
/// caller here already has `r` from its own dispatch.
fn picker_mut(ed: &mut Editor, r: LayerRef) -> &mut PickerSession {
    &mut ed
        .state
        .input
        .at_mut::<PickerLayer>(r)
        .expect("picker_input is only called while a Picker layer is on top of the stack")
        .0
}

/// Close the picker, firing `callback` (or `on_select` when `None`) with
/// the selected payload — `#f` when nothing matches, so accepting is
/// always a terminal action rather than a silent no-op.
fn close_picker_with_selection(ed: &mut Editor, r: LayerRef, callback: Option<SteelVal>) {
    let payload = picker_mut(ed, r)
        .selected_payload()
        .cloned()
        .unwrap_or(SteelVal::BoolV(false));
    close_picker_with(&mut ed.state, &ed.view, callback, payload);
}
