//! The bottom-drawer layer, browsed with Helix-style "stay open while
//! editing" semantics. A drawer is opened by `(show-drawer-list! items
//! on-select)` or by `:ls`.

use std::ops::Range;
use std::sync::Arc;

use steel::rvals::SteelVal;
use termina::event::{KeyCode, Modifiers};

use hume_core::{BufferId, HostToken, NonEmpty, Severity, TruncateEnd};
use hume_engine::pipeline::EngineView;
use hume_engine::theme::{ScopeRegistry, diagnostic_scopes};
use hume_engine::types::EditorMode;
use hume_rope::spanned::SpannedText;
use hume_scripting::SteelCall;
use hume_scripting::host::{DrawerItems, token_to_steel};
use hume_ui::list_row::{ListRow, SpanScopes};

use super::list_row::{RenderCall, RenderedRows, list_row};

use super::super::Editor;
use super::super::EditorState;
use super::super::buffer::store::BufferStore;
use super::super::commands::{FocusedPane, half_page};
use super::paste::ignore_paste;
use super::popup::half_page_key;
use super::stack::{
    InputEvent, Layer, LayerHandler, LayerRef, PasteHandler, Removal, RemovalScope,
};

/// An open drawer's rows, what `Enter` on one does, and its selection.
/// Cleared by `Esc` or `close-drawer!`, not by `Enter` (the drawer stays
/// open across selections).
pub(in crate::editor) struct DrawerLayer {
    pub(in crate::editor) rows: DrawerRows,
    /// The end of an over-long row the drawer cuts, set when it opens and
    /// kept for its lifetime.
    truncate: TruncateEnd,
    pub(in crate::editor) selected: usize,
    /// Index of the first visible row, clamped to keep `selected` in view
    /// whenever the selection moves ([`clamp_drawer_scroll`]).
    pub(in crate::editor) scroll: usize,
    /// Identifies which call opened this drawer; see
    /// [`Self::token`]'s doc.
    token: HostToken,
}

impl DrawerLayer {
    /// Mints a fresh token for this drawer. The constructor is the only
    /// minting site, so every `DrawerLayer` in existence carries a token no
    /// other one has (or ever had): [`EditorState::open_drawer`] takes the
    /// outgoing drawer by value and drops it, rather than mutating an
    /// existing one's token in place, so a stale token a plugin is still
    /// holding can never alias a *different*, later drawer that happens to
    /// reuse the same stack slot.
    pub(in crate::editor) fn new(rows: DrawerRows, truncate: TruncateEnd, selected: usize) -> Self {
        let mut drawer = Self {
            rows,
            truncate,
            selected: 0,
            scroll: 0,
            token: HostToken::mint(),
        };
        drawer.select_clamped(selected);
        drawer
    }

    /// Selects row `selected`, or the last row when it is past the end.
    fn select_clamped(&mut self, selected: usize) {
        self.selected = selected.min(self.rows.last_index());
    }

    /// The drawer `:ls` opens, on the row of `current`.
    pub(in crate::editor) fn buffer_list(
        state: &EditorState,
        current: BufferId,
        registry: &mut ScopeRegistry,
    ) -> Self {
        let list = state.buffer_list(current, registry);
        let selected = list
            .buffers
            .iter()
            .position(|&b| b == current)
            .expect("the focused buffer is in the store");
        // A row's number and name come first, so a row is cut at its tail.
        Self::new(DrawerRows::buffers(list), TruncateEnd::Tail, selected)
    }

    /// Rows the drawer shows from the next frame on: what its band's height
    /// and its paging are measured from. A buffer list shows one row per
    /// buffer `store.iter()` yields, the walk `buffer_list` builds its rows
    /// from. Its listed rows catch up with `store` once per frame
    /// (`EditorState::refresh_buffer_list_drawer`), so a buffer opened or
    /// closed since then already counts here. Selection indexes the rows
    /// listed now, so it bounds itself by `rows.len()` instead.
    pub(in crate::editor) fn item_count(&self, store: &BufferStore) -> usize {
        match self.rows {
            DrawerRows::Buffers { .. } => store.iter().count(),
            DrawerRows::Script { .. } => self.rows.len(),
        }
    }

