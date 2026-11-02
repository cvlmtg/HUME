//! Decoration stores (inlay hints, signs, virtual lines, EOL text, extra
//! highlights, line backgrounds, statusline text) and the diagnostics pull
//! API. Not LSP-specific (any Steel plugin can populate these), but LSP is
//! the first and heaviest client.

use hume_core::{BufferId, DiagSeverity, Side};
use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::host::{DiagnosticEntry, DiagnosticRange, SetError};
use crate::json::{WireOrigin, to_steel_handle};
use crate::types::VirtualLineSpec;

use super::SteelResult;
use super::args::{
    ArgPane, CHAR_OFFSET, CHAR_RANGE_KEYS, END_CHAR, END_CHAR_COL, END_LINE, HashEntry, START_CHAR,
    START_CHAR_COL, START_LINE, bool_arg, hash_list, int_arg, item_label, optional_hash_entry,
    optional_symbol_arg, spanned_text, string_arg, symbol_enum_arg, symbol_hash,
};
use super::errors::{generic_err, require_cap};
use super::ids::DocRange;

/// `(set-inlay-hints! pane source hints)`: `hints`: list of `(hash 'char-offset
/// 'text 'side)`, `'char-offset` a char offset, `side` `'before` or `'after`. LSP wire `{"line"
/// "character"}` positions convert via `lsp-position->char-offset` before
/// reaching this builtin. The Steel decoration surface speaks editor-native
/// units only, so a caller never needs to know which server's encoding a
/// wire position came in.
pub(crate) fn set_inlay_hints(
    ctx: &mut SteelCtx,
    bid: BufferId,
    source: String,
    hints: SteelVal,
) -> SteelResult {
    let list = "hints";
    let parsed = hash_list(hints, list, &[CHAR_OFFSET, "text", "side"], |entry| {
        let pos = entry.usize(CHAR_OFFSET)?;
        let text = entry.string("text")?;
        let side = side_arg(entry.required("side")?, &entry.label("side"))?;
        Ok((pos, text, side))
    })?;
    require_cap(ctx.host.decorations())?
        .set_inlay_hints(source, bid, parsed)
        .map_err(set_err(list))?;
    Ok(SteelVal::Void)
}

