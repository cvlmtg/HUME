//! Multi-buffer Steel builtins: buffer/pane query and lifecycle ops.
//!
//! All builtins guard against init-eval context (`EvalMode::Init` or
//! `PluginLoad`) via the `cmd`-gated `builtins!` registration table entry,
//! where editor refs are not available.  Calling any of these from
//! `init.scm` raises a Steel error instead of returning a meaningless
//! default.

use steel::rvals::{IntoSteelVal, SteelVal};

use super::SteelResult;
use super::args::{ArgPane, list_of, not_live_err, symbol_hash};
use super::errors::generic_err;
use super::ids::{SteelBufferKey, SteelPane};
use crate::SteelCtx;
use crate::types::PaneHandle;

// ── Focus builtins ─────────────────────────────────────────────────────────────

/// `(focused-pane)` → the pane focused *right now*, paired with the buffer
/// it shows: a live host read, freshly resolved on every call. See
/// [`crate::host::BufferHost::focused_pane`]'s doc for when this is (and
/// isn't) the right one to reach for: a command declaring a leading `pane`
/// parameter receives the pane it was invoked through that way (dispatch
/// injects it, same mechanism as `count`/`extend`/`arg`/`force`) instead of
/// reading this. It exists for code with no pane of its own to act on.
pub(crate) fn focused_pane(ctx: &mut SteelCtx) -> SteelResult {
    Ok(SteelPane(ctx.host.buffers().focused_pane()).into_steel_val())
}

// ── Enumeration builtins ───────────────────────────────────────────────────────

/// `(buffers)` → list of every open buffer, as `(bid . #f)`-paned handles
/// (no pane of its own) in open-order.
pub(crate) fn buffers(ctx: &mut SteelCtx) -> SteelResult {
    let list: Vec<SteelVal> = ctx
        .host
        .buffers()
        .buffer_ids()
        .into_iter()
        .map(|id| SteelPane(PaneHandle::buffer_only(id)).into_steel_val())
        .collect();
    list.into_steelval().map_err(generic_err)
}

/// `(panes)` → list of every open pane, across every tab, including panes
/// in inactive tabs, not just the ones currently on screen.
pub(crate) fn panes(ctx: &mut SteelCtx) -> SteelResult {
    let list: Vec<SteelVal> = ctx
        .host
        .buffers()
        .panes()
        .into_iter()
        .map(|h| SteelPane(h).into_steel_val())
        .collect();
    list.into_steelval().map_err(generic_err)
}

/// `(buffer-panes pane)` → list of every pane showing `pane`'s buffer,
/// focused pane first, then the rest of the active tab, then other tabs:
/// the explicit choice a caller makes in place of the pane guess this
/// design removed. `(car (buffer-panes pane))` reproduces that guess.
pub(crate) fn buffer_panes(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    let list: Vec<SteelVal> = ctx
        .host
        .buffers()
        .buffer_panes(pane)
        .into_iter()
        .map(|h| SteelPane(h).into_steel_val())
        .collect();
    list.into_steelval().map_err(generic_err)
}

/// `(buffer-key pane)` → an opaque, hashable, `equal?`-comparable key naming
/// `pane`'s buffer alone. Two panes on the same buffer produce equal keys,
/// unlike a `SteelPane` value itself (see `builtins::ids`'s module doc for
/// why this is a distinct type rather than a pane with its pane field
/// cleared). The idiom for per-buffer plugin state (a hash keyed by buffer,
/// a `debounce-by` key) that must coalesce regardless of which pane a
/// command or hook happened to carry.
pub(crate) fn buffer_key(_ctx: &mut SteelCtx, pane: ArgPane) -> SteelResult {
    SteelBufferKey(pane.0.buffer())
        .into_steelval()
        .map_err(generic_err)
}

// ── Buffer property builtins ───────────────────────────────────────────────────

