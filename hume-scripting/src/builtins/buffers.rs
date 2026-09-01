//! Multi-buffer Steel builtins — buffer/pane query and lifecycle ops.
//!
//! All builtins guard against init-eval context (`EvalMode::Init` or
//! `PluginLoad`) via the `cmd`-gated `builtins!` registration table entry,
//! where editor refs are not available.  Calling any of these from
//! `init.scm` raises a Steel error instead of returning a meaningless
//! default.

use steel::rvals::{IntoSteelVal, SteelVal};

use super::SteelResult;
use super::args::{BidArg, cons_pair, optional_string_arg, optional_usize_arg, usize_arg};
use super::errors::generic_err;
use super::ids::{SteelBufferId, SteelPaneId};
use crate::{SteelCtx, types::Effect};

// ── Focus builtins ─────────────────────────────────────────────────────────────

/// `(current-buffer)` → BufferId of the focused buffer at dispatch time.
pub(crate) fn current_buffer(ctx: &mut SteelCtx) -> SteelResult {
    Ok(SteelBufferId(ctx.focused_buffer_id).into_steel_val())
}

/// `(current-pane)` → PaneId of the focused pane at dispatch time.
pub(crate) fn current_pane(ctx: &mut SteelCtx) -> SteelResult {
    Ok(SteelPaneId(ctx.focused_pane_id).into_steel_val())
}

// ── Enumeration builtins ───────────────────────────────────────────────────────

/// `(buffers)` → list of all open BufferIds in open-order.
pub(crate) fn buffers(ctx: &mut SteelCtx) -> SteelResult {
    let list: Vec<SteelVal> = ctx
        .host
        .buffers()
        .buffer_ids()
        .into_iter()
        .map(|id| SteelBufferId(id).into_steel_val())
        .collect();
    list.into_steelval().map_err(generic_err)
}

/// `(panes)` → list of all open PaneIds.
pub(crate) fn panes(ctx: &mut SteelCtx) -> SteelResult {
    let list: Vec<SteelVal> = ctx
        .host
        .buffers()
        .pane_ids()
        .into_iter()
        .map(|id| SteelPaneId(id).into_steel_val())
        .collect();
    list.into_steelval().map_err(generic_err)
}

// ── Buffer property builtins ───────────────────────────────────────────────────

