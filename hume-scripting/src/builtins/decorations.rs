//! Decoration stores (inlay hints, signs, virtual lines, extra highlights)
//! and the diagnostics pull API. Not LSP-specific — any Steel plugin can
//! populate these — but LSP is the first and heaviest client.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::json::{json_to_steel, steel_to_json};

use super::args::{BidArg, checked_fields, int_arg, string_arg, tuple_list, usize_arg};
use super::errors::generic_err;

type SteelResult = Result<SteelVal, SteelErr>;

/// `(set-inlay-hints! bid hints)` — `hints`: list of `(position text
/// 'before|'after)`, `position` a wire `{"line" "character"}` hashmap.
pub(crate) fn set_inlay_hints(ctx: &mut SteelCtx, bid: BidArg, hints: SteelVal) -> SteelResult {
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
    if let Some(decorations) = ctx.host.decorations() {
        decorations.set_inlay_hints(id, parsed);
    }
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
    if let Some(decorations) = ctx.host.decorations() {
        decorations.set_signs(source, id, parsed);
    }
    Ok(SteelVal::Void)
}

/// `(set-virtual-lines! source bid lines)` — `lines`: list of `(line text)`
/// or `(line text scope)`.
pub(crate) fn set_virtual_lines(
    ctx: &mut SteelCtx,
    source: SteelVal,
    bid: BidArg,
    lines: SteelVal,
) -> SteelResult {
    let source = string_arg(source, "set-virtual-lines! source")?;
    let id = bid.0;
    let parsed = tuple_list(
        lines,
        "set-virtual-lines! lines",
        2..=3,
        "(line text) or (line text scope)",
        |fields| {
            let line = usize_arg(fields[0].clone(), "set-virtual-lines! line")?;
            let text = string_arg(fields[1].clone(), "set-virtual-lines! text")?;
            let scope = fields
                .get(2)
                .map(|v| string_arg(v.clone(), "set-virtual-lines! scope"))
                .transpose()?;
            Ok((line, text, scope))
        },
    )?;
    if let Some(decorations) = ctx.host.decorations() {
        decorations.set_virtual_lines(source, id, parsed);
    }
    Ok(SteelVal::Void)
}

/// `(set-inline-diagnostics! bid lines)` — `lines`: list of `(line text
/// scope)`, one owner per buffer (no `source` arg, unlike
/// `set-virtual-lines!` — the diagnostics plugin is the only client).
pub(crate) fn set_inline_diagnostics(
    ctx: &mut SteelCtx,
    bid: BidArg,
    lines: SteelVal,
) -> SteelResult {
    let id = bid.0;
    let parsed = tuple_list(
        lines,
        "set-inline-diagnostics! lines",
        3..=3,
        "(line text scope)",
        |fields| {
            Ok((
                usize_arg(fields[0].clone(), "set-inline-diagnostics! line")?,
                string_arg(fields[1].clone(), "set-inline-diagnostics! text")?,
                string_arg(fields[2].clone(), "set-inline-diagnostics! scope")?,
            ))
        },
    )?;
    if let Some(decorations) = ctx.host.decorations() {
        decorations.set_inline_diagnostics(id, parsed);
    }
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
    if let Some(decorations) = ctx.host.decorations() {
        decorations.set_extra_highlights(source, id, parsed);
    }
    Ok(SteelVal::Void)
}

/// `(%diagnostics-for-buffer bid severity range)` — the `diagnostics-for-buffer`
/// Scheme wrapper supplies `#:severity`/`#:range` defaults. `severity`: a
/// symbol or `#f`. `range`: a 2-element list `(start end)` or `#f` — a
/// dotted pair isn't usable here (steel-core 0.8.2's `Pair`/`car`/`cdr` are
/// crate-private, so a Rust builtin can't destructure one).
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
        SteelVal::StringV(s) => Some(s.to_string()),
        _ => {
            steel::stop!(TypeMismatch => "diagnostics-for-buffer: #:severity expected a symbol or #f")
        }
    };
    let range = match range {
        SteelVal::BoolV(false) => None,
        other => {
            let fields = checked_fields(other, "diagnostics-for-buffer", 2..=2, "(start end)")?;
            let start = usize_arg(fields[0].clone(), "diagnostics-for-buffer range start")?;
            let end = usize_arg(fields[1].clone(), "diagnostics-for-buffer range end")?;
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

/// `(diagnostic-counts bid)` → `(errors . warnings)` — a genuine dotted
/// pair, built via steel-core's public `cons` (the only public pair API).
pub(crate) fn diagnostic_counts(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    let (errors, warnings) = ctx
        .host
        .decorations()
        .map(|d| d.diagnostic_counts(id))
        .unwrap_or((0, 0));
    let mut errors_val = SteelVal::IntV(errors as isize);
    let mut warnings_val = SteelVal::IntV(warnings as isize);
    steel::primitives::lists::cons(&mut errors_val, &mut warnings_val)
}
