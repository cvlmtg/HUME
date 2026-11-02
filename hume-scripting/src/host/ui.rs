//! Cursor-anchored popup, selection menu, bottom drawer, minibuffer
//! prompt, and the fuzzy-finder picker.
//!
//! Each is a layer on the editor's input stack, and the topmost wins. A
//! synchronous opener (a key or `:` command) can push above anything; an
//! async opener (a Steel callback answering an earlier request) first checks
//! whether the stack has moved since. Each method states its own gate.

use steel::rvals::SteelVal;
use termina::event::KeyEvent;

use hume_core::{HostToken, NonEmpty, PopupLayout, TruncateEnd};
use hume_rope::spanned::SpannedText;

use super::FocusedHandle;

/// A list row's display as a script gave it (a drawer row, a menu row, a
/// row a picker's `#:render` returns): a string, or `(hash 'text t)` with
/// optional `'spans` naming theme scopes for parts of `t` and an optional
/// `'selected-scope` that styles the row in place of the list's selected-row
/// style while it is selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowDisplay {
    pub content: SpannedText<RowSpanScopes>,
    pub selected_scope: Option<String>,
}

/// A row span's `'scope`, and its `'selected-scope`, which takes the place
/// of `'scope` while the row is the selected one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowSpanScopes {
    pub scope: String,
    pub selected: Option<String>,
}

/// A picker item as a `#:render` proc gets it: `(display . payload)`, the
/// shape a script gives `picker!` its items in.
pub fn display_item(display: &str, payload: SteelVal) -> SteelVal {
    crate::builtins::args::cons_pair(SteelVal::StringV(display.into()), payload)
        .expect("cons accepts any two values")
}

/// A drawer's rows, as `show-drawer-list!`/`update-drawer-list!` received
/// them.
#[derive(Debug)]
pub enum DrawerItems {
    /// Rows already rendered: each row's display and the payload
    /// `on-select` gets for it.
    Rows(NonEmpty<(RowDisplay, crate::Rooted)>),
    /// One opaque key per row, also its payload. Before a frame shows rows
    /// not yet rendered at the drawer's current width, the editor calls
    /// `(render rows budget)` with those rows' keys and the cells one row
    /// gets, which must return one display per key, in either of
    /// [`RowDisplay`]'s forms.
    Keys {
        keys: NonEmpty<crate::Rooted>,
        render: crate::Callback,
    },
}

hume_core::named_enum! {
    /// How an open popup reacts to key and mouse input: `show-popup!`'s
    /// `#:kind` symbol.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum PopupKind {
        /// Ignores keys and mouse input. Lives in the current editing mode's
        /// slot (`Base`/`Insert` only, so `'sticky` under any other layer is
        /// a no-op) and closes when that mode ends, on `close-popup!`, or on
        /// the next `show-popup!`. The default when `#:kind` is omitted.
        Sticky => "sticky",
        /// Ctrl-u/Ctrl-d scroll the content and are consumed while it
        /// overflows one screenful; any other key or mouse event, and
        /// Ctrl-u/d with nothing to scroll, closes the popup and falls
        /// through to normal dispatch. Serves both hover and the `gn`/`gp`
        /// diagnostic overlay, which behave alike once content fits.
        Scrollable => "scrollable",
    }
}

/// One `picker!`/`live-picker!` keymap entry: the key spec as the script
/// wrote it (named in an error when two specs name the same key), the key
/// it names, and the binding, called with the selected item's payload (`#f`
/// when nothing matches) and the picker's token.
pub struct PickerBinding {
    pub spec: String,
    pub key: KeyEvent,
    pub binding: crate::Callback,
}

hume_core::named_enum! {
    /// What `picker-move!`'s count counts: its `#:by` symbol.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum PickerStep {
        Row => "row",
        /// As many rows as the panel shows.
        Page => "page",
        /// Half the rows the panel shows.
        HalfPage => "half-page",
    }
}

/// The keyword options `picker!` and `live-picker!` share. `Default` is
/// every keyword's own default.
#[derive(Default)]
pub struct PickerCommonOpts {
    /// `#:prompt`: label painted before the query in the input line.
    pub prompt: String,
    /// `#:query`: the query the picker opens with. `picker!` filters its
    /// items by it at construction. For `live-picker!`, the Scheme wrapper
    /// runs the source once for a non-empty value, undebounced, right after
    /// [`UiHost::open_live_picker`] returns; `on_query_change` is never
    /// fired for it.
    pub query: String,
    /// `#:truncate`: see [`TruncateEnd`].
    pub truncate: TruncateEnd,
    /// `#:render`: see [`UiHost::open_picker`].
    pub render: Option<crate::Callback>,
    /// `#:on-close`: see [`UiHost::open_picker`].
    pub on_close: Option<crate::Callback>,
}

