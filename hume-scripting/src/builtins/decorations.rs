//! Decoration stores (inlay hints, signs, virtual lines, EOL text, extra
//! highlights, line backgrounds) and the diagnostics pull API. Not
//! LSP-specific — any Steel plugin can populate these — but LSP is the first
//! and heaviest client.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::json::json_to_steel;
use crate::types::VirtualLineSpec;

use super::SteelResult;
use super::args::{
    BidArg, cons_pair, int_arg, list_items, pair_fields, string_arg, tuple_list, usize_arg,
};
use super::errors::{generic_err, require_cap};

/// `(set-inlay-hints! source bid hints)` — `hints`: list of `(offset text
/// 'before|'after)`, `offset` a char offset. LSP wire `{"line"
/// "character"}` positions convert via `lsp-position->offset` before
/// reaching this builtin — the Steel decoration surface speaks editor-native
/// units only, so a caller never needs to know which server's encoding a
/// wire position came in.
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
        "(offset text 'before|'after)",
        |fields| {
            let pos = usize_arg(fields[0].clone(), "set-inlay-hints! offset")?;
            let text = string_arg(fields[1].clone(), "set-inlay-hints! text")?;
            let before = match &fields[2] {
                SteelVal::SymbolV(s) if s.as_str() == "before" => true,
                SteelVal::SymbolV(s) if s.as_str() == "after" => false,
                _ => {
                    steel::stop!(Generic => "set-inlay-hints!: third element must be 'before or 'after")
                }
            };
            Ok((pos, text, before))
        },
    )?;
    require_cap(ctx.host.decorations(), "set-inlay-hints!")?
        .set_inlay_hints(source, id, parsed)
        .map_err(generic_err)?;
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
            let text = string_arg(fields[1].clone(), "set-signs! text")?;
            // A sign is a glyph in a fixed-width gutter lane: no control
            // character has a meaning there, and one would misalign the lane
            // rather than render. The gutter measures the text to right-align
            // it but writes it with a terminal-buffer writer that drops what
            // it can't draw, so a tab would reserve columns that then stay
            // blank and push the padding off. Rejected outright rather than
            // substituted — unlike `set-virtual-lines!`, which maps them to
            // spaces because its callers' `'segments` offsets have to keep
            // lining up with the text.
            if text.contains(char::is_control) {
                steel::stop!(Generic =>
                    "set-signs!: 'text must not contain a control character, got {:?}", text);
            }
            Ok((
                usize_arg(fields[0].clone(), "set-signs! line")?,
                text,
                string_arg(fields[2].clone(), "set-signs! scope")?,
                int_arg(fields[3].clone(), "set-signs! priority")?,
            ))
        },
    )?;
    require_cap(ctx.host.decorations(), "set-signs!")?
        .set_signs(source, id, parsed)
        .map_err(generic_err)?;
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
    require_cap(ctx.host.decorations(), "set-virtual-lines!")?
        .set_virtual_lines(source, id, parsed)
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

const VIRTUAL_LINE_KEYS: &[&str] = &["line", "text", "anchor", "scope", "segments"];

/// Decodes `lines` into `VirtualLineSpec`s. Each entry is a hashmap, shaped
/// to match `set-virtual-lines!`'s contract with its first real caller, the
/// git-diff plugin. Only decodes shape (arity, types) —
/// segment bounds/ordering/overlap validation happens at the host boundary
/// (`host_impl.rs`'s `set_virtual_lines`), the sole enforcement point for
/// that contract.
fn virtual_line_specs(lines: SteelVal) -> Result<Vec<VirtualLineSpec>, SteelErr> {
    list_items(lines, "set-virtual-lines! lines")?
        .into_iter()
        .map(virtual_line_spec)
        .collect()
}

fn virtual_line_spec(entry: SteelVal) -> Result<VirtualLineSpec, SteelErr> {
    let SteelVal::HashMapV(map) = &entry else {
        steel::stop!(TypeMismatch =>
            "set-virtual-lines!: each entry must be a hashmap with 'line and 'text keys \
             (plus optional 'anchor/'scope/'segments)");
    };
    for (key, _) in map.iter() {
        let SteelVal::SymbolV(key_name) = key else {
            steel::stop!(Generic =>
                "set-virtual-lines!: hashmap key must be a symbol, got {:?}", key);
        };
        if !VIRTUAL_LINE_KEYS.contains(&key_name.as_str()) {
            steel::stop!(Generic =>
                "set-virtual-lines!: unknown key '{}, expected one of {:?}",
                key_name, VIRTUAL_LINE_KEYS);
        }
    }
    let field = |k: &str| map.get(&SteelVal::SymbolV(k.into())).cloned();

    let line = field("line").ok_or_else(|| generic_err("set-virtual-lines!: missing 'line"))?;
    let line = usize_arg(line, "set-virtual-lines! line")?;

    let text = field("text").ok_or_else(|| generic_err("set-virtual-lines!: missing 'text"))?;
    let text = string_arg(text, "set-virtual-lines! text")?;
    if text.contains(['\n', '\r']) {
        steel::stop!(Generic =>
            "set-virtual-lines!: 'text must not contain a newline — virtual lines render as a \
             single row");
    }
    // A tab renders like a real buffer line's tab — the engine expands it to
    // the next tab stop (`hume_engine::rows::segment_virtual_row`), so
    // callers no longer need to expand it themselves. Any other unrenderable
    // character (a control character, an invisible one) is left verbatim:
    // `push_virtual_cells` substitutes it with its codepoint placeholder,
    // the same chokepoint every other text source goes through — a
    // char-for-char blank here would be a second, weaker copy of that
    // policy, and one that hides exactly what the codepoint substitution
    // exists to surface (a bidi override rendering like a space, say).
    // Leaving `text` untouched also keeps a caller's `'segments` offsets
    // (validated below) trivially aligned with it.

    let before = match field("anchor") {
        None => false,
        Some(SteelVal::SymbolV(s)) if s.as_str() == "before" => true,
        Some(SteelVal::SymbolV(s)) if s.as_str() == "after" => false,
        Some(_) => steel::stop!(Generic =>
            "set-virtual-lines!: 'anchor must be 'before or 'after"),
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
    require_cap(ctx.host.decorations(), "set-eol-text!")?
        .set_eol_text(source, id, parsed)
        .map_err(generic_err)?;
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
        .set_extra_highlights(source, id, parsed)
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

/// `(set-line-backgrounds! source bid entries)` — `entries`: list of `(line
/// scope)`. A full-row background tint on each named line. No `priority`
/// field — unlike signs, row tints have no single-slot contention; same-line
/// entries from different sources break ties by source name.
pub(crate) fn set_line_backgrounds(
    ctx: &mut SteelCtx,
    source: SteelVal,
    bid: BidArg,
    entries: SteelVal,
) -> SteelResult {
    let source = string_arg(source, "set-line-backgrounds! source")?;
    let id = bid.0;
    let parsed = tuple_list(
        entries,
        "set-line-backgrounds! entries",
        2..=2,
        "(line scope)",
        |fields| {
            Ok((
                usize_arg(fields[0].clone(), "set-line-backgrounds! line")?,
                string_arg(fields[1].clone(), "set-line-backgrounds! scope")?,
            ))
        },
    )?;
    require_cap(ctx.host.decorations(), "set-line-backgrounds!")?
        .set_line_backgrounds(source, id, parsed)
        .map_err(generic_err)?;
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
