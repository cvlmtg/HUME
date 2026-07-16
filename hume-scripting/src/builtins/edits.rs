//! Text-edit application and cursor navigation primitives fed by LSP
//! responses (code actions, rename, go-to-definition), but not LSP
//! transport themselves.

use steel::rerrs::SteelErr;
use steel::rvals::{FromSteelVal, SteelVal};

use crate::SteelCtx;
use crate::json::steel_to_json;

use super::args::{BidArg, TextEditArg, checked_fields, list_items, optional_usize_arg, string_arg, usize_arg};
use super::errors::generic_err;

type SteelResult = Result<SteelVal, SteelErr>;

/// `(%apply-text-edits! bid edits expect-gen)` — `edits`: list of `((start-
/// line start-col) (end-line end-col) text)`, wire positions.
///
/// `edits` decodes manually via `TextEditArg::from_steelval` per entry
/// rather than a typed `Vec<TextEditArg>` param — steel-core's blanket
/// `FromSteelVal for Vec<T>` impl discards the inner per-element error on
/// failure, replacing it with a generic message; decoding manually keeps
/// `TextEditArg`'s specific shape-error text.
pub(crate) fn apply_text_edits(
    ctx: &mut SteelCtx,
    bid: BidArg,
    edits: SteelVal,
    expect_gen: SteelVal,
) -> SteelResult {
    let id = bid.0;
    let expect_gen =
        optional_usize_arg(expect_gen, "apply-text-edits! expect-gen")?.map(|n| n as u64);
    let parsed = list_items(edits, "apply-text-edits! edits")?
        .iter()
        .map(|entry| {
            let edit = TextEditArg::from_steelval(entry)?;
            Ok((
                edit.start.line,
                edit.start.col,
                edit.end.line,
                edit.end.col,
                edit.text,
            ))
        })
        .collect::<Result<Vec<_>, SteelErr>>()?;
    ctx.host
        .edits()
        .ok_or_else(|| generic_err(crate::host::unsupported("apply-text-edits!")))?
        .apply_text_edits(id, parsed, expect_gen)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(%apply-workspace-edit! wsedit)` — `wsedit`: the decoded `WorkspaceEdit`
/// hashmap (JSON↔SteelVal shape). Returns the number of buffers modified;
/// the `apply-workspace-edit!` Scheme wrapper reports that count.
pub(crate) fn apply_workspace_edit(ctx: &mut SteelCtx, wsedit: SteelVal) -> SteelResult {
    let json = steel_to_json(&wsedit).map_err(generic_err)?;
    let count = ctx
        .host
        .edits()
        .ok_or_else(|| generic_err(crate::host::unsupported("apply-workspace-edit!")))?
        .apply_workspace_edit(json)
        .map_err(generic_err)?;
    Ok(SteelVal::IntV(count as isize))
}

/// `(goto-location! loc)` — `loc` is one of two shapes, dispatched here (not
/// in Scheme): a raw `Location`/`LocationLink` hashmap (wire
/// position, converted using the focused buffer's server encoding — correct
/// because the caller is that server's own response callback), or `(list
/// target line col)` with char-indexed `line`/`col` and `target` a `bid`, a
/// path string, or a `file://` URI string.
pub(crate) fn goto_location(ctx: &mut SteelCtx, loc: SteelVal) -> SteelResult {
    match &loc {
        SteelVal::HashMapV(_) => {
            let json = steel_to_json(&loc).map_err(generic_err)?;
            let (uri, range) = if let Some(uri) = json.get("targetUri") {
                (
                    uri,
                    json.get("targetSelectionRange")
                        .or_else(|| json.get("targetRange")),
                )
            } else {
                (
                    json.get("uri")
                        .ok_or_else(|| generic_err("goto-location!: missing uri"))?,
                    json.get("range"),
                )
            };
            let uri = uri
                .as_str()
                .ok_or_else(|| generic_err("goto-location!: uri must be a string"))?
                .to_string();
            let range = range.ok_or_else(|| generic_err("goto-location!: missing range"))?;
            let line = range
                .pointer("/start/line")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| generic_err("goto-location!: missing range.start.line"))?
                as usize;
            let character = range
                .pointer("/start/character")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| generic_err("goto-location!: missing range.start.character"))?
                as usize;
            ctx.host
                .edits()
                .ok_or_else(|| generic_err(crate::host::unsupported("goto-location!")))?
                .goto_location_wire(uri, line, character)
                .map(|()| SteelVal::Void)
                .map_err(generic_err)
        }
        SteelVal::ListV(_) => {
            let fields = checked_fields(loc.clone(), "goto-location!", 3..=3, "(target line col)")?;
            let target = fields[0].clone();
            let line = usize_arg(fields[1].clone(), "goto-location! line")?;
            let col = usize_arg(fields[2].clone(), "goto-location! col")?;
            if let Some(bid) = super::ids::downcast_buffer_id(&target) {
                ctx.host
                    .edits()
                    .ok_or_else(|| generic_err(crate::host::unsupported("goto-location!")))?
                    .goto_location_buffer(bid, line, col)
                    .map(|()| SteelVal::Void)
                    .map_err(generic_err)
            } else {
                let s = string_arg(target, "goto-location! target")?;
                ctx.host
                    .edits()
                    .ok_or_else(|| generic_err(crate::host::unsupported("goto-location!")))?
                    .goto_location_path(s, line, col)
                    .map(|()| SteelVal::Void)
                    .map_err(generic_err)
            }
        }
        _ => steel::stop!(TypeMismatch =>
            "goto-location!: expected a Location hashmap or (list target line col)"),
    }
}