    /// Rows of items the drawer shows at once on a terminal
    /// `terminal_height` rows tall: the window it pages and fills against.
    pub(in crate::editor) fn visible_rows(
        &self,
        store: &BufferStore,
        terminal_height: u16,
    ) -> usize {
        hume_ui::drawer::band_visible_rows(self.item_count(store), terminal_height)
    }

    /// Replaces a buffer-list drawer's rows and buffers together, keeping the
    /// selection on the buffer it was on, or on the nearest row when that
    /// buffer is gone.
    fn set_buffer_list(&mut self, list: BufferList) {
        let kept = match &self.rows {
            DrawerRows::Buffers { buffers, .. } => buffers.get(self.selected).copied(),
            DrawerRows::Script { .. } => None,
        };
        let selected = kept
            .and_then(|b| list.buffers.iter().position(|&n| n == b))
            .unwrap_or(self.selected);
        self.rows = DrawerRows::buffers(list);
        self.select_clamped(selected);
    }

    /// Identifies this drawer to a Steel owner: `show-drawer-list!`'s return
    /// value, and the shared guard every token-scoped drawer mutation
    /// (`close-drawer!`, `update-drawer-list!`, `drawer-selected-index`)
    /// checks before touching the open drawer. Mirrors
    /// `PickerSession::token`.
    pub(in crate::editor) fn token(&self) -> HostToken {
        self.token
    }

    /// The call of a script's drawer callback with `payload` (the selected
    /// row, or `#f` on close) and this drawer's token, so an owner can tell
    /// its own drawer's close from a replaced one's. `None` for a drawer no
    /// script owns. Every call to the callback is built here.
    fn callback_call(&self, payload: SteelVal) -> Option<SteelCall> {
        let callback = self.rows.on_select()?.clone();
        Some(SteelCall::new(
            callback,
            vec![payload, token_to_steel(self.token)],
        ))
    }
}

/// A drawer's rows and what `Enter` on one does.
pub(in crate::editor) enum DrawerRows {
    /// A script's rows. `Enter` on row `i` calls `on_select` with
    /// `payloads[i]` and the drawer's token. The payloads and the rows are
    /// only ever replaced together, so a rendered row never outlives the key
    /// it was rendered from.
    Script {
        rows: ScriptRows,
        payloads: Vec<hume_scripting::Rooted>,
        on_select: hume_scripting::Callback,
    },
    /// `:ls`'s rows: row `i` names `buffers[i]`, and `Enter` switches the
    /// focused pane to it.
    Buffers {
        rows: Vec<Arc<ListRow>>,
        buffers: Vec<BufferId>,
    },
}

/// How a script's drawer makes the rows it shows.
pub(in crate::editor) enum ScriptRows {
    /// The rows the script gave, one per payload.
    Given(Vec<Arc<ListRow>>),
    /// Rows a render proc makes from their payloads, the script's keys.
    Rendered(RenderedRows),
}

impl DrawerRows {
    /// The rows a script sent, and the callback `Enter` and a close call.
    pub(in crate::editor) fn script(
        items: DrawerItems,
        on_select: hume_scripting::Callback,
        registry: &mut ScopeRegistry,
    ) -> Self {
        let (rows, payloads) = match items {
            DrawerItems::Rows(items) => {
                let (rows, payloads) = items
                    .into_vec()
                    .into_iter()
                    .map(|(display, payload)| (Arc::new(list_row(display, registry)), payload))
                    .unzip();
                (ScriptRows::Given(rows), payloads)
            }
            DrawerItems::Keys { keys, render } => {
                let keys = keys.into_vec();
                (
                    ScriptRows::Rendered(RenderedRows::new(render, keys.len())),
                    keys,
                )
            }
        };
        Self::Script {
            rows,
            payloads,
            on_select,
        }
    }

    fn buffers(list: BufferList) -> Self {
        Self::Buffers {
            rows: list.rows.into_vec().into_iter().map(Arc::new).collect(),
            buffers: list.buffers,
        }
    }