/// `(register-sign-source! pane source priority)`: declares a sign channel
/// *for `pane`'s buffer*: its gutter slot is this call's rank among every source
/// registered for that same buffer, by `(priority desc, source asc)`, a
/// property of the *source*, not of any one `set-signs!` call, and scoped to
/// that buffer, not shared with any other. A slot is reserved the first time
/// a source registers for a buffer, even before it's placed any signs there.
/// This is what keeps the gutter width stable as signs come and go,
/// instead of tracking whichever priorities happen to be live right now.
/// There is no `unregister-sign-source!`: a source holds its slot in a
/// buffer for that buffer's life, across a reload or a replaced text, until
/// the buffer closes. Re-registering `source` for the same buffer
/// replaces its priority and re-sorts it, same as `register-lsp-server!`'s
/// last-wins semantics.
pub(crate) fn register_sign_source(
    ctx: &mut SteelCtx,
    bid: BufferId,
    source: String,
    priority: SteelVal,
) -> SteelResult {
    let priority = int_arg(priority, "priority")?;
    require_cap(ctx.host.decorations())?
        .register_sign_source(source, bid, priority)
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

/// `(set-signs! pane source signs)`: `signs`: list of `(hash 'line 'text 'scope)`.
/// `source` selects the already-registered channel (see
/// `register-sign-source!`) whose slot every entry here renders in; an
/// unregistered `source` errors rather than being silently dropped.
pub(crate) fn set_signs(
    ctx: &mut SteelCtx,
    bid: BufferId,
    source: String,
    signs: SteelVal,
) -> SteelResult {
    let list = "signs";
    let parsed = hash_list(signs, list, LINE_TEXT_SCOPE, |entry| {
        let text = entry.string("text")?;
        // A sign is a glyph in a fixed-width gutter lane, where no control
        // character has a meaning: the gutter would draw a tab as one blank
        // cell and any other control character as its codepoint
        // placeholder. `set-virtual-lines!` keeps them, because its line is
        // formatted like buffer text and a tab expands to the next tab stop.
        if text.contains(char::is_control) {
            steel::stop!(Generic =>
                "'text must not contain a control character, got {:?}", text);
        }
        Ok((entry.usize("line")?, text, entry.string("scope")?))
    })?;
    require_cap(ctx.host.decorations())?
        .set_signs(source, bid, parsed)
        .map_err(set_err(list))?;
    Ok(SteelVal::Void)
}

/// `(set-virtual-lines! pane source lines)`: `lines`: list of hashmaps, each
/// with required `'line`/`'text`, plus optional `'side` (`'before` or
/// `'after`, default `'after`), `'scope` (whole-line base style, `ui.virtual`
/// fallback when absent), and `'segments` (list of `(hash 'start-char-col
/// 'end-char-col 'scope)` char-column ranges in `text`, styling only the
/// covered chars; chars outside every segment keep `'scope`'s style).
pub(crate) fn set_virtual_lines(
    ctx: &mut SteelCtx,
    bid: BufferId,
    source: String,
    lines: SteelVal,
) -> SteelResult {
    let parsed = virtual_line_specs(lines)?;
    require_cap(ctx.host.decorations())?
        .set_virtual_lines(source, bid, parsed)
        .map_err(set_err(VIRTUAL_LINES))?;
    Ok(SteelVal::Void)
}

/// Decodes `lines` into `VirtualLineSpec`s, each entry's segments fitted to
/// its text. The host checks each `'line` against the buffer.
fn virtual_line_specs(lines: SteelVal) -> Result<Vec<VirtualLineSpec>, SteelErr> {
    hash_list(
        lines,
        VIRTUAL_LINES,
        &["line", "text", "side", "scope", "segments"],
        virtual_line_spec,
    )
}

fn virtual_line_spec(entry: &HashEntry) -> Result<VirtualLineSpec, SteelErr> {
    let line = entry.usize("line")?;
    let text = entry.string("text")?;
    if text.contains(['\n', '\r']) {
        steel::stop!(Generic =>
            "'text must not contain a newline (virtual lines render as a \
             single line)");
    }
    // A tab renders like a real buffer line's tab: the engine expands it to
    // the next tab stop (`hume_engine`'s `format_virtual_lines`), so
    // callers pass it through unexpanded. Any other unrenderable character
    // (a control character, an invisible one) is left verbatim:
    // `push_virtual_cells` substitutes it with its codepoint placeholder,
    // the same chokepoint every other text source goes through. A
    // char-for-char blank here would be a second, weaker copy of that
    // policy, and one that hides what the codepoint substitution
    // exists to surface (a bidi override rendering like a space, say).
    // Leaving `text` untouched also keeps a caller's `'segments` offsets
    // (fitted to it below) aligned with it.

    let side = entry
        .optional("side")
        .map_or(Ok(Side::After), |v| side_arg(v, &entry.label("side")))?;

    let scope = entry.optional_string("scope")?;

    let segments_label = entry.label("segments");
    let segments = match entry.optional("segments") {
        None => Vec::new(),
        Some(v) => virtual_line_segments(v, &segments_label)?,
    };

    Ok(VirtualLineSpec {
        line,
        content: spanned_text(text, segments, &segments_label)?,
        side,
        scope,
    })
}

/// Decodes `'segments`: each a `(hash 'start-char-col 'end-char-col 'scope)`
/// char-column range in `text`, decoded for shape only: [`spanned_text`]
/// fits them to `text`.
fn virtual_line_segments(
    segments: SteelVal,
    ctx_name: &str,
) -> Result<Vec<(usize, usize, String)>, SteelErr> {
    hash_list(
        segments,
        ctx_name,
        START_END_CHAR_COL_SCOPE,
        char_col_range_scope,
    )
}

/// A list of `(hash 'start-char s 'end-char e 'scope name)` spans, each as
/// `(s, e, name)`.
fn char_range_scopes(
    val: SteelVal,
    ctx_name: &str,
) -> Result<Vec<(usize, usize, String)>, SteelErr> {
    hash_list(val, ctx_name, START_END_CHAR_SCOPE, |entry| {
        let (start, end) = entry.char_range()?;
        Ok((start, end, entry.string("scope")?))
    })
}

/// One `(hash 'start-char-col 'end-char-col 'scope)` entry, decoded with
/// [`START_END_CHAR_COL_SCOPE`].
fn char_col_range_scope(entry: &HashEntry) -> Result<(usize, usize, String), SteelErr> {
    let (start, end) = entry.char_col_range()?;
    Ok((start, end, entry.string("scope")?))
}

/// An inlay hint's or a virtual line's `'side`.
fn side_arg(val: SteelVal, ctx_name: &str) -> Result<Side, SteelErr> {
    let SteelVal::SymbolV(s) = val else {
        steel::stop!(Generic => "{}: must be a symbol, 'before or 'after", ctx_name);
    };
    symbol_enum_arg(&s, ctx_name, &Side::NAMED)
}

const LINE_TEXT_SCOPE: &[&str] = &["line", "text", "scope"];

/// The list `set-virtual-lines!` takes, named in its errors.
const VIRTUAL_LINES: &str = "lines";

const START_END_CHAR_SCOPE: &[&str] = &[START_CHAR, END_CHAR, "scope"];
const START_END_CHAR_COL_SCOPE: &[&str] = &[START_CHAR_COL, END_CHAR_COL, "scope"];

/// A setter's [`SetError`] as the error it raises, an entry named by its
/// place in the list `list` labels.
fn set_err(list: &str) -> impl Fn(SetError) -> SteelErr + '_ {
    move |err| match err {
        SetError::Call(message) => generic_err(message),
        SetError::Entry { index, message } => {
            generic_err(format!("{}: {message}", item_label(list, index)))
        }
    }
}

