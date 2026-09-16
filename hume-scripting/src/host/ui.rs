//! Cursor-anchored popup, selection menu, bottom drawer, minibuffer
//! prompt, and the fuzzy-finder picker.

use termina::event::KeyEvent;

use hume_engine::types::TruncateEnd;

/// How an open popup reacts to key and mouse input — `show-popup!`'s
/// `#:kind` symbol, decoded once at the builtin boundary
/// (`builtins::ui::show_popup`) and carried as-is into the editor's own
/// popup state, so there is exactly one definition of the two dismiss
/// behaviors, not a bool pair mapped to a second enum on the other side of
/// the trait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupKind {
    /// Untouched by keys and mouse input alike; lives in the current editing
    /// mode's own slot (`Base`/`Insert` only — anything else has none, so
    /// `#:kind 'sticky` from `:`-typing or with a menu/drawer/picker open is
    /// a no-op), and closes when that mode ends, via `close-popup!`, or on
    /// the next `show-popup!`. Default — `#:kind` omitted, or `'sticky`.
    Sticky,
    /// Ctrl-u/Ctrl-d scroll the content and are consumed *when it overflows
    /// one screenful*; every other key or mouse event — and Ctrl-u/d with
    /// nothing to scroll — closes the popup and falls through to normal
    /// dispatch (`#:kind 'scrollable`). Covers both scrollable hover and the
    /// dismiss-on-any-key `gn`/`gp` diagnostic overlay: the two collapse to
    /// the same behavior once content fits on screen, and a long diagnostic
    /// gets scrolling for free instead of a hard height cap.
    Scrollable,
}

/// Grouped `picker!` open-time keyword options. [`UiHost::open_picker`] takes
/// the Scheme call's positional arguments (`items`, `on_select`) directly;
/// every `#:`-prefixed one rides here instead — the same split
/// [`PickerSourceOpts`] uses for `picker_source_spawn`. `Default` is every
/// keyword's own default (empty prompt, not pending, empty query) — the
/// shape a test that doesn't care about any of them wants.
#[derive(Default)]
pub struct PickerOpts {
    /// `#:prompt` — label painted before the query in the input line.
    pub prompt: String,
    /// `#:pending` — see [`UiHost::open_picker`]'s doc.
    pub pending: bool,
    /// `#:query` — the query the picker opens with, applied (as a fuzzy
    /// filter) to the item list at construction.
    pub query: String,
    /// `#:truncate` — see [`TruncateEnd`].
    pub truncate: TruncateEnd,
    /// `#:actions` — extra key→proc bindings tried, in order, after every
    /// built-in picker key (movement, `Backspace`, `Enter`, `Escape`, query
    /// input) — see [`UiHost::open_picker`]'s doc. Empty by default: a
    /// picker whose payload isn't a placeable buffer target simply omits
    /// `#:actions` rather than opting out of a flag.
    pub actions: Vec<(KeyEvent, steel::rvals::SteelVal)>,
}

/// Grouped `live-picker!` open-time keyword options — the live counterpart
/// of [`PickerOpts`]. No `Default`: `on_query_change` is always
/// `live-picker!`'s own internal requery lambda (never a caller-supplied
/// value directly), so a default that silently opened a non-live session
/// would be a footgun, not a convenience.
pub struct LivePickerOpts {
    /// `#:prompt` — as [`PickerOpts::prompt`].
    pub prompt: String,
    /// `#:query` — the query the picker opens with. Unlike `PickerOpts`'s
    /// (which only filters the already-known item list), a non-empty value
    /// here also has the `live-picker!` Scheme wrapper spawn once,
    /// undebounced, right after this call returns — see
    /// [`UiHost::open_live_picker`]'s doc. `open_live_picker` itself never
    /// fires `on_query_change` for it; that lambda is reserved for
    /// per-keystroke requeries.
    pub query: String,
    /// Fired with `(token query)` on every query-changing keystroke instead
    /// of driving a local fuzzy filter — see
    /// `PickerSession::rebuild_filtered`'s doc for why a live session skips
    /// it. Always `live-picker!`'s own stop-and-clear-then-debounce
    /// wrapper around the caller's `#:command` builder, never the builder
    /// itself.
    pub on_query_change: steel::rvals::SteelVal,
    /// `#:truncate` — see [`TruncateEnd`].
    pub truncate: TruncateEnd,
    /// `#:actions` — see [`PickerOpts::actions`].
    pub actions: Vec<(KeyEvent, steel::rvals::SteelVal)>,
}

