//! Generic Steel-scriptable UI widget builtins.
//!
//! LSP is the first client of these widgets, not their owner: any plugin
//! can call `show-popup!`. `hume-editor`'s `EditorHostImpl` is the only
//! implementation that actually renders anything; other hosts (tests,
//! `MockHost`) have no `UiHost`, so `ctx.host.ui()` returns `None` and each
//! builtin surfaces `unsupported(...)` instead.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;
use termina::event::KeyEvent;

use hume_engine::types::TruncateEnd;

use crate::SteelCtx;
use crate::host::{
    DrawerItems, LivePickerOpts, PickerFeedMode, PickerOpts, PickerSourceOpts, PopupKind,
};
use crate::types::PaneHandle;

use super::SteelResult;
use super::args::{
    bool_arg, callable_arg, list_items, list_to_i32s, list_to_strings, optional_callable_arg,
    optional_path_arg, optional_string_arg, pair_fields, single_key_arg, string_arg,
    symbol_enum_arg, token_arg, token_or_false, usize_arg,
};
use super::errors::{generic_err, require_cap};

/// `(%show-popup! pane text anchor kind lang)`: the `show-popup!` Scheme
/// wrapper supplies `#:anchor`/`#:kind`/`#:lang`'s defaults. Returns a token
/// for `close-popup!`, or `#f` when the popup was dropped. `anchor` selects the
/// render layout: `'cursor` floats near the focused pane's cursor (default);
/// `'bottom` docks as a full-width band above the statusline, reserving pane
/// space like the drawer. `kind` selects the dismiss behavior; see
/// [`PopupKind`].
pub(crate) fn show_popup(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    text: SteelVal,
    anchor: SteelVal,
    kind: SteelVal,
    lang: SteelVal,
) -> SteelResult {
    let text = string_arg(text, "show-popup! text")?;
    let anchor = string_arg(anchor, "show-popup! #:anchor")?;
    let docked = symbol_enum_arg(
        &anchor,
        "show-popup! #:anchor",
        &[("cursor", false), ("bottom", true)],
    )?;
    let kind = string_arg(kind, "show-popup! #:kind")?;
    let kind = symbol_enum_arg(
        &kind,
        "show-popup! #:kind",
        &[
            ("sticky", PopupKind::Sticky),
            ("scrollable", PopupKind::Scrollable),
        ],
    )?;
    let lang = optional_string_arg(lang, "show-popup! #:lang")?;
    let token = require_cap(ctx.host.ui(), "show-popup!")?
        .show_popup(pane, text, kind, docked, lang)
        .map_err(generic_err)?;
    Ok(token_or_false(token))
}

