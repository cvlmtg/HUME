//! The request params a builtin builds from a pane's document, position or
//! ranges, and the tracked positions a later request can be built from.

use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::host::{PositionParams, RangeParams, RangesParams, ShownHandle, token_to_steel};
use hume_core::BufferId;

use crate::builtins::SteelResult;
use crate::builtins::args::{list_of, string_hash, token_arg};
use crate::builtins::errors::{generic_err, require_cap};
use crate::builtins::ids::{DocRange, DocUri};

/// The `{"uri" <doc>}` a params hash names its document by, `<doc>` an
/// opaque value filled in when the request is sent.
fn text_document(doc: DocUri) -> SteelVal {
    string_hash([("uri", doc.into_steel_val())])
}

/// `{"textDocument" {"uri"} "position" <position>}`: a position stays an
/// opaque value until the request is sent.
pub(super) fn position_params_value(params: PositionParams) -> SteelVal {
    string_hash([
        ("textDocument", text_document(params.doc)),
        ("position", params.pos.into_steel_val()),
    ])
}

fn range_params_value(params: RangeParams) -> SteelVal {
    string_hash([
        ("textDocument", text_document(params.doc)),
        ("range", params.range.into_steel_val()),
    ])
}

fn ranges_params_value(params: RangesParams) -> SteelVal {
    string_hash([
        ("textDocument", text_document(params.doc)),
        (
            "ranges",
            list_of(params.ranges.into_iter().map(DocRange::into_steel_val)),
        ),
    ])
}

/// `(lsp-document-params pane)` → `{"textDocument" {"uri"}}` for the buffer
/// `pane` names. Reads only the buffer, so a value with no pane works.
pub(crate) fn lsp_document_params(ctx: &mut SteelCtx, bid: BufferId) -> SteelResult {
    require_cap(ctx.host.lsp())?;
    Ok(string_hash([(
        "textDocument",
        text_document(DocUri::new(bid)),
    )]))
}

/// `(lsp-position-params pane)` → `{"textDocument" {"uri"} "position" p}`
/// for the primary cursor head in `pane`'s pane, `p` an opaque position
/// each server receives in its own encoding.
pub(crate) fn lsp_position_params(ctx: &mut SteelCtx, pane: ShownHandle) -> SteelResult {
    require_cap(ctx.host.lsp())?
        .lsp_position_params(pane)
        .map(position_params_value)
        .map_err(generic_err)
}

/// `(track-position! pane)` → a token naming the primary cursor head in
/// `pane`'s pane, carried through every edit.
pub(crate) fn track_position(ctx: &mut SteelCtx, pane: ShownHandle) -> SteelResult {
    let token = require_cap(ctx.host.lsp())?
        .track_position(pane)
        .map_err(generic_err)?;
    Ok(token_to_steel(token))
}

/// `(tracked-position-params token)` → the `lsp-position-params` shape for
/// where the tracked position is now, or `#f` for a released or unknown
/// token or a closed or replaced buffer. A `#f` token is that same stale
/// case; only a non-integer raises.
pub(crate) fn tracked_position_params(ctx: &mut SteelCtx, token: SteelVal) -> SteelResult {
    let token = token_arg(token, "token")?;
    Ok(require_cap(ctx.host.lsp())?
        .tracked_position_params(token)
        .map_or(SteelVal::BoolV(false), position_params_value))
}

/// `(keep-tracked-position! token)`: keeps a position an `lsp-request!`
/// holds through `#:tracked` past its callback; the caller releases it with
/// `untrack-position!`. A `#f`, released or unknown token is a no-op.
pub(crate) fn keep_tracked_position(ctx: &mut SteelCtx, token: SteelVal) -> SteelResult {
    let token = token_arg(token, "token")?;
    require_cap(ctx.host.lsp())?.keep_tracked_position(token);
    Ok(SteelVal::Void)
}

/// `(untrack-position! token)`: releases the position. A `#f`, released or
/// unknown token is a no-op.
pub(crate) fn untrack_position(ctx: &mut SteelCtx, token: SteelVal) -> SteelResult {
    let token = token_arg(token, "token")?;
    require_cap(ctx.host.lsp())?.untrack_position(token);
    Ok(SteelVal::Void)
}

/// `(lsp-primary-range-params pane)` → same shape but a `"range"` from the
/// primary selection alone, an opaque range value.
pub(crate) fn lsp_primary_range_params(ctx: &mut SteelCtx, pane: ShownHandle) -> SteelResult {
    require_cap(ctx.host.lsp())?
        .lsp_primary_range_params(pane)
        .map(range_params_value)
        .map_err(generic_err)
}

/// `(lsp-linewise-ranges-params pane)` → `{"textDocument" {"uri"} "ranges"
/// [...]}`, one opaque range per linewise selection in `pane`'s own pane
/// (touching selections coalesced into one range apiece). Carries no
/// all/none/mixed verdict. `:format-source` gets that from `selections-linewise?`/
/// `selections-charwise?` instead, since every `lsp-*-params` builtin's
/// return value is a wire-ready params hash forwarded to `lsp-request!`
/// unchanged or with a protocol key inserted, and a non-protocol key here
/// would break that (see `CursorHost::selections_linewise`'s doc comment).
pub(crate) fn lsp_linewise_ranges_params(ctx: &mut SteelCtx, pane: ShownHandle) -> SteelResult {
    require_cap(ctx.host.lsp())?
        .lsp_linewise_ranges_params(pane)
        .map(ranges_params_value)
        .map_err(generic_err)
}