/// `(buffer-path pane)` → absolute path string, or `#f` for unsaved buffers.
pub(crate) fn buffer_path(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    match ctx.host.buffers().buffer_path(pane.buffer()) {
        Some(p) => hume_platform::path::strip_unc_prefix(p)
            .to_string_lossy()
            .into_owned()
            .into_steelval()
            .map_err(generic_err),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(buffer-display-path pane)` → fully display-ready path string
/// (absolutized, lexically normalized, UNC-stripped, `~`-collapsed) to print
/// verbatim, or `#f` for unsaved buffers. Unlike `buffer-path`, never
/// suitable for filesystem ops.
pub(crate) fn buffer_display_path(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    match ctx.host.buffers().buffer_display_path(pane.buffer()) {
        Some(p) => p.into_steelval().map_err(generic_err),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(buffer-name pane)` → display name (filename or `"*scratch*"`). `pane`'s
/// buffer liveness is already checked at decode time (`LivePane`), so a
/// `None` here would mean the host answered inconsistently with
/// `buffer_exists`. Never observed, but the trait still returns `Option`,
/// so it's handled rather than assumed.
pub(crate) fn buffer_name(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    let bid = pane.buffer();
    ctx.host
        .buffers()
        .buffer_display_name(bid)
        .ok_or_else(|| not_live_err("buffer-name", bid))?
        .into_steelval()
        .map_err(generic_err)
}

/// `(buffer-live? pane)` → `#t` if `pane`'s buffer still names an open
/// buffer, `#f` otherwise. Never raises, unlike every `LivePane`-checked
/// builtin. The idiom for a timer, debounce, or async continuation whose
/// captured `pane` may have closed by the time it fires: check this first,
/// rather than discovering the fact via a raise from whatever builtin the
/// callback was actually going to call.
pub(crate) fn buffer_live(ctx: &mut SteelCtx, pane: ArgPane) -> SteelResult {
    Ok(SteelVal::BoolV(
        ctx.host.buffers().buffer_exists(pane.0.buffer()),
    ))
}

/// `(pane-live? pane)` → `#t` if `pane` names a pane that still exists and
/// still shows its own buffer, `#f` otherwise. Never raises: the pane-aware
/// sibling of `buffer-live?` above. See [`BufferHost::pane_live`]'s doc for
/// the async-continuation use case this exists for.
pub(crate) fn pane_live(ctx: &mut SteelCtx, pane: ArgPane) -> SteelResult {
    Ok(SteelVal::BoolV(ctx.host.buffers().pane_live(pane.0)))
}

/// `(buffer-dirty? pane)` → `#t` if the buffer has unsaved edits.
pub(crate) fn buffer_dirty(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    let bid = pane.buffer();
    let dirty = ctx
        .host
        .buffers()
        .buffer_is_dirty(bid)
        .ok_or_else(|| not_live_err("buffer-dirty?", bid))?;
    Ok(SteelVal::BoolV(dirty))
}

/// `(buffer-generation pane)` → int, bumped by every mutation to `pane`'s
/// buffer. Steel-side staleness token; not LSP-specific despite the
/// motivation.
pub(crate) fn buffer_generation(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    let bid = pane.buffer();
    let generation = ctx
        .host
        .buffers()
        .buffer_generation(bid)
        .ok_or_else(|| not_live_err("buffer-generation", bid))?;
    Ok(SteelVal::IntV(generation as isize))
}

/// `(buffer-undo-tree pane)` → a list with one `(hash 'id 'parent 'age-secs
/// 'current? 'saved?)` per revision of `pane`'s buffer's undo history, in id
/// order (every parent precedes its children). `'parent` is `#f` for the
/// root; `'age-secs` is whole seconds since the revision was recorded.
pub(crate) fn buffer_undo_tree(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    let bid = pane.buffer();
    let nodes = ctx
        .host
        .buffers()
        .buffer_undo_tree(bid)
        .ok_or_else(|| not_live_err("buffer-undo-tree", bid))?;
    Ok(list_of(nodes.into_iter().map(|node| {
        symbol_hash([
            ("id", SteelVal::IntV(node.id as isize)),
            (
                "parent",
                node.parent
                    .map_or(SteelVal::BoolV(false), |p| SteelVal::IntV(p as isize)),
            ),
            ("age-secs", SteelVal::IntV(node.age_secs as isize)),
            ("current?", SteelVal::BoolV(node.current)),
            ("saved?", SteelVal::BoolV(node.saved)),
        ])
    })))
}

/// `(buffer-text pane)` → the buffer's full live (dirty) content, string,
/// trailing `\n` included.
pub(crate) fn buffer_text(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    let bid = pane.buffer();
    ctx.host
        .buffers()
        .buffer_text(bid)
        .ok_or_else(|| not_live_err("buffer-text", bid))?
        .into_steelval()
        .map_err(generic_err)
}

/// `(buffer-line-count pane)` → int: `pane`'s buffer's content line count,
/// excluding the phantom line past its structural trailing `\n` (matches
/// the statusline and `:w`). O(1): reads `buffer_line_count` directly rather
/// than counting a materialized `(buffer-lines pane)` list.
pub(crate) fn buffer_line_count(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    let bid = pane.buffer();
    let count = ctx
        .host
        .buffers()
        .buffer_line_count(bid)
        .ok_or_else(|| not_live_err("buffer-line-count", bid))?;
    Ok(SteelVal::IntV(count as isize))
}

/// `(%buffer-lines pane start end)`: Rust half of the bootstrap-wrapped
/// `(buffer-lines pane #:start .. #:end ..)`. `start`/`end` are already-decoded
/// `Option<usize>` from `bootstrap.scm`'s `#f`-defaulted keyword args:
/// `start` defaults to `0`, `end` to the buffer's content line count.
/// Content lines in `[start, end)`, 0-based, end-exclusive, each with its
/// trailing line break stripped. The phantom line past the buffer's
/// structural trailing `\n` is never included (matches the statusline's and
/// `:w`'s line count). Raises rather than clamping on `start > end` or
/// `end` past the line count.
pub(crate) fn buffer_lines(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    start: Option<usize>,
    end: Option<usize>,
) -> SteelResult {
    let bid = pane.buffer();
    let start = start.unwrap_or(0);
    // One error message for both lookups below: the second is unreachable
    // in practice (nothing can close `bid` between two synchronous host
    // calls) but the trait returns `Option`, so it's handled, not assumed.
    let invalid_id = || not_live_err("buffer-lines", bid);
    let line_count = ctx
        .host
        .buffers()
        .buffer_line_count(bid)
        .ok_or_else(invalid_id)?;
    let end = end.unwrap_or(line_count);
    if start > end || end > line_count {
        return Err(generic_err(format!(
            "buffer-lines: range {start}..{end} out of bounds for a {line_count}-line buffer"
        )));
    }
    // Trusted mint: the check above is what licenses ContentLine::new here.
    // `end` may legitimately equal `line_count` (the one-past-last-line
    // exclusive bound `ContentLineCount::end_exclusive()` also names), which
    // `ContentLine::checked` would reject.
    let range = hume_rope::offset::ExclusiveRange::new(
        hume_rope::line::ContentLine::new(start),
        hume_rope::line::ContentLine::new(end),
    );
    let lines = ctx
        .host
        .buffers()
        .buffer_lines(bid, range)
        .ok_or_else(invalid_id)?;
    lines.into_steelval().map_err(generic_err)
}

// ── Mutating builtins ─────────────────────────────────────────────────────────

/// `(open-buffer! path)` → a pane-less handle for the opened buffer.
///
/// Opens `path` as a new buffer and returns its handle. If the path is
/// already open, returns the existing buffer's handle without opening a new
/// one. Does not switch the focused pane; call `(switch-to-buffer! pane
/// target)` separately if desired.
///
/// Language detection can't run inline here (it needs Steel-eval capability
/// this builtin's host doesn't hold), so the editor-side open chokepoint
/// (`buffer::lifecycle::open_buffer_and_notify`) queues it onto
/// `EditorState.pending_language_detection` instead; `Editor::
/// apply_script_effects` drains it once this eval returns.
pub(crate) fn open_buffer(ctx: &mut SteelCtx, path: String) -> SteelResult {
    let bid = ctx
        .host
        .buffers()
        .open_buffer(std::path::Path::new(&path))
        .map_err(generic_err)?;
    SteelPane(PaneHandle::buffer_only(bid))
        .into_steelval()
        .map_err(generic_err)
}

/// `(close-buffer! pane)` → void.
///
/// Closes `pane`'s buffer. Raises a Steel error for an invalid or unknown
/// buffer.
pub(crate) fn close_buffer(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    // The host applies the close (and its focus fallout) synchronously, not
    // as a deferred effect: `focused-pane`/`switch-to-buffer!`'s own
    // `focused_pane()` read already sees it on their very next call.
    ctx.host
        .buffers()
        .close_buffer(pane.buffer())
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

/// `(switch-to-buffer! pane target)` → void.
///
/// Redirects `pane`'s own pane to `target`'s buffer, recording the current
/// position in the jump list. Raises a Steel error for an invalid or
/// unknown `target`, or when `pane` carries no pane, a closed one, or one
/// that no longer shows `pane`'s own buffer.
pub(crate) fn switch_to_buffer(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    target: PaneHandle,
) -> SteelResult {
    ctx.host
        .buffers()
        .switch_to_buffer(pane, target.buffer())
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

// ── Live cursor/selection reads ───────────────────────────────────────────────

/// `(buffer-cursor-line pane)` → 0-indexed line of the primary
/// cursor in `pane`'s own pane.
///
/// Reads live state: reflects any synchronous edits or motions that ran
/// earlier in the same Steel eval (e.g. after `(move-left)`).
pub(crate) fn buffer_cursor_line(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    Ok(SteelVal::IntV(
        ctx.host
            .cursor()
            .buffer_cursor_line(pane)
            .map_err(generic_err)? as isize,
    ))
}

/// `(buffer-selections pane)` → list of `(anchor head start end primary?)`
/// per selection in `pane`'s own pane. The entry shape is opaque to scripts;
/// `core:stdlib`'s `stdlib/selection-*` accessors are the reading API.
/// Offsets are raw 0-indexed chars, direction preserved (anchor > head when
/// backward), sorted by selection start, exactly one `primary?` = `#t`.
/// `start`..`end` is what the selection covers, `end` exclusive.
pub(crate) fn buffer_selections(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    let sels = ctx
        .host
        .cursor()
        .buffer_selections(pane)
        .map_err(generic_err)?;
    let list: Vec<SteelVal> = sels
        .into_iter()
        .map(|sel| {
            vec![
                SteelVal::IntV(sel.anchor as isize),
                SteelVal::IntV(sel.head as isize),
                SteelVal::IntV(sel.start as isize),
                SteelVal::IntV(sel.end as isize),
                SteelVal::BoolV(sel.primary),
            ]
            .into_steelval()
            .map_err(generic_err)
        })
        .collect::<Result<_, _>>()?;
    list.into_steelval().map_err(generic_err)
}

/// `(offset->line pane idx)` → 0-indexed line containing 0-indexed
/// char offset `idx` in `pane`'s buffer's live text, or `#f` when `idx` is
/// out of range (> buffer length in chars). Raises on a stale buffer, same
/// liveness contract as every other explicit-pane builtin, checked at
/// decode time (`LivePane`) before `idx` is ever looked at, so a malformed
/// `idx` still raises its own error on a stale buffer rather than being
/// masked behind the liveness one (steel-core decodes `idx`'s own `Usize`
/// argument type first regardless, left to right, but the ordering holds
/// either way).
pub(crate) fn offset_to_line(ctx: &mut SteelCtx, pane: PaneHandle, idx: usize) -> SteelResult {
    match ctx.host.cursor().offset_to_line(pane.buffer(), idx) {
        Some(line) => Ok(SteelVal::IntV(line as isize)),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(line->offset pane line)` → 0-based char offset where 0-based content
/// `line` starts in `pane`'s buffer's live text. Raises on a stale buffer or
/// a `line` past the content line count, same bounds contract as
/// `buffer-lines` (raises rather than clamping).
///
/// Named for the conversion family (`offset->line`, `lsp-position->offset`,
/// `lsp-range->offsets`, `path->display`), not the `buffer-text`/
/// `buffer-lines` accessor family.
pub(crate) fn line_to_offset(ctx: &mut SteelCtx, pane: PaneHandle, line: usize) -> SteelResult {
    let bid = pane.buffer();
    let invalid_id = || not_live_err("line->offset", bid);
    let line_count = ctx
        .host
        .buffers()
        .buffer_line_count(bid)
        .ok_or_else(invalid_id)?;
    if line >= line_count {
        return Err(generic_err(format!(
            "line->offset: line {line} is out of range (buffer has {line_count} content lines)"
        )));
    }
    // Trusted mint: the check above is what licenses ContentLine::new here.
    let line = hume_rope::line::ContentLine::new(line);
    let offset = ctx
        .host
        .buffers()
        .line_to_offset(bid, line)
        .ok_or_else(invalid_id)?;
    Ok(SteelVal::IntV(offset as isize))
}

/// `(viewport-range pane)` → `(hash 'start first-line 'end end-line)` visible in
/// `pane`'s own pane: 0-based, end-exclusive, matching `buffer-lines`'
/// range convention. Raises when `pane` carries no pane, a closed one, or
/// one that no longer shows `pane`'s buffer. Answers for a background-tab
/// pane too, but its scroll position may lag until that tab is next
/// focused (see `EditorHostImpl::viewport_range`'s own doc). Reads live
/// view state, which only exists at command dispatch, hook fire, or a
/// queued-call drain.
pub(crate) fn viewport_range(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    let range = ctx
        .host
        .buffers()
        .viewport_range(pane)
        .map_err(generic_err)?;
    Ok(symbol_hash([
        ("start", SteelVal::IntV(range.start.index() as isize)),
        ("end", SteelVal::IntV(range.end.index() as isize)),
    ]))
}

/// `(selections-linewise? pane)`.
pub(crate) fn selections_linewise(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    Ok(SteelVal::BoolV(
        ctx.host
            .cursor()
            .selections_linewise(pane)
            .map_err(generic_err)?,
    ))
}

/// `(selections-charwise? pane)`.
pub(crate) fn selections_charwise(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    Ok(SteelVal::BoolV(
        ctx.host
            .cursor()
            .selections_charwise(pane)
            .map_err(generic_err)?,
    ))
}

/// `(symbol-under-cursor pane)`.
pub(crate) fn symbol_under_cursor(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    Ok(SteelVal::StringV(
        ctx.host
            .cursor()
            .symbol_under_cursor(pane)
            .map_err(generic_err)?
            .into(),
    ))
}

#[cfg(test)]
mod tests;
