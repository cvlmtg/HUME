//! Decoration stores (inlay hints, signs, virtual lines, EOL text, extra
//! highlights) and the diagnostics pull API. Not LSP-specific — any Steel
//! plugin can populate these — but LSP is the first and heaviest client.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::json::{json_to_steel, steel_to_json};
use crate::types::VirtualLineSpec;

use super::SteelResult;
use super::args::{
    BidArg, cons_pair, int_arg, list_items, pair_fields, string_arg, tuple_list, usize_arg,
};
use super::errors::{generic_err, require_cap};

/// `(set-inlay-hints! source bid hints)` — `hints`: list of `(position text
/// 'before|'after)`, `position` a wire `{"line" "character"}` hashmap.
pub(crate) fn set_inlay_hints(
    ctx: &mut SteelCtx,
    source: SteelVal,
    bid: BidArg,
    hints: SteelVal,
) -> SteelResult {
    let source = string_arg(source, "set-inlay-hints! source")?;
    let id = bid.0;
    let parsed = tuple_list(
        hints,
        "set-inlay-hints! hints",
        3..=3,
        "(position text 'before|'after)",
        |fields| {
            let position_json = steel_to_json(&fields[0])
                .map_err(|e| generic_err(format!("set-inlay-hints! position: {e}")))?;
            // Validated here, at the boundary, rather than left to the host
            // side's extraction — a malformed position must error loudly, not
            // silently drop the hint (host_impl.rs's `set_inlay_hints` treats
            // this shape as already guaranteed).
            let has_valid_position = position_json.get("line").is_some_and(|v| v.is_u64())
                && position_json.get("character").is_some_and(|v| v.is_u64());
            if !has_valid_position {
                steel::stop!(Generic =>
                    "set-inlay-hints!: position must be a hashmap with numeric 'line' and 'character' keys, got {}",
                    position_json
                );
            }
            let text = string_arg(fields[1].clone(), "set-inlay-hints! text")?;
            let before = match &fields[2] {
                SteelVal::SymbolV(s) if s.as_str() == "before" => true,
                SteelVal::SymbolV(s) if s.as_str() == "after" => false,
                _ => {
                    steel::stop!(Generic => "set-inlay-hints!: third element must be 'before or 'after")
                }
            };
            Ok((position_json, text, before))
        },
    )?;
    require_cap(ctx.host.decorations(), "set-inlay-hints!")?.set_inlay_hints(source, id, parsed);
    Ok(SteelVal::Void)
}

/// `(set-signs! source bid signs)` — `signs`: list of `(line text scope priority)`.
pub(crate) fn set_signs(
    ctx: &mut SteelCtx,
    source: SteelVal,
    bid: BidArg,
    signs: SteelVal,
) -> SteelResult {
    let source = string_arg(source, "set-signs! source")?;
    let id = bid.0;
    let parsed = tuple_list(
        signs,
        "set-signs! signs",
        4..=4,
        "(line text scope priority)",
        |fields| {
            Ok((
                usize_arg(fields[0].clone(), "set-signs! line")?,
                string_arg(fields[1].clone(), "set-signs! text")?,
                string_arg(fields[2].clone(), "set-signs! scope")?,
                int_arg(fields[3].clone(), "set-signs! priority")?,
            ))
        },
    )?;
    require_cap(ctx.host.decorations(), "set-signs!")?.set_signs(source, id, parsed);
    Ok(SteelVal::Void)
}

