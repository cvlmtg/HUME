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

use super::args::{
    bool_arg, list_items, list_to_strings, optional_path_arg, pair_fields, string_arg, usize_arg,
};
use super::errors::{generic_err, require_cap};

type SteelResult = Result<SteelVal, SteelErr>;

/// `(%show-popup! text anchor dismiss-on-key)` — the `show-popup!` Scheme
/// wrapper supplies `#:anchor`/`#:dismiss-on-key`'s defaults. `'cursor` is
/// the only anchor accepted in v1.
pub(crate) fn show_popup(
    ctx: &mut SteelCtx,
    text: SteelVal,
    anchor: SteelVal,
    dismiss_on_key: SteelVal,
) -> SteelResult {
    let text = string_arg(text, "show-popup! text")?;
    let anchor = string_arg(anchor, "show-popup! #:anchor")?;
    if anchor != "cursor" {
        steel::stop!(Generic => "show-popup!: #:anchor must be 'cursor, got '{}'", anchor);
    }
    let dismiss_on_key = bool_arg(dismiss_on_key, "show-popup! #:dismiss-on-key")?;
    require_cap(ctx.host.ui(), "show-popup!")?
        .show_popup(text, dismiss_on_key)
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

/// `(show-drawer-list! items on-select)`.
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
/// dotted pair (`docs/FUZZY-FINDERS.md` Q-B3) — a proper list entry is
/// rejected by `pair_fields`. `payload` stays an opaque `SteelVal`; Rust
/// never interprets it.
fn picker_items(items: SteelVal, ctx_name: &str) -> Result<Vec<(String, SteelVal)>, SteelErr> {
    list_items(items, ctx_name)?
        .into_iter()
        .map(|entry| {
            let (display, payload) = pair_fields(entry, ctx_name, "(display . payload)")?;
            Ok((string_arg(display, ctx_name)?, payload))
        })
        .collect()
}

/// `(%picker! items on-select prompt)` — the `picker!` Scheme wrapper
/// supplies `#:prompt`'s default. Returns the new session's token.
pub(crate) fn picker(
    ctx: &mut SteelCtx,
    items: SteelVal,
    on_select: SteelVal,
    prompt: SteelVal,
) -> SteelResult {
    let items = picker_items(items, "picker! items")?;
    let prompt = string_arg(prompt, "picker! #:prompt")?;
    let token = require_cap(ctx.host.ui(), "picker!")?
        .open_picker(items, prompt, on_select)
        .map_err(generic_err)?;
    Ok(SteelVal::IntV(token as isize))
}

/// `(picker-push! token items)` — no keyword defaults, so this registers
/// directly. Returns whether the push was applied (`#f` for a stale token
/// or no open picker — never an error, both are expected-normal races).
pub(crate) fn picker_push(ctx: &mut SteelCtx, token: SteelVal, items: SteelVal) -> SteelResult {
    let token = usize_arg(token, "picker-push! token")? as u64;
    let items = picker_items(items, "picker-push! items")?;
    let applied = require_cap(ctx.host.ui(), "picker-push!")?.picker_push(token, items);
    Ok(SteelVal::BoolV(applied))
}

/// `(%picker-source-spawn! token cmd args cwd nul)` — the
/// `picker-source-spawn!` Scheme wrapper supplies `#:cwd`/`#:nul`'s
/// defaults. A stale token or no open picker returns `#f` without spawning
/// anything, the same expected-normal-race contract as `picker-push!`; a
/// genuine spawn failure (missing binary, bad `#:cwd`) raises.
pub(crate) fn picker_source_spawn(
    ctx: &mut SteelCtx,
    token: SteelVal,
    cmd: SteelVal,
    args: SteelVal,
    cwd: SteelVal,
    nul: SteelVal,
) -> SteelResult {
    let token = usize_arg(token, "picker-source-spawn! token")? as u64;
    let cmd = string_arg(cmd, "picker-source-spawn! cmd")?;
    if cmd.trim().is_empty() {
        steel::stop!(Generic => "picker-source-spawn!: cmd must not be empty");
    }
    let args = list_to_strings(args, "picker-source-spawn! args")?;
    let cwd = optional_path_arg(cwd, "picker-source-spawn! #:cwd")?;
    let nul = bool_arg(nul, "picker-source-spawn! #:nul")?;

    let applied = require_cap(ctx.host.ui(), "picker-source-spawn!")?
        .picker_source_spawn(token, &cmd, args, cwd, nul)
        .map_err(|e| generic_err(format!("picker-source-spawn!: {e}")))?;
    Ok(SteelVal::BoolV(applied))
}

/// `(picker-close!)`.
pub(crate) fn picker_close(ctx: &mut SteelCtx) -> SteelResult {
    require_cap(ctx.host.ui(), "picker-close!")?.picker_close();
    Ok(SteelVal::Void)
}
