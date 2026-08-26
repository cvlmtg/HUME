//! Generic Steel-scriptable UI widget builtins.
//!
//! LSP is the first client of these widgets, not their owner — any plugin
//! can call `show-popup!`. `hume-editor`'s `EditorHostImpl` is the only
//! implementation that actually renders anything; other hosts (tests,
//! `MockHost`) have no `UiHost`, so `ctx.host.ui()` returns `None` and each
//! builtin surfaces `unsupported(...)` instead.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::host::{LivePickerOpts, PickerFeedMode, PickerOpts, PickerSourceOpts, PopupKind};

use super::SteelResult;
use super::args::{
    bool_arg, callable_arg, list_items, list_to_i32s, list_to_strings, optional_path_arg,
    optional_string_arg, optional_usize_arg, pair_fields, string_arg, usize_arg,
};
use super::errors::{generic_err, require_cap};

/// `(%show-popup! text anchor kind lang)` — the `show-popup!` Scheme wrapper
/// supplies `#:anchor`/`#:kind`/`#:lang`'s defaults. `anchor` selects the
/// render layout: `'cursor` floats near the focused pane's cursor (default);
/// `'bottom` docks as a full-width band above the statusline, reserving pane
/// space like the drawer. `kind` selects the dismiss behavior — see
/// [`PopupKind`].
pub(crate) fn show_popup(
    ctx: &mut SteelCtx,
    text: SteelVal,
    anchor: SteelVal,
    kind: SteelVal,
    lang: SteelVal,
) -> SteelResult {
    let text = string_arg(text, "show-popup! text")?;
    let anchor = string_arg(anchor, "show-popup! #:anchor")?;
    let docked = match anchor.as_str() {
        "cursor" => false,
        "bottom" => true,
        other => {
            steel::stop!(Generic => "show-popup!: #:anchor must be 'cursor or 'bottom, got '{}'", other)
        }
    };
    let kind = string_arg(kind, "show-popup! #:kind")?;
    let kind = match kind.as_str() {
        "sticky" => PopupKind::Sticky,
        "scrollable" => PopupKind::Scrollable,
        other => {
            steel::stop!(Generic => "show-popup!: #:kind must be 'sticky or 'scrollable, got '{}'", other)
        }
    };
    let lang = optional_string_arg(lang, "show-popup! #:lang")?;
    require_cap(ctx.host.ui(), "show-popup!")?
        .show_popup(text, kind, docked, lang)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(%close-popup!)`.
pub(crate) fn close_popup(ctx: &mut SteelCtx) -> SteelResult {
    require_cap(ctx.host.ui(), "close-popup!")?
        .close_popup()
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(show-menu! items on-select)` — no keyword defaults, so this registers
/// directly (no `%`-prefix wrapper needed).
pub(crate) fn show_menu(ctx: &mut SteelCtx, items: SteelVal, on_select: SteelVal) -> SteelResult {
    let items = list_to_strings(items, "show-menu! items")?;
    require_cap(ctx.host.ui(), "show-menu!")?
        .show_menu(items, on_select)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(close-menu!)`.
pub(crate) fn close_menu(ctx: &mut SteelCtx) -> SteelResult {
    require_cap(ctx.host.ui(), "close-menu!")?
        .close_menu()
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(show-drawer-list! items on-select)` — no keyword defaults, so this
/// registers directly (no `%`-prefix wrapper needed).
pub(crate) fn show_drawer_list(
    ctx: &mut SteelCtx,
    items: SteelVal,
    on_select: SteelVal,
) -> SteelResult {
    let items = list_to_strings(items, "show-drawer-list! items")?;
    require_cap(ctx.host.ui(), "show-drawer-list!")?
        .show_drawer_list(items, on_select)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(close-drawer!)`.
pub(crate) fn close_drawer(ctx: &mut SteelCtx) -> SteelResult {
    require_cap(ctx.host.ui(), "close-drawer!")?
        .close_drawer()
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(%prompt! label prefill on-confirm)` — the `prompt!` Scheme wrapper
/// supplies `#:prefill`'s default. `on-confirm` fires exactly once, later
/// (queued, never inline) — with the confirmed text, or `#f` on cancel.
pub(crate) fn prompt(
    ctx: &mut SteelCtx,
    label: SteelVal,
    prefill: SteelVal,
    on_confirm: SteelVal,
) -> SteelResult {
    let label = string_arg(label, "prompt! label")?;
    let prefill = string_arg(prefill, "prompt! prefill")?;
    require_cap(ctx.host.ui(), "prompt!")?
        .prompt(label, prefill, on_confirm)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// Decodes a picker `items` list: each entry must be a `(display . payload)`
/// dotted pair — a proper list entry is rejected by `pair_fields`.
/// `payload` stays an opaque `SteelVal`; Rust
/// never interprets it, except to reject `#f` — that value is reserved for
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

/// `(%picker! items on-select prompt pending query)` — the `picker!` Scheme
/// wrapper supplies the keyword defaults. Returns the new session's token.
pub(crate) fn picker(
    ctx: &mut SteelCtx,
    items: SteelVal,
    on_select: SteelVal,
    prompt: SteelVal,
    pending: SteelVal,
    query: SteelVal,
) -> SteelResult {
    let items = picker_items(items, "picker! items")?;
    let prompt = string_arg(prompt, "picker! #:prompt")?;
    let pending = bool_arg(pending, "picker! #:pending")?;
    let query = string_arg(query, "picker! #:query")?;
    let opts = PickerOpts {
        prompt,
        pending,
        query,
    };
    let token = require_cap(ctx.host.ui(), "picker!")?
        .open_picker(items, on_select, opts)
        .map_err(generic_err)?;
    Ok(SteelVal::IntV(token as isize))
}

/// `(%live-picker! on-select prompt query on-query-change)` — the
/// `live-picker!` Scheme wrapper supplies the keyword defaults and composes
/// `on-query-change` itself (stop-and-clear-then-debounce around the
/// caller's `#:command`); this layer only decodes it as a required
/// callable — checked here, unlike `on-select`: a bad `on-select` only ever
/// errors at accept/dismiss time, but a live session has no other use for
/// this argument, so a bad value is a definition-time mistake, not a
/// runtime one.
pub(crate) fn live_picker(
    ctx: &mut SteelCtx,
    on_select: SteelVal,
    prompt: SteelVal,
    query: SteelVal,
    on_query_change: SteelVal,
) -> SteelResult {
    let prompt = string_arg(prompt, "live-picker! #:prompt")?;
    let query = string_arg(query, "live-picker! #:query")?;
    let on_query_change = callable_arg(on_query_change, "live-picker! on-query-change")?;
    let opts = LivePickerOpts {
        prompt,
        query,
        on_query_change,
    };
    let token = require_cap(ctx.host.ui(), "live-picker!")?
        .open_live_picker(on_select, opts)
        .map_err(generic_err)?;
    Ok(SteelVal::IntV(token as isize))
}

/// `(picker-push! token items)` — no keyword defaults, so this registers
/// directly. Returns whether the push was applied (`#f` for a stale token
/// or no open picker — never an error, both are expected-normal races).
pub(crate) fn picker_push(ctx: &mut SteelCtx, token: SteelVal, items: SteelVal) -> SteelResult {
    let token = usize_arg(token, "picker-push! token")? as u64;
    let items = picker_items(items, "picker-push! items")?;
    let applied = require_cap(ctx.host.ui(), "picker-push!")?.picker_feed(
        token,
        items,
        PickerFeedMode::Append,
    );
    Ok(SteelVal::BoolV(applied))
}

/// `(picker-replace! token items)` — no keyword defaults, so this registers
/// directly, same shape as `picker-push!` but replacing the item list
/// instead of appending to it. Returns whether the replace was applied.
pub(crate) fn picker_replace(ctx: &mut SteelCtx, token: SteelVal, items: SteelVal) -> SteelResult {
    let token = usize_arg(token, "picker-replace! token")? as u64;
    let items = picker_items(items, "picker-replace! items")?;
    let applied = require_cap(ctx.host.ui(), "picker-replace!")?.picker_feed(
        token,
        items,
        PickerFeedMode::Replace,
    );
    Ok(SteelVal::BoolV(applied))
}

/// `(%picker-source-spawn! token cmd args cwd nul ok-exit-codes)` — the
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
    let token = usize_arg(token, "picker-source-spawn! token")? as u64;
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

/// `(picker-source-stop! token)` — no keyword defaults, so this registers
/// directly, same shape as `picker-push!`/`picker-replace!`. Returns
/// whether `token` matched the open session (the same expected-normal-race
/// contract as `picker-push!`), regardless of whether a source was actually
/// attached.
pub(crate) fn picker_source_stop(ctx: &mut SteelCtx, token: SteelVal) -> SteelResult {
    let token = usize_arg(token, "picker-source-stop! token")? as u64;
    let applied = require_cap(ctx.host.ui(), "picker-source-stop!")?.picker_source_stop(token);
    Ok(SteelVal::BoolV(applied))
}

/// `(%picker-close! token)` — the `picker-close!` Scheme wrapper supplies
/// `#:token`'s `#f` default.
pub(crate) fn picker_close(ctx: &mut SteelCtx, token: SteelVal) -> SteelResult {
    let token = optional_usize_arg(token, "picker-close! #:token")?.map(|t| t as u64);
    require_cap(ctx.host.ui(), "picker-close!")?.picker_close(token);
    Ok(SteelVal::Void)
}
