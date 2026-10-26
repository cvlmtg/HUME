//! Conversions between a server's wire positions and locations and the
//! editor's char offsets and drawer places.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::host::{LocationPlace, Place};
use crate::types::PaneHandle;

use crate::builtins::SteelResult;
use crate::builtins::args::{char_range_value, list_items, string_arg, symbol_hash};
use crate::builtins::errors::{generic_err, require_cap};
use crate::builtins::ids::SteelPane;
use crate::builtins::wire::{tagged_json_arg, wire_position, wire_range};

/// `(lsp-position->char-offset pane position)` → `pane`'s buffer's char offset
/// for the wire `{"line" "character"}` hashmap `position`, decoded in
/// `position`'s own tagged producing-server encoding (`Err` if `position`
/// isn't a value from an LSP server response; see
/// `JsonHandle::position_encoding`), or `#f` if `position` would land on
/// the buffer's trailing phantom line or past it (a stale response racing an
/// edit, or a server's past-end convention). Every point-anchored decoration
/// setter (`set-inlay-hints!`) rejects that offset outright, so refusing here
/// lets a caller filter one bad entry instead of the whole setter call
/// failing on it. Raises when the host has no LSP capability.
pub(crate) fn lsp_position_to_char_offset(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    position: SteelVal,
) -> SteelResult {
    let bid = pane.buffer();
    let (handle, encoding) = tagged_json_arg(&position, "position")?;
    let pos = wire_position(handle.value())?;
    let offset = require_cap(ctx.host.lsp())?.lsp_wire_point_to_char(bid, pos, encoding);
    Ok(offset.map_or(SteelVal::BoolV(false), |offset| {
        SteelVal::IntV(offset as isize)
    }))
}

/// `(lsp-range->char-range pane range)` → `(hash 'start-char s 'end-char e)` half-open char
/// offsets for the wire `{"start" {"line" "character"} "end" {"line"
/// "character"}}` hashmap `range`, same encoding rule as
/// `lsp-position->char-offset`, decoded in `range`'s own tagged producing-server
/// encoding. A `character` past its line's end clamps, and an end may sit at
/// the document end `(line count, 0)`. `#f` when an end is on a line past it.
/// Raises when the host has no LSP capability.
pub(crate) fn lsp_range_to_char_range(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    range: SteelVal,
) -> SteelResult {
    let bid = pane.buffer();
    let (handle, encoding) = tagged_json_arg(&range, "range")?;
    let range = wire_range(handle.value())?;
    let range = require_cap(ctx.host.lsp())?.lsp_wire_range_to_chars(bid, range, encoding);
    Ok(range.map_or(SteelVal::BoolV(false), |(start, end)| {
        char_range_value(start, end)
    }))
}

/// `(lsp-label-offsets->text label offsets)` → the slice of `label` that a
/// `ParameterInformation.label` `[start, end)` wire offset pair names.
/// Raises when the host has no LSP capability.
///
/// `offsets` is the raw two-element `[start, end)` array straight off the
/// wire, handed over undecoded the way `goto-location!` takes a raw
/// `Location`: a JSON array handle. Its encoding is read from its own tag: it
/// is itself drawn from the same response `label` came from (`json-ref`), so
/// it carries the producing server's negotiated encoding, and a hand-built
/// value raises, since there is then no server to have negotiated one with. The offsets count code units in that encoding, so
/// Scheme can neither convert them nor index by them. That is the whole
/// reason this builtin exists rather than a Scheme helper.
pub(crate) fn lsp_label_offsets_to_text(
    ctx: &mut SteelCtx,
    label: SteelVal,
    offsets: SteelVal,
) -> SteelResult {
    let label = string_arg(label, "label")?;
    let (handle, encoding) = tagged_json_arg(&offsets, "offsets")?;
    let arr = handle
        .value()
        .as_array()
        .ok_or_else(|| generic_err("offsets must be a two-element (start end) array"))?;
    let [start, end] = arr.as_slice() else {
        return Err(generic_err(format!(
            "offsets must be a two-element (start end) array, got {} element(s)",
            arr.len()
        )));
    };
    let start = json_usize(start, "offsets")?;
    let end = json_usize(end, "offsets")?;
    require_cap(ctx.host.lsp())?;
    let range =
        hume_rope::position_encoding::wire_offsets_to_byte_range(&label, start, end, encoding);
    Ok(SteelVal::StringV(label[range].into()))
}