    pub(in crate::editor) fn len(&self) -> usize {
        match self {
            Self::Script { payloads, .. } => payloads.len(),
            Self::Buffers { buffers, .. } => buffers.len(),
        }
    }

    /// The last row's index. There is one: every list the drawer opens on
    /// is [`NonEmpty`].
    fn last_index(&self) -> usize {
        self.len() - 1
    }

    /// The callback a script's drawer calls on `Enter` and on a close.
    fn on_select(&self) -> Option<&hume_scripting::Callback> {
        match self {
            Self::Script { on_select, .. } => Some(on_select),
            Self::Buffers { .. } => None,
        }
    }

    /// Each row as the drawer lists it, before it is cut to a width: the row
    /// given, or for a rendered row the one made, `None` until made.
    #[cfg(test)]
    pub(in crate::editor) fn listed(&self) -> Vec<Option<&ListRow>> {
        match self {
            Self::Script {
                rows: ScriptRows::Given(rows),
                ..
            }
            | Self::Buffers { rows, .. } => rows.iter().map(|row| Some(&**row)).collect(),
            Self::Script {
                rows: ScriptRows::Rendered(rendered),
                ..
            } => rendered.made().collect(),
        }
    }

    /// The call that makes the rows in `window` the render proc has not
    /// made `budget` cells wide. `None` for a drawer with no render proc, or
    /// when it made every row in `window`.
    fn unrendered(&self, window: Range<usize>, budget: usize) -> Option<RenderCall> {
        let Self::Script {
            rows: ScriptRows::Rendered(rendered),
            payloads,
            ..
        } = self
        else {
            return None;
        };
        rendered.unrendered(window, budget, |i| payloads[i].value().clone())
    }

    /// The rows the render proc made, for [`Editor::render_rows_into`] to
    /// store the rows [`Self::unrendered`] asked for.
    fn rendered_mut(&mut self) -> &mut RenderedRows {
        let Self::Script {
            rows: ScriptRows::Rendered(rendered),
            ..
        } = self
        else {
            unreachable!("only a rendered list asks for rows");
        };
        rendered
    }

    /// The rows in `window`, `budget` cells wide: a row given or listed cut
    /// at its `truncate` end, shared when it fits, a rendered row as the
    /// render proc made it at `budget`, `None` until it has.
    fn window(
        &self,
        window: Range<usize>,
        budget: usize,
        truncate: TruncateEnd,
    ) -> Vec<Option<Arc<ListRow>>> {
        match self {
            Self::Script {
                rows: ScriptRows::Given(rows),
                ..
            }
            | Self::Buffers { rows, .. } => rows[window]
                .iter()
                .map(|row| Some(row.fit_shared(budget, truncate)))
                .collect(),
            Self::Script {
                rows: ScriptRows::Rendered(rendered),
                ..
            } => window.map(|i| rendered.at(i, budget).cloned()).collect(),
        }
    }
}

impl Layer for DrawerLayer {
    fn snapshot_mut(&mut self) -> Option<&mut super::PaneSnapshot> {
        None
    }