/// Grouped `picker-source-spawn!` keyword options — the same split
/// [`PickerOpts`] uses for `open_picker`: [`UiHost::picker_source_spawn`]'s
/// positional arguments (`token`, `cmd`, `args`) stay directly on the trait
/// method. No `Default`: `#:ok-exit-codes` defaults to `'(0)`, not
/// `Vec::default()`'s empty list, so a caller must always supply it
/// explicitly rather than get a silently wrong allowlist.
pub struct PickerSourceOpts {
    /// `#:cwd` — working directory for the spawned process; `None` inherits
    /// the caller's.
    pub cwd: Option<std::path::PathBuf>,
    /// `#:nul` — split stdout on NUL bytes instead of newlines.
    pub nul: bool,
    /// `#:ok-exit-codes` — the complete set of exit codes that count as a
    /// normal outcome. It *replaces* the success check rather than
    /// extending it — nothing is implied, `0` included — so a caller
    /// overriding the `'(0)` default lists `0` alongside whatever it adds:
    /// `'(0 1)` for `rg`, which exits `1` on "no matches". `'(1)` alone
    /// would report every successful run as a failure. Explicit over
    /// convenient: the list is the whole contract.
    pub ok_exit_codes: Vec<i32>,
}

/// How [`UiHost::picker_feed`] merges a batch into the open picker's item
/// list — `picker-push!` (`Append`) vs `picker-replace!` (`Replace`).
pub enum PickerFeedMode {
    Append,
    Replace,
}

/// Cursor-anchored popup, selection menu, bottom drawer, and minibuffer
/// prompt — accessed through [`EditorHost::ui`](super::EditorHost::ui). `None` from that accessor
/// means "no UI surface to drive" (test stubs); every method here is
/// required once a host does provide `UiHost`.
pub trait UiHost {
    /// `(prompt! label #:prefill text on-confirm)` — opens a one-shot
    /// Command-mode minibuffer session. `callback` fires exactly once, with
    /// the confirmed text or `#f` on cancel — queued through the same
    /// drained-at-frame-boundary path as every other Rust→Steel call, never
    /// invoked inline. Errors if a minibuffer session is already open.
    fn prompt(
        &mut self,
        label: String,
        prefill: String,
        callback: steel::rvals::SteelVal,
    ) -> Result<(), String>;

    /// `(show-popup! text #:anchor 'cursor #:kind 'sticky #:lang #f)` — shows
    /// `text` in a popup panel. Geometry (wrap width, flip/clamp position, or
    /// the docked band's size) is resolved fresh every frame by the host, not
    /// here — this just stores the raw content. Replaces any popup already
    /// showing (no stacking).
    ///
    /// `kind`: see [`PopupKind`] for the two dismiss behaviors. `docked`:
    /// `#:anchor 'bottom` — renders as a full-width chrome band directly
    /// above the statusline (reserving pane space, like the drawer) instead
    /// of floating near the cursor. `lang`: when `Some(name)` and a grammar
    /// named `name` is registered, `text` is syntax-highlighted like a real
    /// buffer; otherwise (no grammar by that name, or `None`) it renders as
    /// plain text.
    fn show_popup(
        &mut self,
        text: String,
        kind: PopupKind,
        docked: bool,
        lang: Option<String>,
    ) -> Result<(), String>;

    /// `(close-popup!)` — dismisses the popup. Idempotent: closing when none
    /// is showing is not an error (only an unsupported *host* errors).
    fn close_popup(&mut self) -> Result<(), String>;

    /// `(show-menu! items on-select)` — opens a selection menu near the
    /// cursor. `on-select` fires exactly once: the chosen index, or `#f` on
    /// dismissal — queued, never invoked inline. Replaces any menu already
    /// open (no stacking). Hosts should reject this from Insert mode — a
    /// menu that can't be driven is worse than no menu (note: a command
    /// triggered via `:name` still runs with the *previous* mode active, so
    /// this must be an Insert-specific rejection, not a Normal/Extend-only
    /// allowlist).
    fn show_menu(
        &mut self,
        items: Vec<String>,
        callback: steel::rvals::SteelVal,
    ) -> Result<(), String>;

