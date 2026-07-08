//! Generic Steel-scriptable UI widget builtins.
//!
//! LSP is the first client of these widgets, not their owner — any plugin
//! can call `show-popup!`. `hume-editor`'s `EditorHostImpl` is the only
//! implementation that actually renders anything; other hosts (tests,
//! `MockHost`) get the trait's "not supported" default.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;

use super::{conv_err, require_cmd_ctx, string_arg};

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
