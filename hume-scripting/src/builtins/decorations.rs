//! Decoration stores (inlay hints, signs, virtual lines, EOL text, extra
//! highlights, line backgrounds, statusline text) and the diagnostics pull
//! API. Not LSP-specific (any Steel plugin can populate these), but LSP is
//! the first and heaviest client.

use steel::HashMap as SteelHashMap;
use steel::gc::Gc;
use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::host::DiagnosticEntry;
use crate::json::{WireOrigin, to_steel_handle};
use crate::types::{PaneHandle, VirtualLineSpec};

use super::SteelResult;
use super::args::{
    ArgPane, HashEntry, cons_pair, hash_list, int_arg, optional_pair_fields, optional_symbol_arg,
    string_arg, symbol_enum_arg, usize_arg,
};
use super::errors::{generic_err, require_cap};

/// `(set-inlay-hints! source pane hints)`: `hints`: list of `(hash 'offset
/// 'text 'side)`, `offset` a char offset, `side` `'before` or `'after`. LSP wire `{"line"
/// "character"}` positions convert via `lsp-position->offset` before
/// reaching this builtin. The Steel decoration surface speaks editor-native
/// units only, so a caller never needs to know which server's encoding a
/// wire position came in.
pub(crate) fn set_inlay_hints(
    ctx: &mut SteelCtx,
    source: SteelVal,
    pane: PaneHandle,
    hints: SteelVal,
) -> SteelResult {
    let bid = pane.buffer();
    let source = string_arg(source, "set-inlay-hints! source")?;
    let parsed = hash_list(
        hints,
        "set-inlay-hints!",
        &["offset", "text", "side"],
        |entry| {
            let pos = usize_arg(entry.required("offset")?, "set-inlay-hints! 'offset")?;
            let text = string_arg(entry.required("text")?, "set-inlay-hints! 'text")?;
            let before = side_arg(entry.required("side")?, "set-inlay-hints! 'side")?;
            Ok((pos, text, before))
        },
    )?;
    require_cap(ctx.host.decorations(), "set-inlay-hints!")?
        .set_inlay_hints(source, bid, parsed)
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

/// `(register-sign-source! name pane priority)`: declares a sign channel
/// *for `pane`'s buffer*: its gutter slot is this call's rank among every source
/// registered for that same buffer, by `(priority desc, name asc)`, a
/// property of the *source*, not of any one `set-signs!` call, and scoped to
/// that buffer, not shared with any other. A slot is reserved the first time
/// a source registers for a buffer, even before it's placed any signs there.
/// This is what keeps the gutter width stable as signs come and go,
/// instead of tracking whichever priorities happen to be live right now.
/// There is no `unregister-sign-source!`: a source holds its slot in a
/// buffer for that buffer's life. Re-registering `name` for the same buffer
/// replaces its priority and re-sorts it, same as `register-lsp-server!`'s
/// last-wins semantics.
pub(crate) fn register_sign_source(
    ctx: &mut SteelCtx,
    name: SteelVal,
    pane: PaneHandle,
    priority: SteelVal,
) -> SteelResult {
    let bid = pane.buffer();
    let name = string_arg(name, "register-sign-source! name")?;
    if name.trim().is_empty() {
        steel::stop!(Generic => "register-sign-source!: name must not be empty");
    }
    let priority = int_arg(priority, "register-sign-source! priority")?;
    require_cap(ctx.host.decorations(), "register-sign-source!")?
        .register_sign_source(name, bid, priority)
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

/// `(set-signs! source pane signs)`: `signs`: list of `(hash 'line 'text 'scope)`.
/// `source` selects the already-registered channel (see
/// `register-sign-source!`) whose slot every entry here renders in; an
/// unregistered `source` errors rather than being silently dropped.
pub(crate) fn set_signs(
    ctx: &mut SteelCtx,
    source: SteelVal,
    pane: PaneHandle,
    signs: SteelVal,
) -> SteelResult {
    let bid = pane.buffer();
    let source = string_arg(source, "set-signs! source")?;
    let parsed = hash_list(signs, "set-signs!", LINE_TEXT_SCOPE, |entry| {
        let text = string_arg(entry.required("text")?, "set-signs! 'text")?;
        // A sign is a glyph in a fixed-width gutter lane: no control
        // character has a meaning there, and one would misalign the lane
        // rather than render. The gutter measures the text to right-align
        // it but writes it with a terminal-buffer writer that drops what
        // it can't draw, so a tab would reserve columns that then stay
        // blank and push the padding off. Rejected outright rather than
        // substituted, unlike `set-virtual-lines!`, which maps them to
        // spaces because its callers' `'segments` offsets have to keep
        // lining up with the text.
        if text.contains(char::is_control) {
            steel::stop!(Generic =>
                    "set-signs!: 'text must not contain a control character, got {:?}", text);
        }
        Ok((
            usize_arg(entry.required("line")?, "set-signs! 'line")?,
            text,
            string_arg(entry.required("scope")?, "set-signs! 'scope")?,
        ))
    })?;
    require_cap(ctx.host.decorations(), "set-signs!")?
        .set_signs(source, bid, parsed)
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

/// `(set-virtual-lines! source pane lines)`: `lines`: list of hashmaps, each
/// with required `'line`/`'text`, plus optional `'anchor` (`'before` or
/// `'after`, default `'after`), `'scope` (whole-line base style, `ui.virtual`
/// fallback when absent), and `'segments` (list of `(hash 'start 'end
/// 'scope)` char ranges into `text`, styling only the covered chars; chars outside every
/// segment keep `'scope`'s style). Segment bounds/ordering/overlap are
/// validated at the host boundary, not here (see `VirtualLineSpec::segments`).
pub(crate) fn set_virtual_lines(
    ctx: &mut SteelCtx,
    source: SteelVal,
    pane: PaneHandle,
    lines: SteelVal,
) -> SteelResult {
    let bid = pane.buffer();
    let source = string_arg(source, "set-virtual-lines! source")?;
    let parsed = virtual_line_specs(lines)?;
    require_cap(ctx.host.decorations(), "set-virtual-lines!")?
        .set_virtual_lines(source, bid, parsed)
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

/// Decodes `lines` into `VirtualLineSpec`s. Only decodes shape (keys,
/// types): segment bounds/ordering/overlap validation happens at the host
/// boundary (`host_impl.rs`'s `set_virtual_lines`), the sole enforcement
/// point for that contract.
fn virtual_line_specs(lines: SteelVal) -> Result<Vec<VirtualLineSpec>, SteelErr> {
    hash_list(
        lines,
        "set-virtual-lines!",
        &["line", "text", "anchor", "scope", "segments"],
        virtual_line_spec,
    )
}

fn virtual_line_spec(entry: &HashEntry) -> Result<VirtualLineSpec, SteelErr> {
    let line = usize_arg(entry.required("line")?, "set-virtual-lines! 'line")?;
    let text = string_arg(entry.required("text")?, "set-virtual-lines! 'text")?;
    if text.contains(['\n', '\r']) {
        steel::stop!(Generic =>
            "set-virtual-lines!: 'text must not contain a newline (virtual lines render as a \
             single line)");
    }
    // A tab renders like a real buffer line's tab: the engine expands it to
    // the next tab stop (`hume_engine::display_lines::segment_virtual_line`), so
    // callers pass it through unexpanded. Any other unrenderable character
    // (a control character, an invisible one) is left verbatim:
    // `push_virtual_cells` substitutes it with its codepoint placeholder,
    // the same chokepoint every other text source goes through. A
    // char-for-char blank here would be a second, weaker copy of that
    // policy, and one that hides exactly what the codepoint substitution
    // exists to surface (a bidi override rendering like a space, say).
    // Leaving `text` untouched also keeps a caller's `'segments` offsets
    // (validated below) trivially aligned with it.

    let before = match entry.optional("anchor") {
        None => false,
        Some(v) => side_arg(v, "set-virtual-lines! 'anchor")?,
    };

    let scope = entry
        .optional("scope")
        .map(|v| string_arg(v, "set-virtual-lines! 'scope"))
        .transpose()?;

    let segments = match entry.optional("segments") {
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

/// Decodes `'segments`: each a `(hash 'start 'end 'scope)` char range into
/// `text`. Shape only (keys, types): bounds, ordering, overlap, and
/// grapheme-cluster alignment are validated at the host boundary
/// (`host_impl.rs`'s `set_virtual_lines`), which also converts these char
/// offsets to the byte offsets the engine needs.
fn virtual_line_segments(segments: SteelVal) -> Result<Vec<(usize, usize, String)>, SteelErr> {
    hash_list(
        segments,
        "set-virtual-lines! 'segments",
        START_END_SCOPE,
        |entry| {
            Ok((
                usize_arg(
                    entry.required("start")?,
                    "set-virtual-lines! segment 'start",
                )?,
                usize_arg(entry.required("end")?, "set-virtual-lines! segment 'end")?,
                string_arg(
                    entry.required("scope")?,
                    "set-virtual-lines! segment 'scope",
                )?,
            ))
        },
    )
}

/// An inlay hint's `'side` or a virtual line's `'anchor`: `'before` → `true`,
/// `'after` → `false`.
fn side_arg(val: SteelVal, ctx_name: &str) -> Result<bool, SteelErr> {
    let SteelVal::SymbolV(s) = val else {
        steel::stop!(Generic => "{}: must be a symbol, 'before or 'after", ctx_name);
    };
    symbol_enum_arg(&s, ctx_name, &[("before", true), ("after", false)])
}

const LINE_TEXT_SCOPE: &[&str] = &["line", "text", "scope"];
const START_END_SCOPE: &[&str] = &["start", "end", "scope"];

/// `(set-eol-text! source pane lines)`: `lines`: list of `(hash 'line 'text
/// 'scope)`. Not diagnostics-specific: the diagnostics plugin is its first
/// client, not its owner, same as every other decoration kind is to LSP.
pub(crate) fn set_eol_text(
    ctx: &mut SteelCtx,
    source: SteelVal,
    pane: PaneHandle,
    lines: SteelVal,
) -> SteelResult {
    let bid = pane.buffer();
    let source = string_arg(source, "set-eol-text! source")?;
    let parsed = hash_list(lines, "set-eol-text!", LINE_TEXT_SCOPE, |entry| {
        Ok((
            usize_arg(entry.required("line")?, "set-eol-text! 'line")?,
            string_arg(entry.required("text")?, "set-eol-text! 'text")?,
            string_arg(entry.required("scope")?, "set-eol-text! 'scope")?,
        ))
    })?;
    require_cap(ctx.host.decorations(), "set-eol-text!")?
        .set_eol_text(source, bid, parsed)
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

/// `(set-extra-highlights! source pane spans)`: `spans`: list of `(hash
/// 'start 'end 'scope)`, char offsets.
pub(crate) fn set_extra_highlights(
    ctx: &mut SteelCtx,
    source: SteelVal,
    pane: PaneHandle,
    spans: SteelVal,
) -> SteelResult {
    let bid = pane.buffer();
    let source = string_arg(source, "set-extra-highlights! source")?;
    let parsed = hash_list(spans, "set-extra-highlights!", START_END_SCOPE, |entry| {
        Ok((
            usize_arg(entry.required("start")?, "set-extra-highlights! 'start")?,
            usize_arg(entry.required("end")?, "set-extra-highlights! 'end")?,
            string_arg(entry.required("scope")?, "set-extra-highlights! 'scope")?,
        ))
    })?;
    require_cap(ctx.host.decorations(), "set-extra-highlights!")?
        .set_extra_highlights(source, bid, parsed)
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

/// `(set-line-backgrounds! source pane entries)`: `entries`: list of `(hash
/// 'line 'scope)`. A full-row background tint on each named line. No `priority`
/// field: unlike signs, row tints have no single-slot contention; same-line
/// entries from different sources break ties by source name.
pub(crate) fn set_line_backgrounds(
    ctx: &mut SteelCtx,
    source: SteelVal,
    pane: PaneHandle,
    entries: SteelVal,
) -> SteelResult {
    let bid = pane.buffer();
    let source = string_arg(source, "set-line-backgrounds! source")?;
    let parsed = hash_list(
        entries,
        "set-line-backgrounds!",
        &["line", "scope"],
        |entry| {
            Ok((
                usize_arg(entry.required("line")?, "set-line-backgrounds! 'line")?,
                string_arg(entry.required("scope")?, "set-line-backgrounds! 'scope")?,
            ))
        },
    )?;
    require_cap(ctx.host.decorations(), "set-line-backgrounds!")?
        .set_line_backgrounds(source, bid, parsed)
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

/// `(set-statusline-text! source pane text)`: replaces `source`'s
/// statusline text for `pane`'s buffer wholesale. Rendered by the `steel:<source>`
/// statusline element (see `configure-statusline!`). Placing it is a
/// separate step, this only pushes the value a placed element will show.
pub(crate) fn set_statusline_text(
    ctx: &mut SteelCtx,
    source: SteelVal,
    pane: PaneHandle,
    text: SteelVal,
) -> SteelResult {
    let bid = pane.buffer();
    let source = string_arg(source, "set-statusline-text! source")?;
    let text = string_arg(text, "set-statusline-text! text")?;
    require_cap(ctx.host.decorations(), "set-statusline-text!")?
        .set_statusline_text(source, bid, text)
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

/// `(%diagnostics-for-buffer pane severity range)`: the `diagnostics-for-buffer`
/// Scheme wrapper supplies `#:severity`/`#:range` defaults. `severity`: a
/// symbol or `#f`. `range`: a `(start . end)` dotted pair or `#f`.
pub(crate) fn diagnostics_for_buffer(
    ctx: &mut SteelCtx,
    pane: ArgPane,
    severity: SteelVal,
    range: SteelVal,
) -> SteelResult {
    let id = pane.0.buffer();
    let floor = optional_symbol_arg(severity, "diagnostics-for-buffer #:severity")?;
    let range = optional_pair_fields(range, "diagnostics-for-buffer", "(start . end)")?
        .map(|(start, end)| {
            let start = usize_arg(start, "diagnostics-for-buffer range start")?;
            let end = usize_arg(end, "diagnostics-for-buffer range end")?;
            Ok::<_, SteelErr>((start, end))
        })
        .transpose()?;
    let entries = match ctx.host.decorations() {
        Some(decorations) => decorations
            .diagnostics_for_buffer(id, floor.as_deref(), range)
            .map_err(|e| generic_err(format!("diagnostics-for-buffer: {e}")))?,
        None => Vec::new(),
    };
    let list: Vec<SteelVal> = entries.into_iter().map(diagnostic_entry_to_steel).collect();
    Ok(SteelVal::ListV(list.into()))
}

/// `DiagnosticEntry` -> a symbol-keyed Steel hashmap, field-by-field native
/// except `'raw`, the one field that crosses as a `JsonHandle` sharing the
/// entry's own `Arc` rather than a value rebuilt (and reconverted) just to
/// carry it. Written by hand rather than `json_to_steel` on a
/// `serde_json::Value` blob precisely so `'raw` can take that different
/// path from every other field.
fn diagnostic_entry_to_steel(entry: DiagnosticEntry) -> SteelVal {
    let mut hm = SteelHashMap::new();
    let mut insert = |k: &'static str, v: SteelVal| {
        hm.insert(SteelVal::SymbolV(k.into()), v);
    };
    insert("start", SteelVal::IntV(entry.start as isize));
    insert("end", SteelVal::IntV(entry.end as isize));
    insert("line", SteelVal::IntV(entry.line as isize));
    insert("end-line", SteelVal::IntV(entry.end_line as isize));
    insert("char-col", SteelVal::IntV(entry.char_col as isize));
    insert("grapheme-col", SteelVal::IntV(entry.grapheme_col as isize));
    insert("severity", SteelVal::SymbolV(entry.severity.into()));
    insert(
        "severity-rank",
        SteelVal::IntV(entry.severity_rank as isize),
    );
    insert("message", SteelVal::StringV(entry.message.into()));
    // `None` -> Void, matching json_to_steel's null mapping.
    insert(
        "code",
        entry
            .code
            .map_or(SteelVal::Void, |c| SteelVal::StringV(c.into())),
    );
    insert(
        "source",
        entry
            .source
            .map_or(SteelVal::Void, |s| SteelVal::StringV(s.into())),
    );
    insert(
        "raw",
        to_steel_handle(entry.raw, WireOrigin::Server(entry.encoding)),
    );
    SteelVal::HashMapV(Gc::new(hm).into())
}

/// `(diagnostic-counts pane)` → `(errors . warnings)` dotted pair.
pub(crate) fn diagnostic_counts(ctx: &mut SteelCtx, pane: ArgPane) -> SteelResult {
    let id = pane.0.buffer();
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