    fn handler(&self) -> LayerHandler {
        drawer_input
    }
    /// Swallowed like every other key the drawer doesn't bind to
    /// single-line selection/Enter/Esc; the drawer stays open.
    fn paste_handler(&self) -> PasteHandler {
        ignore_paste
    }
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    // No `setup`/`popup_eviction` override: the trait's own default
    // (`PopupEviction::Both`) is right: a `Drawer` is non-modal
    // (`is_modal` below), so it can land directly above a `Popup` the same
    // way a `Menu` can, and `push_layer` evicts it on the way in regardless
    // of which of the two opens second.
    //
    /// Fires `#f` at a Steel drawer's callback when `why` is
    /// [`Removal::Incidental`], i.e. swept up as collateral above some other
    /// target. No concrete path reaches this today: a `Drawer` only ever
    /// lands directly on `Base` (`show-drawer-list!` requires the mode layer
    /// to be `Base` and `:ls` runs from the command line, which has closed
    /// by then; `push_layer`'s eviction clears the one non-modal layer,
    /// `Popup`, that could otherwise sit beneath a fresh one), and `Base` is
    /// never a truncate target. Still branches on `why`, matching
    /// `MenuLayer`'s own fix for the same bug class, rather than leaving
    /// "silent unless swept as collateral" true only by that accident of
    /// what happens to land where today. Stays silent on [`Removal::Explicit`]: an
    /// explicit `close-drawer!` (routed through `EditorState::excise_layer`,
    /// which reaches this as the named target without touching whatever
    /// else is stacked above it, such as an `Insert` session browsing the drawer,
    /// say) must stay silent; `drawer_input`'s own `Esc` arm takes the layer
    /// *by value* via `EditorState::take_firing_false` and fires its own
    /// callback explicitly instead.
    fn tear_down(self: Box<Self>, state: &mut EditorState, _view: &EngineView, why: Removal) {
        if let Removal::Incidental = why {
            super::stack::FiresFalseOnReplace::close_firing_false(self, state);
        }
    }
    /// Non-modal: the drawer is built to be worked over (a stray key falls
    /// through and it stays open), so an async opener's staleness check
    /// (`InputStack::is_settled_for`) must not read "a drawer is open" as
    /// "the stack moved": a code-action menu still needs to open while the
    /// user is browsing one.
    fn is_modal(&self) -> bool {
        false
    }
    /// A `Drawer`'s browse-while-editing design means an `Insert` session or
    /// a code-action `Menu` routinely lands above it, and closing the drawer
    /// must not take that along.
    fn removal_scope(&self) -> RemovalScope {
        RemovalScope::SelfOnly
    }
}

impl EditorState {
    /// Opens `drawer`, replacing any drawer already open. The outgoing
    /// drawer's Steel owner, if it has one, learns it is gone through a
    /// `#f` call. `Removal::Explicit` closes stay silent; `Esc`, a failed
    /// row render and an incidental teardown fire one too.
    pub(in crate::editor) fn open_drawer(
        &mut self,
        view: &EngineView,
        drawer: DrawerLayer,
    ) -> HostToken {
        if let Some(r) = self.input.ref_of::<DrawerLayer>() {
            self.take_firing_false::<DrawerLayer>(view, r);
        }
        let token = drawer.token();
        self.push_layer(view, drawer);
        token
    }

    /// Replaces the open drawer's list in place, keeping the browse session
    /// and the `#:truncate` end the drawer opened with (the single writer
    /// behind `update-drawer-list!`). `selected` is
    /// clamped into the new list; `scroll` is left as-is; a shrunk list
    /// leaving it pointing past the end is
    /// [`Self::clamp_drawer_scroll_to_terminal`]'s job, not this one's; see
    /// that method's own doc for why the two are split. Returns whether the
    /// update applied (`false` when no drawer is open or `token` doesn't
    /// match the open drawer's own, an expected-normal race, never an
    /// error).
    pub(in crate::editor) fn set_drawer_items(
        &mut self,
        token: HostToken,
        rows: DrawerRows,
        selected: usize,
    ) -> bool {
        let Some(r) = self.input.drawer_ref_with_token(token) else {
            return false;
        };
        let drawer = self
            .input
            .at_mut::<DrawerLayer>(r)
            .expect("drawer_ref_with_token names a live DrawerLayer");
        drawer.rows = rows;
        drawer.select_clamped(selected);
        true
    }

    /// Clamps the open drawer's `scroll` to `len.saturating_sub(visible)`,
    /// the last window that still has content in it, against the current
    /// terminal geometry. Called from `Editor::prepare_frame` every frame,
    /// before the drawer's rows are rendered and its view synced, the same
    /// "re-derive from the live model every frame, regardless of what
    /// changed" shape `Editor::sync_drawer_view` itself has, rather than at
    /// each mutation site: a list shrinking (`set_drawer_items`) and a
    /// terminal resize (no drawer mutation at all) both need this same
    /// bound, and a resize is the one trigger no *write*-side clamp could
    /// ever catch, since nothing about the drawer changes when it happens.
    /// One read-time clamp catches both, instead of `clamp_scroll_to_window`
    /// carrying a `len` parameter every caller has to supply. It also keeps
    /// `set_drawer_items` from needing `EditorState` to see the terminal
    /// height at all.
    pub(in crate::editor) fn clamp_drawer_scroll_to_terminal(&mut self, terminal_height: u16) {
        let Some(drawer) = self.input.find_mut::<DrawerLayer>() else {
            return;
        };
        let len = drawer.item_count(&self.buffers);
        let visible = drawer.visible_rows(&self.buffers, terminal_height);
        drawer.scroll =
            hume_ui::menu_box::clamp_scroll_to_window(drawer.selected, drawer.scroll, visible)
                .min(len.saturating_sub(visible));
    }
}

