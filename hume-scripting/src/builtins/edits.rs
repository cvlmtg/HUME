//! BufferText-edit application and cursor navigation primitives fed by LSP
//! responses (code actions, rename, go-to-definition); `insert-key!` too,
//! which isn't LSP-driven but shares `EditHost`'s capability gate (see that
//! trait's own doc).

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::types::PaneHandle;

use super::SteelResult;
use super::args::{
    checked_fields, json_arg, list_items, optional_usize_arg, single_key_arg, string_arg,
    usize_arg, wire_text_edit_arg,
};
use super::errors::{generic_err, require_cap};

/// `(%apply-text-edits! pane edits expect-gen)`. `edits` is a list of
/// `JsonHandle`s onto wire `TextEdit`s (unconverted response elements, e.g.
/// from `textDocument/formatting`); see `wire_text_edit_arg`.
///
/// `edits` decodes manually via `wire_text_edit_arg` per entry rather than a
/// typed `Vec<WireTextEdit>` param: steel-core's blanket
/// `FromSteelVal for Vec<T>` impl discards the inner per-element error on
/// failure, replacing it with a generic message; decoding manually keeps
/// `wire_text_edit_arg`'s specific shape-error text.
pub(crate) fn apply_text_edits(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    edits: SteelVal,
    expect_gen: SteelVal,
) -> SteelResult {
    let expect_gen =
        optional_usize_arg(expect_gen, "apply-text-edits! expect-gen")?.map(|n| n as u64);
    let parsed = list_items(edits, "apply-text-edits! edits")?
        .into_iter()
        .map(wire_text_edit_arg)
        .collect::<Result<Vec<_>, SteelErr>>()?;
    require_cap(ctx.host.edits(), "apply-text-edits!")?
        .apply_text_edits(pane, parsed, expect_gen)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(%apply-workspace-edit! pane wsedit expect-gen)`. `wsedit` is a
/// `WorkspaceEdit` hashmap or JSON handle. Its positions decode using the
/// handle's own tagged encoding (the server that produced it; see
/// `JsonHandle::position_encoding`), so unlike `goto-location!`'s
/// char-indexed shape this errors on a hand-built (untagged) value: there
/// is no server to have negotiated an encoding with. Returns the number of
/// buffers modified; the `apply-workspace-edit!` Scheme wrapper reports
/// that count.
pub(crate) fn apply_workspace_edit(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    wsedit: SteelVal,
    expect_gen: SteelVal,
) -> SteelResult {
    let handle = json_arg(wsedit, "apply-workspace-edit!")?;
    let encoding = handle
        .position_encoding("apply-workspace-edit!")
        .map_err(generic_err)?;
    let expect_gen =
        optional_usize_arg(expect_gen, "apply-workspace-edit! expect-gen")?.map(|n| n as u64);
    let count = require_cap(ctx.host.edits(), "apply-workspace-edit!")?
        .apply_workspace_edit(pane, handle.value(), encoding, expect_gen)
        .map_err(generic_err)?;
    Ok(SteelVal::IntV(count as isize))
}

/// `(goto-location! pane loc)`: `loc` is one of two shapes, dispatched here
/// (not in Scheme):
///
/// - a raw `Location`/`LocationLink` hashmap or JSON handle: wire position,
///   decoded and converted using the handle's own tagged encoding. The
///   server that produced the response negotiated it for the request that's
///   being answered, regardless of which file the location points into (an
///   LSP round-trip is async; the user is free to switch panes while a
///   request is in flight, which is exactly why the jump lands in `pane`,
///   not necessarily the focused one). Errors on an untagged (hand-built)
///   value, same as `apply-workspace-edit!`.
/// - `(list target line char-col)`, already char-indexed: `target` is a
///   path string, a `file://` URI string, or a pane. This shape never
///   touches server encoding.
pub(crate) fn goto_location(ctx: &mut SteelCtx, pane: PaneHandle, loc: SteelVal) -> SteelResult {
    match &loc {
        SteelVal::HashMapV(_) | SteelVal::Custom(_) => {
            let handle = json_arg(loc, "goto-location!")?;
            let encoding = handle
                .position_encoding("goto-location!")
                .map_err(generic_err)?;
            require_cap(ctx.host.edits(), "goto-location!")?
                .goto_location_value(pane, handle.value(), encoding)
                .map(|()| SteelVal::Void)
                .map_err(generic_err)
        }
        SteelVal::ListV(_) => {
            let fields = checked_fields(
                loc.clone(),
                "goto-location!",
                3..=3,
                "(target line char-col)",
            )?;
            let target = fields[0].clone();
            let line = hume_rope::line::RopeyLine::new(usize_arg(
                fields[1].clone(),
                "goto-location! line",
            )?);
            let char_col = usize_arg(fields[2].clone(), "goto-location! char-col")?;
            if let Some(handle) = super::ids::downcast_pane(&target) {
                require_cap(ctx.host.edits(), "goto-location!")?
                    .goto_location_buffer(pane, handle.buffer(), line, char_col)
                    .map(|()| SteelVal::Void)
                    .map_err(generic_err)
            } else {
                let s = string_arg(target, "goto-location! target")?;
                require_cap(ctx.host.edits(), "goto-location!")?
                    .goto_location_path(pane, s, line, char_col)
                    .map(|()| SteelVal::Void)
                    .map_err(generic_err)
            }
        }
        _ => steel::stop!(TypeMismatch =>
            "goto-location!: expected a Location hashmap/handle or (list target line char-col)"),
    }
}

/// `(insert-key! pane key)`: `key` is a `bind-key!`-syntax spec naming
/// exactly one chord (see [`single_key_arg`]), decoded here rather than
/// left to the host: the host trait takes an already-parsed `KeyEvent`,
/// same as every other typed `EditHost` param.
pub(crate) fn insert_key(ctx: &mut SteelCtx, pane: PaneHandle, key: SteelVal) -> SteelResult {
    let key = single_key_arg(key, "insert-key!")?;
    require_cap(ctx.host.edits(), "insert-key!")?
        .insert_key(pane, key)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

#[cfg(test)]
mod tests;
