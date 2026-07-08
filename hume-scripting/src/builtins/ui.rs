//! Generic Steel-scriptable UI widget builtins.
//!
//! LSP is the first client of these widgets, not their owner — any plugin
//! can call `show-popup!`. `hume-editor`'s `EditorHostImpl` is the only
//! implementation that actually renders anything; other hosts (tests,
//! `MockHost`) get the trait's "not supported" default.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;

use super::{conv_err, list_to_strings, require_cmd_ctx, string_arg};

type SteelResult = Result<SteelVal, SteelErr>;

/// `(%show-popup! text anchor)` — the `show-popup!` Scheme wrapper supplies
/// `#:anchor`'s default. `'cursor` is the only anchor accepted in v1.
pub(crate) fn show_popup(ctx: &mut SteelCtx, text: SteelVal, anchor: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "show-popup!");
    let text = string_arg(text, "show-popup! text")?;
    let anchor = string_arg(anchor, "show-popup! #:anchor")?;
    if anchor != "cursor" {
        steel::stop!(Generic => "show-popup!: #:anchor must be 'cursor, got '{}'", anchor);
    }
    ctx.host
        .show_popup(text)
        .map(|()| SteelVal::Void)
        .map_err(conv_err)
}

/// `(%close-popup!)`.
pub(crate) fn close_popup(ctx: &mut SteelCtx) -> SteelResult {
    require_cmd_ctx!(ctx, "close-popup!");
    ctx.host.close_popup().map(|()| SteelVal::Void).map_err(conv_err)
}

/// `(show-menu! items on-select)` — no keyword defaults, so this registers
/// directly (no `%`-prefix wrapper needed).
pub(crate) fn show_menu(ctx: &mut SteelCtx, items: SteelVal, on_select: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "show-menu!");
    let items = list_to_strings(items, "show-menu! items")?;
    ctx.host
        .show_menu(items, on_select)
        .map(|()| SteelVal::Void)
        .map_err(conv_err)
}

/// `(close-menu!)`.
pub(crate) fn close_menu(ctx: &mut SteelCtx) -> SteelResult {
    require_cmd_ctx!(ctx, "close-menu!");
    ctx.host.close_menu().map(|()| SteelVal::Void).map_err(conv_err)
}