/// `(set-eol-text! pane source lines #:hide-on-insert-line #f)`: `lines`: list
/// of `(hash 'line 'text 'scope)`. `#:hide-on-insert-line #t` hides every
/// entry on the primary cursor's line while the pane types in Insert mode.
/// `bootstrap.scm` defines the public keyword form over this `%` native. Not
/// diagnostics-specific: the diagnostics plugin is its first client, not its
/// owner, same as every other decoration kind is to LSP.
pub(crate) fn set_eol_text(
    ctx: &mut SteelCtx,
    bid: BufferId,
    source: String,
    lines: SteelVal,
    hide_on_insert_line: SteelVal,
) -> SteelResult {
    let list = "lines";
    let parsed = hash_list(lines, list, LINE_TEXT_SCOPE, |entry| {
        Ok((
            entry.usize("line")?,
            entry.string("text")?,
            entry.string("scope")?,
        ))
    })?;
    let hide_on_insert_line = bool_arg(hide_on_insert_line, "#:hide-on-insert-line")?;
    require_cap(ctx.host.decorations())?
        .set_eol_text(source, bid, parsed, hide_on_insert_line)
        .map_err(set_err(list))?;
    Ok(SteelVal::Void)
}

/// `(set-extra-highlights! pane source spans)`: `spans`: list of `(hash
/// 'start-char 'end-char 'scope)`, char offsets.
pub(crate) fn set_extra_highlights(
    ctx: &mut SteelCtx,
    bid: BufferId,
    source: String,
    spans: SteelVal,
) -> SteelResult {
    let list = "spans";
    let parsed = char_range_scopes(spans, list)?;
    require_cap(ctx.host.decorations())?
        .set_extra_highlights(source, bid, parsed)
        .map_err(set_err(list))?;
    Ok(SteelVal::Void)
}

/// `(set-line-backgrounds! pane source entries)`: `entries`: list of `(hash
/// 'line 'scope)`. A full-row background tint on each named line. No `priority`
/// field: unlike signs, row tints have no single-slot contention; same-line
/// entries from different sources break ties by source name.
pub(crate) fn set_line_backgrounds(
    ctx: &mut SteelCtx,
    bid: BufferId,
    source: String,
    entries: SteelVal,
) -> SteelResult {
    let list = "entries";
    let parsed = hash_list(entries, list, &["line", "scope"], |entry| {
        Ok((entry.usize("line")?, entry.string("scope")?))
    })?;
    require_cap(ctx.host.decorations())?
        .set_line_backgrounds(source, bid, parsed)
        .map_err(set_err(list))?;
    Ok(SteelVal::Void)
}