/// A non-negative JSON integer, for a field read directly off a
/// `serde_json::Value` rather than decoded through a `SteelVal` arg (see
/// `crate::builtins::args::usize_arg`'s Steel-side counterpart).
fn json_usize(v: &serde_json::Value, ctx_name: &str) -> Result<usize, SteelErr> {
    v.as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| {
            generic_err(format!(
                "{ctx_name}: expected a non-negative integer, got {v}"
            ))
        })
}

/// Decodes `locs` (a Steel list of raw `Location`/`LocationLink` handles)
/// through the host, one row per location.
fn decode_places(ctx: &mut SteelCtx, locs: SteelVal) -> Result<Vec<LocationPlace>, SteelErr> {
    let handles = list_items(locs, "locs")?
        .iter()
        .map(|entry| tagged_json_arg(entry, "locs"))
        .collect::<Result<Vec<_>, _>>()?;
    require_cap(ctx.host.lsp())?
        .lsp_location_places(&handles)
        .map_err(generic_err)
}

fn place_hash(place: Place) -> SteelVal {
    symbol_hash([
        ("path", SteelVal::StringV(place.path.into())),
        ("line", SteelVal::IntV(place.line as isize)),
        (
            "grapheme-col-or-wire",
            SteelVal::IntV(place.grapheme_col_or_wire as isize),
        ),
        (
            "buffer",
            match place.buffer {
                Some(bid) => SteelPane(PaneHandle::buffer_only(bid)).into_steel_val(),
                None => SteelVal::BoolV(false),
            },
        ),
        ("location", place.location.into_steel_val()),
    ])
}

/// `(lsp-locations->places locs)` → one entry per element of `locs`
/// (a list of handles onto raw `Location`/`LocationLink` values from a
/// server response), in order:
/// `(hash 'path p 'line l 'grapheme-col-or-wire c 'buffer b 'location loc)`,
/// or `#f` for a location with no place `goto-location!` can jump to: a URI
/// that is not a `file://` one, or a line past the content of the file's open
/// buffer. Entries are not
/// merged, so a caller can pair them back with the values the locations came
/// from. This is the
/// display-side counterpart to `goto-location!`'s wire conversion, decoded
/// through the same shared decoder. Each entry reads its own tagged
/// producing-server encoding (see `LspHost::lsp_location_places`'s
/// doc). The column is an exact grapheme column when the target has an open
/// buffer, and otherwise the location's own wire `character` verbatim; this function
/// never reads a target file to refine that last case. `path`/`line` are
/// always present, since they come from the location itself. `buffer` is a
/// buffer-only pane handle for the open buffer the target is, `#f` when the
/// file is not open. See `LspHost::lsp_location_places`'s doc for the
/// full column-unit rule and why a location that can't be decoded (or isn't
/// tagged) at all aborts the whole call rather than producing a degraded
/// entry.
pub(crate) fn lsp_locations_to_places(ctx: &mut SteelCtx, locs: SteelVal) -> SteelResult {
    let rows = decode_places(ctx, locs)?;
    let entries: Vec<SteelVal> = rows
        .into_iter()
        .map(|row| match row {
            LocationPlace::Found(place) => place_hash(place),
            LocationPlace::Skipped => SteelVal::BoolV(false),
        })
        .collect();
    Ok(SteelVal::ListV(entries.into()))
}

/// `(lsp-locations->merged-places locs)` → `(hash 'places l 'skipped n)`:
/// `l` is the entries of `lsp-locations->places` without the `#f` ones and
/// with the first kept for each path, line and column, a list of places to
/// show or jump to, since several servers answering one request often name
/// the same place. `n` counts the `#f` entries left out, so a caller reports
/// them without decoding the answer a second time. Two
/// servers counting one unopened non-ASCII position in different encodings
/// still show twice, since the column of an unopened target is the wire one.
pub(crate) fn lsp_locations_to_merged_places(ctx: &mut SteelCtx, locs: SteelVal) -> SteelResult {
    let rows = decode_places(ctx, locs)?;
    let mut seen = rustc_hash::FxHashSet::default();
    let mut places = Vec::new();
    let mut skipped = 0;
    for row in rows {
        match row {
            LocationPlace::Found(place) => {
                if seen.insert((place.path.clone(), place.line, place.grapheme_col_or_wire)) {
                    places.push(place_hash(place));
                }
            }
            LocationPlace::Skipped => skipped += 1,
        }
    }
    Ok(symbol_hash([
        ("places", SteelVal::ListV(places.into())),
        ("skipped", SteelVal::IntV(skipped)),
    ]))
}