/// `(set-virtual-lines! source bid lines)` — `lines`: list of hashmaps, each
/// with required `'line`/`'text`, plus optional `'anchor` (`'before` or
/// `'after`, default `'after`), `'scope` (whole-line base style — `ui.virtual`
/// fallback when absent), and `'segments` (list of `(start end scope)` char
/// ranges into `text`, styling only the covered chars; chars outside every
/// segment keep `'scope`'s style). Segment bounds/ordering/overlap are
/// validated at the host boundary, not here — see `VirtualLineSpec::segments`.
pub(crate) fn set_virtual_lines(
    ctx: &mut SteelCtx,
    source: SteelVal,
    bid: BidArg,
    lines: SteelVal,
) -> SteelResult {
    let source = string_arg(source, "set-virtual-lines! source")?;
    let id = bid.0;
    let parsed = virtual_line_specs(lines)?;
    require_cap(ctx.host.decorations(), SET_VIRTUAL_LINES)?
        .set_virtual_lines(source, id, parsed)
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

const VIRTUAL_LINE_KEYS: &[&str] = &["line", "text", "anchor", "scope", "segments"];
const SET_VIRTUAL_LINES: &str = "set-virtual-lines!";

/// Decodes `lines` into `VirtualLineSpec`s. Each entry is a hashmap, not the
/// old positional `(line text scope)` list this replaces — free to break: no
/// `.scm` plugin calls this builtin yet, only Rust tests. Only decodes shape
/// (arity, types) — segment bounds/ordering/overlap validation moved to the
/// host boundary (`host_impl.rs`'s `set_virtual_lines`), the sole enforcement
/// point for that contract now.
fn virtual_line_specs(lines: SteelVal) -> Result<Vec<VirtualLineSpec>, SteelErr> {
    list_items(lines, "set-virtual-lines! lines")?
        .into_iter()
        .map(virtual_line_spec)
        .collect()
}

fn virtual_line_spec(entry: SteelVal) -> Result<VirtualLineSpec, SteelErr> {
    let SteelVal::HashMapV(map) = &entry else {
        steel::stop!(TypeMismatch =>
            "{}: each entry must be a hashmap with 'line and 'text keys \
             (plus optional 'anchor/'scope/'segments)", SET_VIRTUAL_LINES);
    };
    for (key, _) in map.iter() {
        let SteelVal::SymbolV(key_name) = key else {
            steel::stop!(Generic =>
                "{}: hashmap key must be a symbol, got {:?}", SET_VIRTUAL_LINES, key);
        };
        if !VIRTUAL_LINE_KEYS.contains(&key_name.as_str()) {
            steel::stop!(Generic =>
                "{}: unknown key '{}, expected one of {:?}",
                SET_VIRTUAL_LINES, key_name, VIRTUAL_LINE_KEYS);
        }
    }
    let field = |k: &str| map.get(&SteelVal::SymbolV(k.into())).cloned();

    let line =
        field("line").ok_or_else(|| generic_err(format!("{SET_VIRTUAL_LINES}: missing 'line")))?;
    let line = usize_arg(line, "set-virtual-lines! line")?;

    let text =
        field("text").ok_or_else(|| generic_err(format!("{SET_VIRTUAL_LINES}: missing 'text")))?;
    let text = string_arg(text, "set-virtual-lines! text")?;
    if let Some(c) = text.chars().find(|c| c.is_control()) {
        steel::stop!(Generic =>
            "{}: 'text contains control character {:?} — virtual lines render as a \
             single row, so text must not contain newlines or other control characters",
            SET_VIRTUAL_LINES, c);
    }

    let before = match field("anchor") {
        None => false,
        Some(SteelVal::SymbolV(s)) if s.as_str() == "before" => true,
        Some(SteelVal::SymbolV(s)) if s.as_str() == "after" => false,
        Some(_) => steel::stop!(Generic =>
            "{}: 'anchor must be 'before or 'after", SET_VIRTUAL_LINES),
    };

    let scope = field("scope")
        .map(|v| string_arg(v, "set-virtual-lines! scope"))
        .transpose()?;

    let segments = match field("segments") {
        None => Vec::new(),
        Some(v) => virtual_line_segments(v)?,
    };

    Ok(VirtualLineSpec {
        line,
        text,
        before,
        scope,
        segments,
    })
}

/// Decodes `'segments`: each a `(start end scope)` char range into `text`.
/// Shape only (arity, types) — bounds, ordering, overlap, and
/// grapheme-cluster alignment are validated at the host boundary
/// (`host_impl.rs`'s `set_virtual_lines`), which also converts these char
/// offsets to the byte offsets the engine needs.
fn virtual_line_segments(segments: SteelVal) -> Result<Vec<(usize, usize, String)>, SteelErr> {
    tuple_list(
        segments,
        "set-virtual-lines! segments",
        3..=3,
        "(start end scope)",
        |fields| {
            let start = usize_arg(fields[0].clone(), "set-virtual-lines! segment start")?;
            let end = usize_arg(fields[1].clone(), "set-virtual-lines! segment end")?;
            let scope = string_arg(fields[2].clone(), "set-virtual-lines! segment scope")?;
            Ok((start, end, scope))
        },
    )
}

/// `(set-eol-text! source bid lines)` — `lines`: list of `(line text
/// scope)`. Not diagnostics-specific — the diagnostics plugin is its first
/// client, not its owner, same as every other decoration kind is to LSP.
pub(crate) fn set_eol_text(
    ctx: &mut SteelCtx,
    source: SteelVal,
    bid: BidArg,
    lines: SteelVal,
) -> SteelResult {
    let source = string_arg(source, "set-eol-text! source")?;
    let id = bid.0;
    let parsed = tuple_list(
        lines,
        "set-eol-text! lines",
        3..=3,
        "(line text scope)",
        |fields| {
            Ok((
                usize_arg(fields[0].clone(), "set-eol-text! line")?,
                string_arg(fields[1].clone(), "set-eol-text! text")?,
                string_arg(fields[2].clone(), "set-eol-text! scope")?,
            ))
        },
    )?;
    require_cap(ctx.host.decorations(), "set-eol-text!")?.set_eol_text(source, id, parsed);
    Ok(SteelVal::Void)
}

/// `(set-extra-highlights! source bid spans)` — `spans`: list of `(start end scope)`.
pub(crate) fn set_extra_highlights(
    ctx: &mut SteelCtx,
    source: SteelVal,
    bid: BidArg,
    spans: SteelVal,
) -> SteelResult {
    let source = string_arg(source, "set-extra-highlights! source")?;
    let id = bid.0;
    let parsed = tuple_list(
        spans,
        "set-extra-highlights! spans",
        3..=3,
        "(start end scope)",
        |fields| {
            Ok((
                usize_arg(fields[0].clone(), "set-extra-highlights! start")?,
                usize_arg(fields[1].clone(), "set-extra-highlights! end")?,
                string_arg(fields[2].clone(), "set-extra-highlights! scope")?,
            ))
        },
    )?;
    require_cap(ctx.host.decorations(), "set-extra-highlights!")?
        .set_extra_highlights(source, id, parsed);
    Ok(SteelVal::Void)
}

/// `(%diagnostics-for-buffer bid severity range)` — the `diagnostics-for-buffer`
/// Scheme wrapper supplies `#:severity`/`#:range` defaults. `severity`: a
/// symbol or `#f`. `range`: a `(start . end)` dotted pair or `#f`.
pub(crate) fn diagnostics_for_buffer(
    ctx: &mut SteelCtx,
    bid: BidArg,
    severity: SteelVal,
    range: SteelVal,
) -> SteelResult {
    let id = bid.0;
    let floor = match severity {
        SteelVal::BoolV(false) => None,
        SteelVal::SymbolV(s) => Some(s.to_string()),
        _ => {
            steel::stop!(TypeMismatch => "diagnostics-for-buffer: #:severity expected a symbol or #f")
        }
    };
    let range = match range {
        SteelVal::BoolV(false) => None,
        other => {
            let (start, end) = pair_fields(other, "diagnostics-for-buffer", "(start . end)")?;
            let start = usize_arg(start, "diagnostics-for-buffer range start")?;
            let end = usize_arg(end, "diagnostics-for-buffer range end")?;
            Some((start, end))
        }
    };
    let entries = match ctx.host.decorations() {
        Some(decorations) => decorations
            .diagnostics_for_buffer(id, floor.as_deref(), range)
            .map_err(|e| generic_err(format!("diagnostics-for-buffer: {e}")))?,
        None => Vec::new(),
    };
    let list: Vec<SteelVal> = entries.iter().map(json_to_steel).collect();
    Ok(SteelVal::ListV(list.into()))
}

/// `(diagnostic-counts bid)` → `(errors . warnings)` dotted pair.
pub(crate) fn diagnostic_counts(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    let (errors, warnings) = ctx
        .host
        .decorations()
        .map(|d| d.diagnostic_counts(id))
        .unwrap_or((0, 0));
    cons_pair(
        SteelVal::IntV(errors as isize),
        SteelVal::IntV(warnings as isize),
    )
}

#[cfg(test)]
mod tests;