/// `(buffer-path bid)` → absolute path string, or `#f` for unsaved buffers.
pub(crate) fn buffer_path(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.require_live(ctx, "buffer-path")?;
    match ctx.host.buffers().buffer_path(id) {
        Some(p) => p
            .to_string_lossy()
            .into_owned()
            .into_steelval()
            .map_err(generic_err),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(buffer-display-path bid)` → fully display-ready path string (absolutized,
/// lexically normalized, UNC-stripped, `~`-collapsed) — print verbatim, or `#f`
/// for unsaved buffers. Unlike `buffer-path`, never suitable for filesystem ops.
pub(crate) fn buffer_display_path(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.require_live(ctx, "buffer-display-path")?;
    match ctx.host.buffers().buffer_display_path(id) {
        Some(p) => p.into_steelval().map_err(generic_err),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(buffer-name bid)` → display name (filename or `"*scratch*"`).
pub(crate) fn buffer_name(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    ctx.host
        .buffers()
        .buffer_display_name(id)
        .ok_or_else(|| generic_err(format!("buffer-name: invalid buffer id {id:?}")))?
        .into_steelval()
        .map_err(generic_err)
}

/// `(buffer-dirty? bid)` → `#t` if the buffer has unsaved edits.
pub(crate) fn buffer_dirty(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    let dirty = ctx
        .host
        .buffers()
        .buffer_is_dirty(id)
        .ok_or_else(|| generic_err(format!("buffer-dirty?: invalid buffer id {id:?}")))?;
    Ok(SteelVal::BoolV(dirty))
}

/// `(buffer-generation bid)` → int — bumped by every mutation to `bid`.
/// Steel-side staleness token; not LSP-specific despite the motivation.
pub(crate) fn buffer_generation(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    let generation = ctx
        .host
        .buffers()
        .buffer_generation(id)
        .ok_or_else(|| generic_err(format!("buffer-generation: invalid buffer id {id:?}")))?;
    Ok(SteelVal::IntV(generation as isize))
}

/// `(buffer-text bid)` → the buffer's full live (dirty) content, string,
/// trailing `\n` included.
pub(crate) fn buffer_text(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    ctx.host
        .buffers()
        .buffer_text(id)
        .ok_or_else(|| generic_err(format!("buffer-text: invalid buffer id {id:?}")))?
        .into_steelval()
        .map_err(generic_err)
}

/// `(%buffer-lines bid start end)` — Rust half of the bootstrap-wrapped
/// `(buffer-lines bid #:start .. #:end ..)`. `start`/`end` are already-decoded
/// `Option<usize>` from `bootstrap.scm`'s `#f`-defaulted keyword args:
/// `start` defaults to `0`, `end` to the buffer's content line count.
/// Content lines in `[start, end)`, 0-based, end-exclusive, each with its
/// trailing line break stripped — the phantom line past the buffer's
/// structural trailing `\n` is never included (matches the statusline's and
/// `:w`'s line count). Raises rather than clamping on `start > end` or
/// `end` past the line count.
pub(crate) fn buffer_lines(
    ctx: &mut SteelCtx,
    bid: BidArg,
    start: SteelVal,
    end: SteelVal,
) -> SteelResult {
    let id = bid.0;
    // Decode both args before touching the host — a stale bid must not mask
    // a genuine type error in either argument (or the reverse) depending on
    // which happens to be checked first.
    let start = optional_usize_arg(start, "buffer-lines start")?.unwrap_or(0);
    let end = optional_usize_arg(end, "buffer-lines end")?;
    // One error message for both lookups below — the second is unreachable
    // in practice (nothing can close `id` between two synchronous host
    // calls) but the trait returns `Option`, so it's handled, not assumed.
    let invalid_id = || generic_err(format!("buffer-lines: invalid buffer id {id:?}"));
    let line_count = ctx
        .host
        .buffers()
        .buffer_line_count(id)
        .ok_or_else(invalid_id)?;
    let end = end.unwrap_or(line_count);
    if start > end || end > line_count {
        return Err(generic_err(format!(
            "buffer-lines: range {start}..{end} out of bounds for a {line_count}-line buffer"
        )));
    }
    let lines = ctx
        .host
        .buffers()
        .buffer_lines(id, start..end)
        .ok_or_else(invalid_id)?;
    lines.into_steelval().map_err(generic_err)
}

// ── Mutating builtins ─────────────────────────────────────────────────────────

/// `(open-buffer! path)` → BufferId.
///
/// Opens `path` as a new buffer and returns its `BufferId`. If the path is
/// already open, returns the existing id without opening a new buffer. Does
/// not switch the focused pane — call `(switch-to-buffer! bid)` separately
/// if desired.
///
/// Language detection can't run inline here — it needs Steel-eval capability
/// this builtin's host doesn't hold — so the editor-side open chokepoint
/// (`buffer::lifecycle::open_buffer_and_notify`) queues it onto
/// `EditorState.pending_language_detection` instead; `Editor::
/// apply_script_effects` drains it once this eval returns.
pub(crate) fn open_buffer(ctx: &mut SteelCtx, path: String) -> SteelResult {
    let bid = ctx
        .host
        .buffers()
        .open_buffer(std::path::Path::new(&path))
        .map_err(generic_err)?;
    SteelBufferId(bid).into_steelval().map_err(generic_err)
}

/// `(close-buffer! bid)` → void.
///
/// Closes the buffer identified by `bid`. Raises a Steel error for an invalid
/// or unknown `bid`.
pub(crate) fn close_buffer(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.require_live(ctx, "close-buffer!")?;
    let new_live = ctx.host.buffers().close_buffer(id).map_err(generic_err)?;
    ctx.live_focused_buffer_id = new_live;
    Ok(SteelVal::Void)
}

/// `(switch-to-buffer! bid)` → void.
///
/// Redirects the focused pane to the buffer identified by `bid`, recording
/// the current position in the jump list. Raises a Steel error for an invalid
/// or unknown `bid`.
pub(crate) fn switch_to_buffer(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let target = bid.require_live(ctx, "switch-to-buffer!")?;
    let current = ctx.live_focused_buffer_id;
    ctx.host
        .buffers()
        .switch_to_buffer(current, target)
        .map_err(generic_err)?;
    ctx.live_focused_buffer_id = target;
    Ok(SteelVal::Void)
}

// ── Language builtins ─────────────────────────────────────────────────────────

/// Reverse-scan the effect log for the last `set-buffer-language!` call for
/// `id` queued so far this eval; fall back to `fallback` (the buffer's
/// stored language).
fn effective_language(
    effects: &[crate::types::QueuedEffect],
    id: hume_engine::pipeline::BufferId,
    fallback: Option<String>,
) -> Option<String> {
    effects
        .iter()
        .rev()
        .find_map(|queued| match &queued.effect {
            Effect::SetBufferLanguage { buffer, language } if *buffer == id => {
                Some(language.clone())
            }
            _ => None,
        })
        .unwrap_or(fallback)
}

/// `(buffer-language bid)` → string or `#f`.
pub(crate) fn buffer_language(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.require_live(ctx, "buffer-language")?;
    let fallback = ctx.host.buffers().buffer_stored_language(id);
    let lang = effective_language(ctx.effects, id, fallback);
    match lang {
        Some(name) => name.into_steelval().map_err(generic_err),
        None => Ok(SteelVal::BoolV(false)),
    }
}

// ── Live cursor/selection reads ───────────────────────────────────────────────

/// `(current-line-number)` → 1-indexed line number of the primary cursor, or `#f`.
///
/// Reads live state — reflects any synchronous edits or motions that ran
/// earlier in the same Steel eval (e.g. after `(move-left)`).
pub(crate) fn current_line_number(ctx: &mut SteelCtx) -> SteelResult {
    match ctx.host.cursor().current_line_number() {
        Some(n) => Ok(SteelVal::IntV(n as isize)),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(current-selections)` → list of `(anchor head primary?)` per selection —
/// raw 0-indexed inclusive char offsets, direction preserved (anchor > head
/// when backward), sorted by selection start, exactly one `primary?` = `#t` —
/// or `#f` when the focused (pane, buffer) has no seeded pane state.
pub(crate) fn current_selections(ctx: &mut SteelCtx) -> SteelResult {
    match ctx.host.cursor().current_selections() {
        Some(sels) => {
            let list: Vec<SteelVal> = sels
                .into_iter()
                .map(|(anchor, head, primary)| {
                    vec![
                        SteelVal::IntV(anchor as isize),
                        SteelVal::IntV(head as isize),
                        SteelVal::BoolV(primary),
                    ]
                    .into_steelval()
                    .map_err(generic_err)
                })
                .collect::<Result<_, _>>()?;
            list.into_steelval().map_err(generic_err)
        }
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(char-index->line idx)` → 1-indexed line number containing 0-indexed char
/// offset `idx`, or `#f` when the focused buffer id is stale (buffer no
/// longer exists) or `idx` is out of range (> buffer length in chars).
pub(crate) fn char_index_to_line(ctx: &mut SteelCtx, idx: SteelVal) -> SteelResult {
    let idx = usize_arg(idx, "char-index->line")?;
    match ctx.host.cursor().char_index_to_line(idx) {
        Some(line) => Ok(SteelVal::IntV(line as isize)),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(line->offset bid line)` → 0-based char offset where 0-based content
/// `line` starts in `bid`'s live text. Raises on a stale `bid` or a `line`
/// past the content line count — same bounds contract as `buffer-lines`
/// (raises rather than clamping), checked here via `buffer_line_count`
/// before the host is asked to convert.
///
/// Not the inverse of `char-index->line`: that builtin is 1-indexed and
/// reads the *focused* buffer; this is 0-indexed and takes an explicit
/// `bid`. Named for the conversion family (`char-index->line`,
/// `lsp-position->offset`, `lsp-range->offsets`, `path->display`), not the
/// `buffer-text`/`buffer-lines` accessor family.
pub(crate) fn line_to_offset(ctx: &mut SteelCtx, bid: BidArg, line: SteelVal) -> SteelResult {
    let id = bid.0;
    // Decode before touching the host — same reasoning as `buffer_lines`:
    // a stale bid must not mask a genuine type error in `line`.
    let line = usize_arg(line, "line->offset")?;
    let invalid_id = || generic_err(format!("line->offset: invalid buffer id {id:?}"));
    let line_count = ctx
        .host
        .buffers()
        .buffer_line_count(id)
        .ok_or_else(invalid_id)?;
    if line >= line_count {
        return Err(generic_err(format!(
            "line->offset: line {line} is out of range (buffer has {line_count} content lines)"
        )));
    }
    let offset = ctx
        .host
        .buffers()
        .line_to_offset(id, line)
        .ok_or_else(invalid_id)?;
    Ok(SteelVal::IntV(offset as isize))
}

/// `(viewport-range bid)` → `(first-line . end-line)` currently visible
/// for `bid` (the focused pane's if shown there, else the first pane showing
/// it) — 0-based, end-exclusive, matching `buffer-lines`' range convention —
/// or `#f` if `bid` isn't open in any pane. Reads live view state, which
/// only exists at command dispatch, hook fire, or a queued-call drain.
pub(crate) fn viewport_range(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    match ctx.host.buffers().viewport_range(id) {
        Some(range) => cons_pair(
            SteelVal::IntV(range.start as isize),
            SteelVal::IntV(range.end as isize),
        ),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(selections-linewise? bid)`.
pub(crate) fn selections_linewise(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    Ok(SteelVal::BoolV(ctx.host.cursor().selections_linewise(id)))
}

/// `(symbol-under-cursor bid)`.
pub(crate) fn symbol_under_cursor(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    Ok(SteelVal::StringV(
        ctx.host.cursor().symbol_under_cursor(id).into(),
    ))
}

/// `(set-buffer-language! bid lang-or-#f)` — deferred; applied after the eval returns.
pub(crate) fn set_buffer_language_steel(
    ctx: &mut SteelCtx,
    bid: BidArg,
    lang: SteelVal,
) -> SteelResult {
    let new_lang = optional_string_arg(lang, "set-buffer-language!")?;
    let id = bid.require_live(ctx, "set-buffer-language!")?;
    let fallback = ctx.host.buffers().buffer_stored_language(id);
    if effective_language(ctx.effects, id, fallback) == new_lang {
        return Ok(SteelVal::Void);
    }
    ctx.push_effect(Effect::SetBufferLanguage {
        buffer: id,
        language: new_lang,
    });
    Ok(SteelVal::Void)
}

#[cfg(test)]
mod tests;
