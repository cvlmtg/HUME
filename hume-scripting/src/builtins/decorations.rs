//! Decoration stores (inlay hints, signs, virtual lines, extra highlights)
//! and the diagnostics pull API. Not LSP-specific — any Steel plugin can
//! populate these — but LSP is the first and heaviest client.

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
    require_cap(ctx.host.decorations(), "set-inlay-hints!")?.set_inlay_hints(id, parsed);
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
/// fallback when absent), and `'segments` (list of `(start end scope)` byte
/// ranges into `text`, styling only the covered bytes; bytes outside every
/// segment keep `'scope`'s style).
pub(crate) fn set_virtual_lines(
    ctx: &mut SteelCtx,
    source: SteelVal,
    bid: BidArg,
    lines: SteelVal,
) -> SteelResult {
    let source = string_arg(source, "set-virtual-lines! source")?;
    let id = bid.0;
    let parsed = virtual_line_specs(lines, "set-virtual-lines!")?;
    require_cap(ctx.host.decorations(), "set-virtual-lines!")?
        .set_virtual_lines(source, id, parsed);
    Ok(SteelVal::Void)
}

const VIRTUAL_LINE_KEYS: &[&str] = &["line", "text", "anchor", "scope", "segments"];

/// Decodes `lines` into `VirtualLineSpec`s. Each entry is a hashmap, not the
/// old positional `(line text scope)` list this replaces — free to break: no
/// `.scm` plugin calls this builtin yet, only Rust tests. Guarantees the
/// contract `VirtualLineSpec`'s doc promises (segments sorted, non-overlapping,
/// non-empty, in-bounds, char-boundary-aligned), so nothing downstream
/// re-validates.
fn virtual_line_specs(lines: SteelVal, name: &str) -> Result<Vec<VirtualLineSpec>, SteelErr> {
    list_items(lines, &format!("{name} lines"))?
        .into_iter()
        .map(|entry| virtual_line_spec(entry, name))
        .collect()
}

fn virtual_line_spec(entry: SteelVal, name: &str) -> Result<VirtualLineSpec, SteelErr> {
    let SteelVal::HashMapV(map) = &entry else {
        steel::stop!(TypeMismatch =>
            "{}: each entry must be a hashmap with 'line and 'text keys \
             (plus optional 'anchor/'scope/'segments)", name);
    };
    for (key, _) in map.iter() {
        let SteelVal::SymbolV(key_name) = key else {
            steel::stop!(Generic => "{}: hashmap key must be a symbol, got {:?}", name, key);
        };
        if !VIRTUAL_LINE_KEYS.contains(&key_name.as_str()) {
            steel::stop!(Generic =>
                "{}: unknown key '{}, expected one of {:?}", name, key_name, VIRTUAL_LINE_KEYS);
        }
    }
    let field = |k: &str| map.get(&SteelVal::SymbolV(k.into())).cloned();

    let line = field("line").ok_or_else(|| generic_err(format!("{name}: missing 'line")))?;
    let line = usize_arg(line, &format!("{name} line"))?;

    let text = field("text").ok_or_else(|| generic_err(format!("{name}: missing 'text")))?;
    let text = string_arg(text, &format!("{name} text"))?;

    let before = match field("anchor") {
        None => false,
        Some(SteelVal::SymbolV(s)) if s.as_str() == "before" => true,
        Some(SteelVal::SymbolV(s)) if s.as_str() == "after" => false,
        Some(_) => steel::stop!(Generic => "{}: 'anchor must be 'before or 'after", name),
    };

    let scope = field("scope")
        .map(|v| string_arg(v, &format!("{name} scope")))
        .transpose()?;

    let segments = match field("segments") {
        None => Vec::new(),
        Some(v) => virtual_line_segments(v, &text, name)?,
    };

    Ok(VirtualLineSpec {
        line,
        text,
        before,
        scope,
        segments,
    })
}

/// Decodes and validates `'segments`: each a `(start end scope)` byte range
/// into `text`. Sorts by `start`, then checks in one pass — in-bounds,
/// char-boundary-aligned, non-empty, non-overlapping — the exact invariant
/// `VirtualLineSpec::segments` documents.
fn virtual_line_segments(
    segments: SteelVal,
    text: &str,
    name: &str,
) -> Result<Vec<(usize, usize, String)>, SteelErr> {
    let mut segments = tuple_list(
        segments,
        &format!("{name} segments"),
        3..=3,
        "(start end scope)",
        |fields| {
            let start = usize_arg(fields[0].clone(), &format!("{name} segment start"))?;
            let end = usize_arg(fields[1].clone(), &format!("{name} segment end"))?;
            let scope = string_arg(fields[2].clone(), &format!("{name} segment scope"))?;
            Ok((start, end, scope))
        },
    )?;
    segments.sort_by_key(|(start, _, _)| *start);

    let mut prev_end = 0usize;
    for (start, end, _) in &segments {
        if start >= end {
            steel::stop!(Generic =>
                "{} segments: segment ({}, {}) must have start < end", name, start, end);
        }
        if *end > text.len() {
            steel::stop!(Generic =>
                "{} segments: segment end {} is past text's byte length {}", name, end, text.len());
        }
        if !text.is_char_boundary(*start) || !text.is_char_boundary(*end) {
            steel::stop!(Generic =>
                "{} segments: segment ({}, {}) is not aligned to a char boundary in text",
                name, start, end);
        }
        if *start < prev_end {
            steel::stop!(Generic =>
                "{} segments: segments must not overlap (segment starting at {} \
                 overlaps the previous one ending at {})", name, start, prev_end);
        }
        prev_end = *end;
    }

    Ok(segments)
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
    require_cap(ctx.host.decorations(), "set-inline-diagnostics!")?
        .set_inline_diagnostics(id, parsed);
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