/// `picker!`'s keyword options; its positional `items` and `keymap` are
/// [`UiHost::open_picker`]'s own parameters.
#[derive(Default)]
pub struct PickerOpts {
    pub common: PickerCommonOpts,
    /// `#:pending`: see [`UiHost::open_picker`].
    pub pending: bool,
}

/// `live-picker!`'s keyword options. No `Default`: `on_query_change` always
/// comes from `live-picker!`'s own requery lambda.
pub struct LivePickerOpts {
    pub common: PickerCommonOpts,
    /// Fired with `(token query)` on every query-changing keystroke, in
    /// place of a local fuzzy filter; feeds for that query must carry
    /// `token`. Always `live-picker!`'s stop-then-debounce wrapper around
    /// the caller's `#:source`.
    pub on_query_change: crate::Callback,
}

/// `picker-source-spawn!`'s keyword options. No `Default`: the keyword
/// defaults to `'(0)`, and an empty `ok_exit_codes` would count every exit
/// as a failure.
pub struct PickerSourceOpts {
    /// `#:cwd`: working directory for the spawned process.
    pub cwd: std::path::PathBuf,
    /// `#:nul`: split stdout on NUL bytes instead of newlines.
    pub nul: bool,
    /// `#:ok-exit-codes`: the complete set of exit codes that count as a
    /// normal outcome, `0` included only if listed. `rg`, which exits `1` on
    /// no matches, needs `'(0 1)`.
    pub ok_exit_codes: Vec<i32>,
}

/// How [`UiHost::picker_feed`] merges a batch into the open picker's item
/// list: `picker-push!` (`Append`) vs `picker-replace!` (`Replace`).
pub enum PickerFeedMode {
    Append,
    Replace,
}

/// Cursor-anchored popup, selection menu, bottom drawer, minibuffer prompt
/// and picker, accessed through [`EditorHost::ui`](super::EditorHost::ui),
/// which is `None` for a host with no UI surface. Every opener opens over
/// the focused pane its `pane` names.
pub trait UiHost {
    /// `(prompt! pane label on-confirm #:prefill text)` opens a one-shot
    /// Command-mode minibuffer session. `callback` fires once, queued and
    /// never inline, with the confirmed text or `#f` on cancel. `Err` if a
    /// minibuffer session is already open.
    fn prompt(
        &mut self,
        pane: FocusedHandle,
        label: String,
        prefill: String,
        callback: crate::Callback,
    ) -> Result<(), String>;

    /// `(on-next-char! callback)` hands the next key typed to `callback`: its
    /// character, or `#f` when it isn't one or the wait ends any other way.
    /// Fires once, queued like [`Self::prompt`]'s callback. During `.` it
    /// answers with the next character the repeated command read, and errs
    /// when none is left.
    fn on_next_char(&mut self, callback: crate::Callback) -> Result<(), String>;

    /// `(show-popup! text #:anchor 'cursor #:kind 'sticky #:lang #f)` shows
    /// `text` in a popup, replacing any popup already showing. The host
    /// resolves geometry every frame; this stores the content. `lang` names
    /// a grammar to highlight `text` with; an unknown name or `None` renders
    /// plain text.
    ///
    /// Returns a token scoping [`Self::close_popup`] to this popup, or `None`
    /// when no layer can hold it right now.
    fn show_popup(
        &mut self,
        pane: FocusedHandle,
        text: String,
        kind: PopupKind,
        layout: PopupLayout,
        lang: Option<String>,
    ) -> Result<Option<HostToken>, String>;

    /// `(close-popup! token)` dismisses the popup `token` names. A no-op when
    /// no popup is showing or another `show-popup!` call opened the one that
    /// is. Only a host without popups errors.
    fn close_popup(&mut self, token: HostToken) -> Result<(), String>;

    /// `(show-menu! items on-select)` opens a selection menu near the cursor
    /// over `items`, `(display . payload)` rows with drawer-style row
    /// displays. `on-select` fires at most once, queued: the chosen row's
    /// payload, or `#f` on any dismissal but [`Self::close_menu`], which
    /// fires nothing.
    ///
    /// An async opener: the call is dropped, without an error, unless the
    /// mode layer is `Base` (a menu under Insert or a minibuffer mode could
    /// not route its keys) and the stack is otherwise unmoved since the
    /// request was fired. An open drawer or scrollable popup does not count
    /// as moved. A menu already open is replaced, its callback firing `#f`:
    /// a second response to one request is a refresh.
    ///
    /// Returns a token scoping [`Self::close_menu`] to this menu, or `None`
    /// when the call was dropped.
    fn show_menu(
        &mut self,
        pane: FocusedHandle,
        items: NonEmpty<(RowDisplay, crate::Rooted)>,
        callback: crate::Callback,
    ) -> Result<Option<HostToken>, String>;

