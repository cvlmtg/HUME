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

use super::args::{bool_arg, list_to_strings, string_arg};
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
