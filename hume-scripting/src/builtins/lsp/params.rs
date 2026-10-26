//! The request params a builtin builds from a pane's document, position or
//! ranges, and the tracked positions a later request can be built from.

use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::host::{PositionParams, RangeParams, RangesParams};
use crate::types::PaneHandle;

use crate::builtins::SteelResult;
use crate::builtins::args::{list_of, string_hash, token_arg};
use crate::builtins::errors::{generic_err, require_cap};
use crate::builtins::ids::DocRange;

/// The JSON-shaped `{"uri" uri}` a params hash names its document by.
fn text_document(uri: String) -> SteelVal {
    string_hash([("uri", SteelVal::StringV(uri.into()))])
}

/// `{"textDocument" {"uri"} "position" <position>}`: a position stays an
/// opaque value until the request is sent.
pub(super) fn position_params_value(params: PositionParams) -> SteelVal {
    string_hash([
        ("textDocument", text_document(params.uri)),
        ("position", params.pos.into_steel_val()),
    ])
}

fn range_params_value(params: RangeParams) -> SteelVal {
    string_hash([
        ("textDocument", text_document(params.uri)),
        ("range", params.range.into_steel_val()),
    ])
}

fn ranges_params_value(params: RangesParams) -> SteelVal {
    string_hash([
        ("textDocument", text_document(params.uri)),
        (
            "ranges",
            list_of(params.ranges.into_iter().map(DocRange::into_steel_val)),
        ),
    ])
}

/// A kind-B params read as a Steel value: `Err` (a stale `pane`) raises,
/// `Ok(None)` (a buffer with no path) is `#f`.
fn params_result<T>(result: Result<Option<T>, String>, to_steel: fn(T) -> SteelVal) -> SteelResult {
    let params = result.map_err(generic_err)?;
    Ok(params.map_or(SteelVal::BoolV(false), to_steel))
}

/// `(lsp-document-params pane)` → `{"textDocument" {"uri"}}` for the buffer
/// `pane` names, or `#f` for a buffer with no path. Reads only the buffer, so
/// a value with no pane works.
pub(crate) fn lsp_document_params(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    Ok(require_cap(ctx.host.lsp())?
        .lsp_document_uri(pane.buffer())
        .map_or(SteelVal::BoolV(false), |uri| {
            string_hash([("textDocument", text_document(uri))])
        }))
}

/// `(lsp-position-params pane)` → `{"textDocument" {"uri"} "position" p}`
/// for the primary cursor head in `pane`'s own pane, `p` an opaque position
/// each server receives in its own encoding, or `#f` for a buffer with no
/// path. Raises (kind-B fail-fast) when `pane` carries no pane, a closed
/// one, or one that no longer shows its buffer.
pub(crate) fn lsp_position_params(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    params_result(
        require_cap(ctx.host.lsp())?.lsp_position_params(pane),
        position_params_value,
    )
}

/// `(track-position! pane)` → a token naming the primary cursor head in
/// `pane`'s own pane, carried through every edit. Raises (kind-B fail-fast)
/// when `pane` carries no pane, a closed one, or one that no longer shows its
/// buffer.
pub(crate) fn track_position(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    let token = require_cap(ctx.host.lsp())?
        .track_position(pane)
        .map_err(generic_err)?;
    Ok(token.to_steel())
}

/// `(tracked-position-params token)` → the `lsp-position-params` shape for
/// where the tracked position is now, or `#f` for a released or unknown
/// token, a closed or replaced buffer, or a buffer with no path. A `#f`
/// token is that same stale case; only a non-integer raises.
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
pub(crate) fn lsp_primary_range_params(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    params_result(
        require_cap(ctx.host.lsp())?.lsp_primary_range_params(pane),
        range_params_value,
    )
}

/// `(lsp-linewise-ranges-params pane)` → `{"textDocument" {"uri"} "ranges"
/// [...]}`, one opaque range per linewise selection in `pane`'s own pane
/// (touching selections coalesced into one range apiece). Carries no
/// all/none/mixed verdict. `:format-source` gets that from `selections-linewise?`/
/// `selections-charwise?` instead, since every `lsp-*-params` builtin's
/// return value is a wire-ready params hash forwarded to `lsp-request!`
/// unchanged or with a protocol key inserted, and a non-protocol key here
/// would break that (see `CursorHost::selections_linewise`'s doc comment).
pub(crate) fn lsp_linewise_ranges_params(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    params_result(
        require_cap(ctx.host.lsp())?.lsp_linewise_ranges_params(pane),
        ranges_params_value,
    )
}