impl super::stack::FiresFalseOnReplace for DrawerLayer {
    fn close_firing_false(self: Box<Self>, state: &mut EditorState) {
        if let Some(call) = self.callback_call(SteelVal::BoolV(false)) {
            state.queue_steel_call(call);
        }
    }
}

/// Named sugar over the generic lookup; `stack.rs` stays
/// agnostic. Read-only: every handler-side mutation addresses its own
/// dispatched-to layer via `at_mut::<DrawerLayer>(r)` instead, so there is
/// no `drawer_mut()` sibling.
impl super::stack::InputStack {
    pub(in crate::editor) fn drawer(&self) -> Option<&DrawerLayer> {
        self.find()
    }

    /// The open drawer, but only if its token is `token`: the shared guard
    /// for every token-scoped drawer mutation (`update-drawer-list!`,
    /// `drawer-selected-index`). Mirrors `picker::session_for_token`'s own
    /// doc and shape: a mismatch, or no drawer open at all, is
    /// expected-normal (a late callback racing a drawer the user already
    /// closed or replaced), so callers treat `None` as a silent no-op, never
    /// an error.
    pub(in crate::editor) fn drawer_with_token(&self, token: HostToken) -> Option<&DrawerLayer> {
        self.drawer().filter(|d| d.token() == token)
    }

    /// [`Self::drawer_with_token`]'s ref-returning counterpart, for
    /// `close-drawer!`, which needs a [`LayerRef`] to excise rather than a
    /// borrow to read.
    pub(in crate::editor) fn drawer_ref_with_token(&self, token: HostToken) -> Option<LayerRef> {
        self.drawer_with_token(token)?;
        self.ref_of::<DrawerLayer>()
    }
}

/// Handles one key while the bottom drawer is open. Ctrl-d/Ctrl-u page by
/// half the visible band, bare Down/Up step one row at a time for fine
/// adjustment, and `Enter` (which runs the drawer's [`DrawerRows`] action
/// repeatedly across a browse session, unlike the menu, without closing the
/// drawer) is handled in place; `Esc` retires the layer and fires `#f` at a
/// Steel drawer's callback; any other key, including `j`/`k`, falls through
/// completely untouched (no close, no callback), leaving the drawer open
/// while focus moves to whatever the fallen-through key does (Helix-style
/// browse-while-editing, so vertical motion in the source buffer keeps
/// working).
///
/// No mode gate of its own, same reasoning as [`super::menu::menu_input`].
/// A mouse event falls through untouched, same as any other key the
/// drawer doesn't bind: browsing never blocks the cursor from moving.
pub(in crate::editor) fn drawer_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    let key = match ev {
        InputEvent::Key(key) => key,
        InputEvent::Mouse(mouse) => {
            ed.fall_through(r, InputEvent::Mouse(mouse));
            return;
        }
    };
    // Every movement key differs only in the delta passed to
    // `move_drawer_selection`, collapsed to one borrow instead of one per
    // key, mirroring `picker_input`'s own step table.
    let half_page_step = half_page(drawer_visible_rows(ed, r)) as isize;
    let step: Option<isize> = half_page_key(&key)
        .map(|down| {
            if down {
                half_page_step
            } else {
                -half_page_step
            }
        })
        .or_else(|| match key.code {
            KeyCode::Down if key.modifiers == Modifiers::NONE => Some(1),
            KeyCode::Up if key.modifiers == Modifiers::NONE => Some(-1),
            _ => None,
        });
    if let Some(delta) = step {
        move_drawer_selection(ed, r, delta);
        return;
    }

    match key.code {
        KeyCode::Enter => select_row(ed, r),
        KeyCode::Escape => {
            ed.state.take_firing_false::<DrawerLayer>(&ed.view, r);
        }
        _ => {
            ed.fall_through(r, InputEvent::Key(key));
        }
    }
}