    /// `(close-menu! token)` dismisses the menu `token` names without
    /// invoking its callback, even when another layer covers it. A no-op if
    /// none is open or the open one carries a different token.
    fn close_menu(&mut self, token: HostToken) -> Result<(), String>;

    /// `(show-drawer-list! items on-select #:selected [0] #:truncate ['tail]
    /// #:render [#f])` opens a scrolling pick-list in the bottom band,
    /// replacing any drawer already open. `items` are `(display . payload)`
    /// rows or render keys (see [`DrawerItems`]); the drawer never interprets
    /// a payload. `on-select` gets the chosen row's payload and the drawer's
    /// token, which tells an owner its own drawer's close from a replaced
    /// one's, and may fire more than once: the drawer stays open across
    /// `Enter` until `Esc` or `close-drawer!`. `selected` is clamped into
    /// `items`; a row wider than the drawer is cut at its `truncate` end.
    ///
    /// Returns the token [`Self::close_drawer`], [`Self::update_drawer_list`]
    /// and [`Self::drawer_selection`] take, or `None` when this async
    /// opener's request was dropped as stale.
    fn show_drawer_list(
        &mut self,
        pane: FocusedHandle,
        items: DrawerItems,
        callback: crate::Callback,
        selected: usize,
        truncate: TruncateEnd,
    ) -> Result<Option<HostToken>, String>;

    /// `(close-drawer! token)` dismisses the drawer without invoking its
    /// callback (`Esc` calls it with `#f`). A no-op if none is open or
    /// `token` is not the open drawer's. Closes a covered drawer too: it
    /// stays open under Insert or a popup while the user edits.
    fn close_drawer(&mut self, token: HostToken) -> Result<(), String>;

    /// `(update-drawer-list! token items on-select selected #:render [#f])`
    /// replaces the open drawer's rows, `on-select` and render proc in place,
    /// keeping its selection, scroll and `#:truncate` end. `selected` is
    /// clamped into the new list. Returns `false` when no drawer is open or
    /// `token` is not its own: the caller's drawer was closed or replaced,
    /// which its callback's `#f` already reported.
    fn update_drawer_list(
        &mut self,
        token: HostToken,
        items: DrawerItems,
        callback: crate::Callback,
        selected: usize,
    ) -> bool;

    /// `(drawer-selection token)`: the open drawer's selected row as its
    /// payload and index, or `None` when no drawer is open or `token` is not
    /// its own. Lets an owner map its selection across
    /// [`Self::update_drawer_list`] by what the row is, where it sits, or
    /// both.
    fn drawer_selection(&self, token: HostToken) -> Option<(SteelVal, usize)>;

    /// `(picker! pane items keymap #:prompt "…" #:pending [#f] #:query [""]
    /// #:truncate ['head] #:render [#f] #:on-close [#f])` opens the
    /// fuzzy-finder panel, filtered locally over `items` (a query that drives
    /// an external command is [`open_live_picker`](Self::open_live_picker)).
    /// `items` are `(display . payload)` pairs; Rust never interprets a
    /// payload. Returns the token later `picker-push!`/`picker-replace!`/
    /// `picker-source-spawn!`/`picker-move!`/`picker-close!` calls are scoped
    /// to, or `None` under `.` replay, which opens no picker. Allowed from
    /// any mode; closes a live completion session first, since only one
    /// modal owner may be active.
    ///
    /// `keymap` is every key the picker answers besides typing: a bound key
    /// runs its binding, queued, and nothing else, so the picker stays open
    /// until a binding calls `picker-close!`. An unbound plain character (no
    /// Ctrl or Alt) types into the query, an unbound `Backspace` deletes from
    /// it, and any other unbound key does nothing. `Err` when two specs name
    /// the same key.
    ///
    /// `on_close` fires once, queued, however the picker closes:
    /// `picker-close!`, a second `picker!`/`live-picker!` replacing it, or
    /// being removed under a layer that closes. `pending` shows a "results
    /// still arriving" indicator for a caller that opens empty and feeds
    /// through `spawn-async!` (`picker-source-spawn!` implies it); the first
    /// applied `push!`/`replace!` clears it.
    ///
    /// `render`, when given, shows each row. Before a frame shows rows not
    /// yet rendered at the panel's current width, the editor calls
    /// `(render rows budget query)`: `rows` are those rows' `(display .
    /// payload)` items in on-screen order, streamed rows included, `budget`
    /// the cells one row gets, and `query` the query the rows answer (for a
    /// live picker, the one whose search produced them). It returns one row
    /// display per row and may change the text; ranking, `.` and a binding's
    /// payload still use the item. Each row is rendered once per width and
    /// query, and a row still wider than `budget` is cut at the `#:truncate`
    /// end. `render` runs read-only; an error or a wrong result closes the
    /// picker, firing `on_close`.
    fn open_picker(
        &mut self,
        pane: FocusedHandle,
        items: Vec<(String, crate::Rooted)>,
        keymap: Vec<PickerBinding>,
        opts: PickerOpts,
    ) -> Result<Option<HostToken>, String>;