    /// `(close-menu!)` — dismisses the menu *without* invoking its callback
    /// (caller-initiated close, distinct from the key-driven dismissal paths
    /// which do call back with `#f`).
    fn close_menu(&mut self) -> Result<(), String>;

    /// `(show-drawer-list! items on-select)` — opens a scrolling pick-list
    /// in the bottom chrome band. `items` are pre-formatted display strings;
    /// the drawer never interprets their content — the jump (if any) is the
    /// caller's job, typically `(goto-location! ...)` inside `on-select`.
    /// `on-select` receives the chosen index and, unlike the popup/menu's
    /// one-shot callback, may fire more than once: the drawer stays open
    /// across `Enter` (Helix-style browse) until `Esc` or `close-drawer!`.
    /// Replaces any drawer already open (no stacking).
    fn show_drawer_list(
        &mut self,
        items: Vec<String>,
        callback: steel::rvals::SteelVal,
    ) -> Result<(), String>;

    /// `(close-drawer!)` — dismisses the drawer *without* invoking its
    /// callback (caller-initiated close, distinct from `Esc`, which does
    /// call back with `#f`).
    fn close_drawer(&mut self) -> Result<(), String>;

    /// `(picker! items on-select #:prompt "…" #:pending [#f] #:query [""])`
    /// — opens the fuzzy-finder panel, always fuzzy-filtered over `items`
    /// (query-change never leaves the local filter — for a source whose
    /// query drives an external command instead, see
    /// [`open_live_picker`](Self::open_live_picker)). `items` are
    /// `(display . payload)` pairs; `payload` is handed back to `on-select`
    /// verbatim, never interpreted by Rust. Returns a token that scopes
    /// later `picker-push!`/`picker-replace!`/`picker-source-spawn!` calls
    /// to this session. Unlike the menu/drawer, the picker is allowed from
    /// any mode but closes any live completion session first, since only
    /// one modal owner may be active at a time. `on-select` fires exactly
    /// once, queued (never invoked inline): the selected payload on
    /// `Enter`, or `#f` on `Esc`, `picker-close!`, or being replaced by a
    /// second `picker!`/`live-picker!` call. `pending`: set when a caller
    /// opens empty and expects more results via `spawn-async!` rather than
    /// `picker-source-spawn!` (which already implies "still populating" on
    /// its own) — surfaced to the UI as a "results still arriving"
    /// indicator, cleared by the first `push!`/`replace!` that actually
    /// applies. `query`, `actions`: see [`PickerOpts`]. An `actions` entry is
    /// tried only after every built-in picker key (movement, `Backspace`,
    /// `Enter`, `Escape`, query input) — it can never override one of those,
    /// so `#:actions` is purely additive. A matching entry fires *instead
    /// of* `on-select` for that keystroke — not in addition to it — with the
    /// same selected-payload argument and exactly-once, queued contract.
    fn open_picker(
        &mut self,
        items: Vec<(String, steel::rvals::SteelVal)>,
        on_select: steel::rvals::SteelVal,
        opts: PickerOpts,
    ) -> Result<u64, String>;

    /// `(live-picker! on-select #:command command #:prompt "…" #:query [""]
    /// #:debounce-ms [150] #:cwd [#f] #:nul [#f] #:ok-exit-codes ['(0)])` —
    /// opens the fuzzy-finder panel with the query driving an external
    /// source instead of the local fuzzy filter: `filtered` is always the
    /// identity permutation over whatever `items` currently holds (see
    /// `PickerSession::rebuild_filtered`'s doc). No `items`/`pending`
    /// parameter — a live session starts empty and is populated entirely by
    /// its own `on_query_change` callback (via `picker-push!`/
    /// `picker-replace!`/`picker-source-spawn!`, exactly as a `picker!`
    /// session's async sources are). Same return, exactly-once `on-select`,
    /// and modal-ownership contract as `open_picker`. `opts`: see
    /// [`LivePickerOpts`] — this method itself never fires
    /// `on_query_change`; the `live-picker!` Scheme wrapper spawns for a
    /// non-empty seed `query` itself, undebounced, right after this
    /// returns, so a bad `#:command` raise on that seed leaves the session
    /// this call already opened in place (Esc still closes it) rather than
    /// tearing it down — the wrapper deliberately doesn't catch-and-reraise
    /// around it, since a call sourced from a native builtin re-raised
    /// through a nested Steel handler corrupts the VM's continuation stack.
    fn open_live_picker(
        &mut self,
        on_select: steel::rvals::SteelVal,
        opts: LivePickerOpts,
    ) -> Result<u64, String>;