/// `(set-statusline-text! pane source text)`: replaces `source`'s
/// statusline text for `pane`'s buffer wholesale. Rendered by the `steel:<source>`
/// statusline element (see `configure-statusline!`, defined in the prelude). Placing it is a
/// separate step, this only pushes the value a placed element will show.
pub(crate) fn set_statusline_text(
    ctx: &mut SteelCtx,
    bid: BufferId,
    source: String,
    text: SteelVal,
) -> SteelResult {
    let text = string_arg(text, "text")?;
    require_cap(ctx.host.decorations())?
        .set_statusline_text(source, bid, text)
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

/// `(%diagnostics-for-buffer pane severity range)`: the `diagnostics-for-buffer`
/// Scheme wrapper supplies `#:severity`/`#:range` defaults. `severity`: a
/// symbol or `#f`. `range`: a `(hash 'start-char s 'end-char e)`, a request's
/// range (the `"range"` of `lsp-primary-range-params`), or `#f`.
pub(crate) fn diagnostics_for_buffer(
    ctx: &mut SteelCtx,
    pane: ArgPane,
    severity: SteelVal,
    range: SteelVal,
) -> SteelResult {
    let id = pane.0.buffer();
    let floor = severity_floor_arg(severity)?;
    let range = match DocRange::from_steel_val(&range) {
        Some(request) => Some(DiagnosticRange::Request(request)),
        None => optional_hash_entry(range, "#:range", CHAR_RANGE_KEYS)?
            .map(|range| range.char_range())
            .transpose()?
            .map(|(start, end)| DiagnosticRange::Chars { start, end }),
    };
    let entries = require_cap(ctx.host.decorations())?
        .diagnostics_for_buffer(id, floor, range)
        .map_err(generic_err)?;
    let list: Vec<SteelVal> = entries.into_iter().map(diagnostic_entry_to_steel).collect();
    Ok(SteelVal::ListV(list.into()))
}

/// `DiagnosticEntry` -> a symbol-keyed Steel hashmap, field-by-field native
/// except `'raw`, which crosses as a `JsonHandle` sharing the entry's own
/// `Arc` rather than a value rebuilt (and reconverted) just to carry it, and
/// `'server`, which is the publishing server's `ServerRef`. Written by hand
/// rather than `json_to_steel` on a `serde_json::Value` blob so those two
/// can take a different path from every other field.
fn diagnostic_entry_to_steel(entry: DiagnosticEntry) -> SteelVal {
    let optional_string =
        |v: Option<String>| v.map_or(SteelVal::Void, |v| SteelVal::StringV(v.into()));
    symbol_hash([
        (START_CHAR, SteelVal::IntV(entry.start_char as isize)),
        (END_CHAR, SteelVal::IntV(entry.end_char as isize)),
        (START_LINE, SteelVal::IntV(entry.start_line as isize)),
        (END_LINE, SteelVal::IntV(entry.end_line as isize)),
        ("grapheme-col", SteelVal::IntV(entry.grapheme_col as isize)),
        ("severity", SteelVal::SymbolV(entry.severity.name().into())),
        (
            "severity-rank",
            SteelVal::IntV(entry.severity.rank().into()),
        ),
        ("message", SteelVal::StringV(entry.message.into())),
        // `None` -> Void, matching json_to_steel's null mapping.
        ("code", optional_string(entry.code)),
        ("source", optional_string(entry.source)),
        (
            "raw",
            to_steel_handle(
                entry.raw,
                WireOrigin::Server {
                    id: entry.server.id,
                    encoding: entry.encoding,
                },
            ),
        ),
        ("server", entry.server.into_steel_val()),
    ])
}

/// `(diagnostic-glyph severity)`: the one-cell mark for a diagnostic
/// severity symbol, as the statusline shows it.
pub(crate) fn diagnostic_glyph(severity: SteelVal) -> SteelResult {
    let SteelVal::SymbolV(name) = severity else {
        steel::stop!(TypeMismatch => "severity: expected a symbol, got {}", severity);
    };
    let severity = symbol_enum_arg(&name, "severity", &DiagSeverity::NAMED)?;
    Ok(SteelVal::StringV(severity.glyph().into()))
}

/// A `#:severity` floor: a diagnostic severity symbol, or `#f` for the
/// `lsp-diagnostics-min-severity` default.
fn severity_floor_arg(severity: SteelVal) -> Result<Option<DiagSeverity>, SteelErr> {
    optional_symbol_arg(severity, "#:severity")?
        .map(|name| symbol_enum_arg(&name, "#:severity", &DiagSeverity::NAMED))
        .transpose()
}

/// `(%diagnostic-counts pane severity)` → `(hash 'error n 'warning n 'info n
/// 'hint n)`, keyed by each severity's name; `bootstrap.scm`'s
/// `diagnostic-counts` supplies the `#:severity` floor's `#f` default.
pub(crate) fn diagnostic_counts(
    ctx: &mut SteelCtx,
    pane: ArgPane,
    severity: SteelVal,
) -> SteelResult {
    let id = pane.0.buffer();
    let floor = severity_floor_arg(severity)?;
    let counts = require_cap(ctx.host.decorations())?.diagnostic_counts(id, floor);
    Ok(symbol_hash(DiagSeverity::ALL.map(|severity| {
        (
            severity.name(),
            SteelVal::IntV(counts.get(severity) as isize),
        )
    })))
}

#[cfg(test)]
mod tests;