/// `Enter` on the selected row, per the drawer's [`DrawerRows`].
fn select_row(ed: &mut Editor, r: LayerRef) {
    let drawer = ed
        .state
        .input
        .at::<DrawerLayer>(r)
        .expect("dispatch_at already checked kind(r) == DrawerLayer");
    match &drawer.rows {
        DrawerRows::Script { payloads, .. } => {
            let call = drawer
                .callback_call(payloads[drawer.selected].value().clone())
                .expect("a script's drawer has a callback");
            ed.state.queue_steel_call(call);
        }
        DrawerRows::Buffers { buffers, .. } => {
            let target = buffers[drawer.selected];
            if ed.state.buffers.try_get(target).is_none() {
                ed.report(Severity::Warning, "buffer was closed".to_string());
            } else {
                ed.enter_buffer(FocusedPane::current(&ed.view), target);
            }
            let focused = ed.focused_buffer_id();
            ed.state
                .refresh_buffer_list_drawer(focused, &mut ed.view.registry);
            clamp_drawer_scroll(ed, r);
        }
    }
}

/// The rows of a buffer-list drawer and the buffers they name, in row order.
struct BufferList {
    rows: NonEmpty<ListRow>,
    buffers: Vec<BufferId>,
}

impl EditorState {
    /// Rebuilds the open drawer's rows when it lists buffers and the store,
    /// the markers or the focused buffer have moved since it last did. Runs
    /// from `Editor::prepare_frame` every frame, so a row's number is always
    /// the one `:b <n>` resolves.
    pub(in crate::editor) fn refresh_buffer_list_drawer(
        &mut self,
        current: BufferId,
        registry: &mut ScopeRegistry,
    ) {
        let Some(drawer) = self.input.drawer() else {
            return;
        };
        let DrawerRows::Buffers {
            rows,
            buffers: listed,
        } = &drawer.rows
        else {
            return;
        };
        let list = self.buffer_list(current, registry);
        if *listed == list.buffers && rows.iter().map(|row| &**row).eq(list.rows.iter()) {
            return;
        }
        self.input
            .find_mut::<DrawerLayer>()
            .expect("drawer() found a DrawerLayer")
            .set_buffer_list(list);
    }

    /// One `:ls` row per open buffer, and the buffers they name, in row
    /// order. A row's number is the one `:b <n>` resolves. An unsaved
    /// buffer's row is `diagnostic.warning.message-text`, and the whole row
    /// `diagnostic.warning.message` while selected. The current buffer's row
    /// is otherwise `markup.bold`.
    fn buffer_list(&self, current: BufferId, registry: &mut ScopeRegistry) -> BufferList {
        let alternate = self.buffers.second_most_recent();
        let (rows, buffers) = self
            .buffers
            .iter()
            .enumerate()
            .map(|(i, (id, buf))| {
                let current_marker = if id == current {
                    '%'
                } else if alternate == Some(id) {
                    '#'
                } else {
                    ' '
                };
                let unsaved = self.has_unsaved_changes(id);
                let dirty_marker = if unsaved { '+' } else { ' ' };
                let mut row = format!(
                    "{:>4}  {current_marker}{dirty_marker}  {:<32}  {}",
                    i + 1,
                    buf.display_name(),
                    buf.display_path().unwrap_or_default(),
                );
                row.truncate(row.trim_end().len());
                let whole_row = |row: String, scopes| {
                    let end = row.chars().count();
                    ListRow::styled(
                        SpannedText::new(row, vec![(0, end, scopes)])
                            .expect("a span over a whole non-empty row fits it"),
                    )
                };
                let row = if unsaved {
                    let selected = registry.intern(diagnostic_scopes::WARNING_MESSAGE);
                    let scopes = SpanScopes {
                        scope: registry.intern(diagnostic_scopes::WARNING_MESSAGE_TEXT),
                        selected: Some(selected),
                    };
                    whole_row(row, scopes).with_selected_scope(Some(selected))
                } else if id == current {
                    let scopes = SpanScopes {
                        scope: registry.intern("markup.bold"),
                        selected: None,
                    };
                    whole_row(row, scopes)
                } else {
                    ListRow::plain(row)
                };
                (row, id)
            })
            .unzip();
        BufferList {
            rows: NonEmpty::new(rows).expect("the store always holds a buffer"),
            buffers,
        }
    }
}