    /// `(picker-push! token items)` / `(picker-replace! token items)` —
    /// appends to, or wholesale replaces, the open picker's item list and
    /// reranks, but only if `token` matches the session the caller opened
    /// (returned by `open_picker`). One method for both: the token guard
    /// and "no open picker"/stale-token no-op contract are identical, only
    /// the merge policy ([`PickerFeedMode`]) differs. A stale token — the
    /// picker was closed or replaced since — is expected-normal for an
    /// async source racing the user, so it is a silent no-op, not an error:
    /// returns whether the feed was applied. `Replace` is the requery half
    /// of a live source: the previous pattern's rows stay on screen through
    /// the requery's stop/debounce/respawn gap and are only dropped once the
    /// new search has something to show in their place (or settles on
    /// nothing) — items are otherwise append-only.
    fn picker_feed(
        &mut self,
        token: u64,
        items: Vec<(String, steel::rvals::SteelVal)>,
        mode: PickerFeedMode,
    ) -> bool;

    /// `(picker-source-spawn! token cmd args #:cwd dir #:nul flag
    /// #:ok-exit-codes '(0))` — attaches a streaming external-command
    /// source to the open picker (direct argv spawn, no shell). Its stdout
    /// lines flow directly into the store, never through Steel. Replaces
    /// (killing) any source already attached to the same session — a
    /// second spawn is a re-spawn, not a second concurrent source, which is
    /// also how a live source re-runs per query change. If the outgoing
    /// source had already exited, its exit is reported exactly as it would
    /// have been had it disconnected on its own — a re-spawn must not
    /// silence a genuine failure just because a newer search superseded it
    /// before the drain got to it. `opts`: see [`PickerSourceOpts`].
    ///
    /// `Ok(false)` — same "expected-normal race, not an error" contract as
    /// `picker_feed` — means a stale token or no open picker; nothing was
    /// spawned. `Err` means the process itself failed to spawn (missing
    /// binary, bad `#:cwd`).
    fn picker_source_spawn(
        &mut self,
        token: u64,
        cmd: &str,
        args: Vec<String>,
        opts: PickerSourceOpts,
    ) -> Result<bool, String>;

    /// `(picker-source-stop! token)` — stops the open picker's attached
    /// streaming source, if any, without touching the item list.
    /// `picker-replace!` can clear stale rows but has no way to silence the
    /// search that produced them; this is that missing half, for a live
    /// requery whose new query has nothing to spawn a replacement source
    /// for (e.g. backspacing to an empty pattern). Same
    /// expected-normal-race contract as `picker_feed`: returns whether
    /// `token` matched the open session, regardless of whether a source was
    /// actually attached. Reports the outgoing source's exit if it had
    /// already exited, same as a re-spawn via `picker_source_spawn`.
    fn picker_source_stop(&mut self, token: u64) -> bool;

    /// `(picker-close! #:token [token #f])` — ends the open picker, if any,
    /// firing its `on-select` with `#f` (unlike `close-menu!`/
    /// `close-drawer!`, which drop the callback without invoking it — the
    /// picker's callback lifecycle guarantees exactly one fire per session
    /// no matter how it ends). `token` scopes the close to a specific
    /// session the same way `picker-push!`'s does: `Some(t)` is a no-op if
    /// the open picker's token doesn't match `t` (someone else's session
    /// has since taken over) — the async-callback case `picker-push!`
    /// already guards against. `#f`/omitted closes whatever picker is open,
    /// unconditionally, for the synchronous "the user hit Esc" caller that
    /// has no token to check against. Idempotent either way: closing when
    /// none is open is not an error.
    fn picker_close(&mut self, token: Option<u64>);
}