    /// `(live-picker! pane keymap #:source source #:prompt "…" #:query [""]
    /// #:debounce-ms [150] #:truncate ['head] #:render [#f] #:on-close [#f])`
    /// opens the fuzzy-finder panel with the query driving an external source
    /// instead of the local filter, so rows show unfiltered in item order.
    /// The session starts empty and is fed only by its `on_query_change`,
    /// through `picker-push!`/`picker-replace!`/`picker-source-spawn!`. Same
    /// return, keymap, `#:render`, `#:on-close` and modal-ownership contract
    /// as [`Self::open_picker`].
    ///
    /// Never fires `on_query_change` itself: the `live-picker!` wrapper runs
    /// the source for a non-empty `query` right after this returns, so a
    /// `#:source` that raises on it leaves the opened session in place.
    fn open_live_picker(
        &mut self,
        pane: FocusedHandle,
        keymap: Vec<PickerBinding>,
        opts: LivePickerOpts,
    ) -> Result<Option<HostToken>, String>;

    /// `(picker-push! token items)` / `(picker-replace! token items)` appends
    /// to, or replaces, the open picker's items and reranks, if `token`
    /// matches the open session. A stale token is a normal race for an async
    /// source, so it is a no-op, not an error; returns whether the feed
    /// applied. A live picker accepts only the token of its current query,
    /// so a late answer for an old query cannot overwrite the current rows
    /// or settle its pending state. `Replace` is a live source's requery: the
    /// previous rows stay on screen through the stop/debounce/respawn gap
    /// until the new search has rows to show or settles on none. Apart from
    /// `Replace`, items are append-only.
    fn picker_feed(
        &mut self,
        token: HostToken,
        items: Vec<(String, crate::Rooted)>,
        mode: PickerFeedMode,
    ) -> bool;

    /// `(picker-source-spawn! token cmd args #:cwd dir #:nul flag
    /// #:ok-exit-codes '(0))` attaches a streaming external-command source to
    /// the open picker (argv spawn, no shell); its stdout lines reach the
    /// item store without passing through Steel. Kills and replaces a source
    /// already attached to the session, which is how a live source re-runs
    /// per query. An outgoing source that had already exited still has its
    /// exit reported, so a superseded search does not hide a failure.
    ///
    /// `Ok(false)` for a stale token or no open picker, with nothing spawned.
    /// `Err` when the process fails to spawn (missing binary, bad `#:cwd`).
    fn picker_source_spawn(
        &mut self,
        token: HostToken,
        cmd: &str,
        args: Vec<String>,
        opts: PickerSourceOpts,
    ) -> Result<bool, String>;

    /// `(picker-source-stop! token)` stops the open picker's attached source,
    /// if any, leaving the items alone: for a live requery with nothing to
    /// spawn (an emptied pattern). Returns whether `token` matched the open
    /// session, whether or not a source was attached. Reports an already
    /// exited source's exit, as a re-spawn does.
    fn picker_source_stop(&mut self, token: HostToken) -> bool;

    /// `(picker-close! token)` ends the picker `token` names and fires its
    /// `#:on-close`, if any (`close-menu!`/`close-drawer!` drop their
    /// callback). A no-op if no picker is open or it carries another token.
    fn picker_close(&mut self, token: HostToken);

    /// `(picker-move! token count #:by ['row])` moves the selection of the
    /// picker `token` names by `count` steps of `step` (negative: up),
    /// stopping at the first and last item. A no-op under the same guard as
    /// [`Self::picker_close`].
    fn picker_move(&mut self, token: HostToken, count: isize, step: PickerStep);
}