/// Number of drawer rows visible at once. Read by [`clamp_drawer_scroll`]
/// so the scroll window always agrees with what's on screen.
fn drawer_visible_rows(ed: &Editor, r: LayerRef) -> usize {
    let Some(drawer) = ed.state.input.at::<DrawerLayer>(r) else {
        return 0;
    };
    drawer.visible_rows(&ed.state.buffers, ed.view.terminal_area().height)
}

/// Moves the drawer's selection by `delta` (clamped to `[0, len - 1]`), then
/// scrolls it into view.
fn move_drawer_selection(ed: &mut Editor, r: LayerRef, delta: isize) {
    let drawer = ed
        .state
        .input
        .at_mut::<DrawerLayer>(r)
        .expect("dispatch_at already checked kind(r) == DrawerLayer");
    let last = drawer.rows.last_index();
    let new = (drawer.selected as isize + delta).clamp(0, last as isize);
    drawer.selected = new as usize;
    clamp_drawer_scroll(ed, r);
}

/// Clamps `drawer.scroll` so `drawer.selected` stays within the visible
/// window (`clamp_scroll_to_window`, shared with `PickerSession::
/// move_selection`).
fn clamp_drawer_scroll(ed: &mut Editor, r: LayerRef) {
    let visible_rows = drawer_visible_rows(ed, r);
    let Some(drawer) = ed.state.input.at_mut::<DrawerLayer>(r) else {
        return;
    };
    drawer.scroll =
        hume_ui::menu_box::clamp_scroll_to_window(drawer.selected, drawer.scroll, visible_rows);
}

impl Editor {
    /// Resolves the drawer's view for this frame, the one writer of
    /// `views.drawer`: renders the visible rows its render proc has not
    /// made at the band's width, then publishes the visible window, a row
    /// given or listed cut to that width at the `#:truncate` end, or `None`
    /// with no drawer open. Runs every frame after the scroll is clamped to
    /// the terminal and before anything paints, so a scroll, a resize or a
    /// shrunk list shows its rows in the frame it happens. A failed render
    /// closes the drawer, queuing `#f` for `on-select`, which the next
    /// settle delivers, and reports the error.
    pub(in crate::editor) fn sync_drawer_view(&mut self) {
        let terminal = self.view.terminal_area();
        let budget = hume_ui::drawer::row_width(terminal.width);
        let Some(drawer) = self.state.input.find_mut::<DrawerLayer>() else {
            self.state.views.drawer.set(None);
            return;
        };
        let visible = drawer.visible_rows(&self.state.buffers, terminal.height);
        let window = drawer.scroll..drawer.scroll + visible;
        if let Some(call) = drawer.rows.unrendered(window.clone(), budget) {
            let cut = drawer.truncate;
            if let Err(message) =
                self.render_rows_into(call, budget, cut, |drawer: &mut DrawerLayer| {
                    drawer.rows.rendered_mut()
                })
            {
                let r = self
                    .state
                    .input
                    .ref_of::<DrawerLayer>()
                    .expect("a render cannot close the drawer");
                self.state.take_firing_false::<DrawerLayer>(&self.view, r);
                self.state.views.drawer.set(None);
                self.report(Severity::Error, format!("drawer render error: {message}"));
                return;
            }
        }
        let drawer = self.state.input.drawer().expect("found above");
        let rows = drawer.rows.window(window, budget, drawer.truncate);
        let selected_row = (!rows.is_empty()).then(|| drawer.selected - drawer.scroll);
        self.state
            .views
            .drawer
            .set(Some(hume_ui::drawer::DrawerViewState {
                rows,
                selected_row,
                border: self.state.settings.popup_border,
            }));
    }
}