/// `(close-popup! token)`. A `#f` token (the opener returned `#f`, or the
/// popup has since closed) is a no-op, same as any other stale token.
pub(crate) fn close_popup(ctx: &mut SteelCtx, token: SteelVal) -> SteelResult {
    let token = token_arg(token, "close-popup! token")?;
    require_cap(ctx.host.ui(), "close-popup!")?
        .close_popup(token)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(show-menu! pane items on-select)`: no keyword defaults, so this
/// registers directly (no `%`-prefix wrapper needed). Returns a token for
/// `close-menu!`, or `#f` when the call was dropped as stale.
pub(crate) fn show_menu(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    items: SteelVal,
    on_select: SteelVal,
) -> SteelResult {
    let items = list_to_strings(items, "show-menu! items")?;
    let token = require_cap(ctx.host.ui(), "show-menu!")?
        .show_menu(pane, items, on_select)
        .map_err(generic_err)?;
    Ok(token_or_false(token))
}

/// `(close-menu! token)`. A `#f` token is a no-op, same as `close-popup!`.
pub(crate) fn close_menu(ctx: &mut SteelCtx, token: SteelVal) -> SteelResult {
    let token = token_arg(token, "close-menu! token")?;
    require_cap(ctx.host.ui(), "close-menu!")?
        .close_menu(token)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(%show-drawer-list! pane items on-select selected render)`, behind
/// `show-drawer-list!`'s `#:selected`/`#:render` keyword wrapper (see
/// [`drawer_items`] for `render`). `selected` is clamped into `items`. Errors on empty `items`; callers close (or never open)
/// instead. Returns a token scoping
/// `close-drawer!`/`update-drawer-list!`/`drawer-selected-index` to this
/// drawer, same shape as `picker!`'s own return, or `#f` when the request
/// was dropped as stale (the stack moved before it could open; see
/// `UiHost::show_drawer_list`'s own doc), which callers must branch on
/// rather than treat as a live drawer's token.
pub(crate) fn show_drawer_list(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    items: SteelVal,
    on_select: SteelVal,
    selected: SteelVal,
    render: SteelVal,
) -> SteelResult {
    let items = drawer_items(items, render, "show-drawer-list!")?;
    let selected = usize_arg(selected, "show-drawer-list! selected")?;
    let token = require_cap(ctx.host.ui(), "show-drawer-list!")?
        .show_drawer_list(pane, items, on_select, selected)
        .map_err(generic_err)?;
    Ok(token_or_false(token))
}

/// `(close-drawer! token)`. A `#f` token is a no-op, same as `close-popup!`.
pub(crate) fn close_drawer(ctx: &mut SteelCtx, token: SteelVal) -> SteelResult {
    let token = token_arg(token, "close-drawer! token")?;
    require_cap(ctx.host.ui(), "close-drawer!")?
        .close_drawer(token)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(%update-drawer-list! token items on-select selected render)`, behind
/// `update-drawer-list!`'s `#:render` keyword wrapper (see [`drawer_items`]
/// for `render`). Replaces the open drawer's rows in
/// place, keeping the browse session; `selected` is clamped into the new
/// list. Returns whether the update applied: `#f` when no drawer is open
/// or `token` doesn't match the open drawer's own (an expected-normal race,
/// never an error, same contract as `picker-replace!`'s stale token) or when
/// `items` is empty (callers close instead).
pub(crate) fn update_drawer_list(
    ctx: &mut SteelCtx,
    token: SteelVal,
    items: SteelVal,
    on_select: SteelVal,
    selected: SteelVal,
    render: SteelVal,
) -> SteelResult {
    let token = token_arg(token, "update-drawer-list! token")?;
    let items = drawer_items(items, render, "update-drawer-list!")?;
    let selected = usize_arg(selected, "update-drawer-list! selected")?;
    let applied = require_cap(ctx.host.ui(), "update-drawer-list!")?
        .update_drawer_list(token, items, on_select, selected);
    Ok(SteelVal::BoolV(applied))
}

/// A drawer's `items`: strings when `render` is `#f`, otherwise opaque keys
/// rendered on demand by the `render` proc.
fn drawer_items(items: SteelVal, render: SteelVal, builtin: &str) -> Result<DrawerItems, SteelErr> {
    let items_name = format!("{builtin} items");
    Ok(
        match optional_callable_arg(render, &format!("{builtin} render"))? {
            None => DrawerItems::Rows(list_to_strings(items, &items_name)?),
            Some(render) => DrawerItems::Keys {
                keys: list_items(items, &items_name)?,
                render,
            },
        },
    )
}

/// `(drawer-selected-index token)`: the open drawer's selected row, or
/// `#f` when no drawer is open, `token` doesn't match its own, or `token`
/// itself is `#f` (the opener that produced it returned `#f`).
pub(crate) fn drawer_selected_index(ctx: &mut SteelCtx, token: SteelVal) -> SteelResult {
    let token = token_arg(token, "drawer-selected-index token")?;
    match require_cap(ctx.host.ui(), "drawer-selected-index")?.drawer_selected_index(token) {
        Some(idx) => Ok(SteelVal::IntV(idx as isize)),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(%prompt! label prefill on-confirm)`: the `prompt!` Scheme wrapper
/// supplies `#:prefill`'s default. `on-confirm` fires exactly once, later
/// (queued, never inline), with the confirmed text, or `#f` on cancel.
pub(crate) fn prompt(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    label: SteelVal,
    prefill: SteelVal,
    on_confirm: SteelVal,
) -> SteelResult {
    let label = string_arg(label, "prompt! label")?;
    let prefill = string_arg(prefill, "prompt! prefill")?;
    require_cap(ctx.host.ui(), "prompt!")?
        .prompt(pane, label, prefill, on_confirm)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// Decodes a picker `items` list: each entry must be a `(display . payload)`
/// dotted pair; a proper list entry is rejected by `pair_fields`.
/// `payload` stays an opaque `SteelVal`; Rust
/// never interprets it, except to reject `#f`: that value is reserved for
/// the dismiss signal (`on-select` receives it on Esc / `picker-close!` /
/// replace), so a `#f` payload would make an accepted row indistinguishable
/// from a dismissal.
fn picker_items(items: SteelVal, ctx_name: &str) -> Result<Vec<(String, SteelVal)>, SteelErr> {
    list_items(items, ctx_name)?
        .into_iter()
        .map(|entry| {
            let (display, payload) = pair_fields(entry, ctx_name, "(display . payload)")?;
            // absent-decode-safe: #f here is the reserved dismiss sentinel being rejected, not an absent optional
            if matches!(payload, SteelVal::BoolV(false)) {
                steel::stop!(Generic => "{}: item payload must not be #f (#f is reserved for the dismiss signal)", ctx_name);
            }
            Ok((string_arg(display, ctx_name)?, payload))
        })
        .collect()
}

/// Decodes `picker!`'s/`live-picker!`'s `#:actions` alist: each entry is a
/// `(key-spec . proc)` dotted pair; a proper list entry is rejected by
/// `pair_fields`, same as `picker_items`. `key-spec` must parse (via
/// [`single_key_arg`]) to exactly one `KeyEvent`. The picker dispatches
/// one chord at a time, so a multi-key spec like `"z f"` silently binding
/// only its first key would be a trap rather than a useful feature.
fn picker_actions(
    actions: SteelVal,
    ctx_name: &str,
) -> Result<Vec<(KeyEvent, SteelVal)>, SteelErr> {
    list_items(actions, ctx_name)?
        .into_iter()
        .map(|entry| {
            let (spec, proc) = pair_fields(entry, ctx_name, "(key-spec . proc)")?;
            let key = single_key_arg(spec, ctx_name)?;
            let proc = callable_arg(proc, ctx_name)?;
            Ok((key, proc))
        })
        .collect()
}

/// Decodes `picker!`'s/`live-picker!`'s `#:truncate` symbol: `'head`
/// (default, drop the front) or `'tail` (drop the back). Shared so the two
/// builtins can't drift on the accepted spelling or the error message.
fn truncate_end_arg(val: SteelVal, ctx_name: &str) -> Result<TruncateEnd, SteelErr> {
    symbol_enum_arg(
        string_arg(val, ctx_name)?.as_str(),
        ctx_name,
        &[("head", TruncateEnd::Head), ("tail", TruncateEnd::Tail)],
    )
}

/// `(%picker! items on-select prompt pending query truncate actions)`: the
/// `picker!` Scheme wrapper supplies the keyword defaults. Returns the new
/// session's token.
// Each param is a positional/keyword arg the `builtins!` table maps 1:1 from
// `picker!`'s own Steel signature. Bundling them into a struct would break
// that direct correspondence for no benefit, since every arg is already
// decoded and validated independently right below.
#[allow(clippy::too_many_arguments)]
pub(crate) fn picker(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    items: SteelVal,
    on_select: SteelVal,
    prompt: SteelVal,
    pending: SteelVal,
    query: SteelVal,
    truncate: SteelVal,
    actions: SteelVal,
) -> SteelResult {
    let items = picker_items(items, "picker! items")?;
    let prompt = string_arg(prompt, "picker! #:prompt")?;
    let pending = bool_arg(pending, "picker! #:pending")?;
    let query = string_arg(query, "picker! #:query")?;
    let truncate = truncate_end_arg(truncate, "picker! #:truncate")?;
    let actions = picker_actions(actions, "picker! #:actions")?;
    let opts = PickerOpts {
        prompt,
        pending,
        query,
        truncate,
        actions,
    };
    let token = require_cap(ctx.host.ui(), "picker!")?
        .open_picker(pane, items, on_select, opts)
        .map_err(generic_err)?;
    Ok(token.to_steel())
}

/// `(%live-picker! on-select prompt query on-query-change truncate actions)`: the
/// `live-picker!` Scheme wrapper supplies the keyword defaults and composes
/// `on-query-change` itself (stop-and-clear-then-debounce around the
/// caller's `#:command`); this layer only decodes it as a required
/// callable, checked here unlike `on-select`: a bad `on-select` only ever
/// errors at accept/dismiss time, but a live session has no other use for
/// this argument, so a bad value is a definition-time mistake, not a
/// runtime one.
// Same shape as `picker`'s own allow, just above: each param is a
// positional/keyword arg the `builtins!` table maps 1:1 from `live-picker!`'s
// own Steel signature. Bundling them into a struct would break that direct
// correspondence for no benefit, since every arg is already decoded and
// validated independently right below.
#[allow(clippy::too_many_arguments)]
pub(crate) fn live_picker(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    on_select: SteelVal,
    prompt: SteelVal,
    query: SteelVal,
    on_query_change: SteelVal,
    truncate: SteelVal,
    actions: SteelVal,
) -> SteelResult {
    let prompt = string_arg(prompt, "live-picker! #:prompt")?;
    let query = string_arg(query, "live-picker! #:query")?;
    let on_query_change = callable_arg(on_query_change, "live-picker! on-query-change")?;
    let truncate = truncate_end_arg(truncate, "live-picker! #:truncate")?;
    let actions = picker_actions(actions, "live-picker! #:actions")?;
    let opts = LivePickerOpts {
        prompt,
        query,
        on_query_change,
        truncate,
        actions,
    };
    let token = require_cap(ctx.host.ui(), "live-picker!")?
        .open_live_picker(pane, on_select, opts)
        .map_err(generic_err)?;
    Ok(token.to_steel())
}

/// Shared body of `picker-push!`/`picker-replace!`: identical shape, differing
/// only in `name` (for error messages) and `mode` (the merge policy
/// `UiHost::picker_feed` applies; see its doc, `"One method for both"`).
fn picker_feed_builtin(
    ctx: &mut SteelCtx,
    token: SteelVal,
    items: SteelVal,
    name: &str,
    mode: PickerFeedMode,
) -> SteelResult {
    let token = token_arg(token, &format!("{name} token"))?;
    let items = picker_items(items, &format!("{name} items"))?;
    let applied = require_cap(ctx.host.ui(), name)?.picker_feed(token, items, mode);
    Ok(SteelVal::BoolV(applied))
}

/// `(picker-push! token items)`: no keyword defaults, so this registers
/// directly. Returns whether the push was applied (`#f` for a stale token
/// or no open picker; never an error, both are expected-normal races).
pub(crate) fn picker_push(ctx: &mut SteelCtx, token: SteelVal, items: SteelVal) -> SteelResult {
    picker_feed_builtin(ctx, token, items, "picker-push!", PickerFeedMode::Append)
}

/// `(picker-replace! token items)`: no keyword defaults, so this registers
/// directly, same shape as `picker-push!` but replacing the item list
/// instead of appending to it. Returns whether the replace was applied.
pub(crate) fn picker_replace(ctx: &mut SteelCtx, token: SteelVal, items: SteelVal) -> SteelResult {
    picker_feed_builtin(
        ctx,
        token,
        items,
        "picker-replace!",
        PickerFeedMode::Replace,
    )
}

/// `(%picker-source-spawn! token cmd args cwd nul ok-exit-codes)`: the
/// `picker-source-spawn!` Scheme wrapper supplies
/// `#:cwd`/`#:nul`/`#:ok-exit-codes`'s defaults. A stale token or no open
/// picker returns `#f` without spawning anything, the same
/// expected-normal-race contract as `picker-push!`; a genuine spawn failure
/// (missing binary, bad `#:cwd`) raises.
pub(crate) fn picker_source_spawn(
    ctx: &mut SteelCtx,
    token: SteelVal,
    cmd: SteelVal,
    args: SteelVal,
    cwd: SteelVal,
    nul: SteelVal,
    ok_exit_codes: SteelVal,
) -> SteelResult {
    let token = token_arg(token, "picker-source-spawn! token")?;
    let cmd = string_arg(cmd, "picker-source-spawn! cmd")?;
    if cmd.trim().is_empty() {
        steel::stop!(Generic => "picker-source-spawn!: cmd must not be empty");
    }
    let args = list_to_strings(args, "picker-source-spawn! args")?;
    let cwd = optional_path_arg(cwd, "picker-source-spawn! #:cwd")?;
    let nul = bool_arg(nul, "picker-source-spawn! #:nul")?;
    let ok_exit_codes = list_to_i32s(ok_exit_codes, "picker-source-spawn! #:ok-exit-codes")?;
    let opts = PickerSourceOpts {
        cwd,
        nul,
        ok_exit_codes,
    };

    let applied = require_cap(ctx.host.ui(), "picker-source-spawn!")?
        .picker_source_spawn(token, &cmd, args, opts)
        .map_err(|e| generic_err(format!("picker-source-spawn!: {e}")))?;
    Ok(SteelVal::BoolV(applied))
}

/// `(picker-source-stop! token)`: no keyword defaults, so this registers
/// directly, same shape as `picker-push!`/`picker-replace!`. Returns
/// whether `token` matched the open session (the same expected-normal-race
/// contract as `picker-push!`), regardless of whether a source was actually
/// attached.
pub(crate) fn picker_source_stop(ctx: &mut SteelCtx, token: SteelVal) -> SteelResult {
    let token = token_arg(token, "picker-source-stop! token")?;
    let applied = require_cap(ctx.host.ui(), "picker-source-stop!")?.picker_source_stop(token);
    Ok(SteelVal::BoolV(applied))
}

/// `(picker-close! token)`: no keyword defaults, so this registers
/// directly, same shape as `picker-source-stop!`. A `#f` token is a no-op,
/// same as `close-popup!`.
pub(crate) fn picker_close(ctx: &mut SteelCtx, token: SteelVal) -> SteelResult {
    let token = token_arg(token, "picker-close! token")?;
    require_cap(ctx.host.ui(), "picker-close!")?.picker_close(token);
    Ok(SteelVal::Void)
}
