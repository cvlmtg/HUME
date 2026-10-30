//! Multi-buffer Steel builtins: buffer/pane query and lifecycle ops.
//!
//! All builtins guard against init-eval context (`EvalMode::Init` or
//! `PluginLoad`) via the `cmd`-gated `builtins!` registration table entry,
//! where editor refs are not available.  Calling any of these from
//! `init.scm` raises a Steel error instead of returning a meaningless
//! default.

use steel::rerrs::SteelErr;
use steel::rvals::{IntoSteelVal, SteelVal};

use super::SteelResult;
use super::args::{
    ArgPane, LINE_RANGE_KEYS, bool_arg, char_range_value, line_range_value, list_of, not_live_err,
    optional_hash_entry, relative_base_arg, symbol_hash,
};
use super::errors::generic_err;
use super::ids::{BufferGeneration, SteelBufferKey, SteelPane};
use crate::SteelCtx;
use crate::host::{SelectionInfo, ShownHandle};
use hume_core::{BufferId, OnUnsaved, PaneHandle};
use hume_platform::path::AbsolutePath;

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
/// focused pane first, then the rest of the active tab, then other tabs.
/// `(car (buffer-panes pane))` is the caller's own choice of a pane on the
/// buffer.
pub(crate) fn buffer_panes(ctx: &mut SteelCtx, bid: BufferId) -> SteelResult {
    let list: Vec<SteelVal> = ctx
        .host
        .buffers()
        .buffer_panes(bid)
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
pub(crate) fn buffer_key(pane: ArgPane) -> SteelResult {
    SteelBufferKey(pane.0.buffer())
        .into_steelval()
        .map_err(generic_err)
}

// ── Buffer property builtins ───────────────────────────────────────────────────

/// `(buffer-path pane)` → absolute path string, or `#f` for unsaved buffers.
pub(crate) fn buffer_path(ctx: &mut SteelCtx, bid: BufferId) -> SteelResult {
    match ctx.host.buffers().buffer_path(bid) {
        Some(p) => p
            .to_string_lossy()
            .into_owned()
            .into_steelval()
            .map_err(generic_err),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(buffer-display-path pane #:relative base)` → fully display-ready path
/// string (absolutized, `..`-free, UNC-stripped, `~`-collapsed) to print
/// verbatim, or `#f` for unsaved buffers. With `base`, the path relative to
/// it when the file is inside it. Unlike `buffer-path`, never suitable for
/// filesystem ops.
pub(crate) fn buffer_display_path(
    ctx: &mut SteelCtx,
    bid: BufferId,
    relative: SteelVal,
) -> SteelResult {
    let buffers = ctx.host.buffers();
    let base = relative_base_arg(relative, || buffers.cwd())?;
    match buffers.buffer_display_path(bid, base.as_ref()) {
        Some(p) => p.into_steelval().map_err(generic_err),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(buffer-name pane)` → display name (filename or `"*scratch*"`). `pane`'s
/// buffer liveness is already checked at resolve time (`LiveBuffer`), so a
/// `None` here would mean the host answered inconsistently with
/// `buffer_exists`. Never observed, but the trait still returns `Option`,
/// so it's handled rather than assumed.
pub(crate) fn buffer_name(ctx: &mut SteelCtx, bid: BufferId) -> SteelResult {
    ctx.host
        .buffers()
        .buffer_display_name(bid)
        .ok_or_else(|| not_live_err("pane", bid))?
        .into_steelval()
        .map_err(generic_err)
}

/// `(buffer-live? pane)` → `#t` if `pane`'s buffer still names an open
/// buffer, `#f` otherwise. Never raises, unlike every builtin with a checked
/// pane argument. The idiom for a timer, debounce, or async continuation whose
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
/// sibling of `buffer-live?` above, for a debounced or async continuation
/// whose captured `pane` may have closed, or been repointed at another
/// buffer, by the time it fires.
pub(crate) fn pane_live(ctx: &mut SteelCtx, pane: ArgPane) -> SteelResult {
    Ok(SteelVal::BoolV(ctx.host.buffers().pane_live(pane.0)))
}

/// `(buffer-dirty? pane)` → `#t` if the buffer has unsaved edits.
pub(crate) fn buffer_dirty(ctx: &mut SteelCtx, bid: BufferId) -> SteelResult {
    let dirty = ctx
        .host
        .buffers()
        .buffer_is_dirty(bid)
        .ok_or_else(|| not_live_err("pane", bid))?;
    Ok(SteelVal::BoolV(dirty))
}

/// `(buffer-generation pane)` → an opaque [`BufferGeneration`], a new one
/// after every mutation to `pane`'s buffer and `equal?` to every value read
/// before the next. The staleness token `#:expect-generation` and
/// `buffer-changes-since` take.
pub(crate) fn buffer_generation(ctx: &mut SteelCtx, bid: BufferId) -> SteelResult {
    let generation = ctx
        .host
        .buffers()
        .buffer_generation(bid)
        .ok_or_else(|| not_live_err("pane", bid))?;
    Ok(BufferGeneration {
        buffer: bid,
        generation,
    }
    .into_steel_val())
}

/// `(buffer-search pane pattern)` → `(hash 'start-char s 'end-char e)` for
/// the first match of `pattern` in `pane`'s buffer, `#f` when nothing
/// matches. `pattern` reads as input at the `/` prompt does. Raises for an
/// empty or invalid pattern. Moves no selection and leaves the current
/// search alone.
pub(crate) fn buffer_search(ctx: &mut SteelCtx, bid: BufferId, pattern: String) -> SteelResult {
    let found = ctx
        .host
        .buffers()
        .buffer_search(bid, &pattern)
        .ok_or_else(|| not_live_err("pane", bid))?
        .map_err(generic_err)?;
    Ok(found.map_or(SteelVal::BoolV(false), |range| {
        char_range_value(range.start, range.end)
    }))
}

/// `(word-search-pattern pane word)` → the search input `*` sets for `word`
/// in `pane`'s buffer: `word` matched literally, anchored as a whole word
/// where that buffer's `word-chars` and the regex engine agree on its edge
/// characters. Pass it to `buffer-search`. Raises for an empty word.
pub(crate) fn word_search_pattern(ctx: &mut SteelCtx, bid: BufferId, word: String) -> SteelResult {
    if word.is_empty() {
        return Err(generic_err("word is empty"));
    }
    let pattern = ctx
        .host
        .buffers()
        .word_search_pattern(bid, &word)
        .ok_or_else(|| not_live_err("pane", bid))?;
    Ok(SteelVal::StringV(pattern.into()))
}

/// `(buffer-undo-tree pane)` → a list with one `(hash 'id 'parent 'age-secs
/// 'current? 'saved?)` per revision of `pane`'s buffer's undo history, in id
/// order (every parent precedes its children). `'parent` is `#f` for the
/// root; `'age-secs` is whole seconds since the revision was recorded.
pub(crate) fn buffer_undo_tree(ctx: &mut SteelCtx, bid: BufferId) -> SteelResult {
    let nodes = ctx
        .host
        .buffers()
        .buffer_undo_tree(bid)
        .ok_or_else(|| not_live_err("pane", bid))?;
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
pub(crate) fn buffer_text(ctx: &mut SteelCtx, bid: BufferId) -> SteelResult {
    ctx.host
        .buffers()
        .buffer_text(bid)
        .ok_or_else(|| not_live_err("pane", bid))?
        .into_steelval()
        .map_err(generic_err)
}

/// `(buffer-line-count pane)` → int: `pane`'s buffer's content line count,
/// excluding the phantom line past its structural trailing `\n` (matches
/// the statusline and `:w`). O(1): reads `buffer_line_count` directly rather
/// than counting a materialized `(buffer-lines pane)` list.
pub(crate) fn buffer_line_count(ctx: &mut SteelCtx, bid: BufferId) -> SteelResult {
    let count = ctx
        .host
        .buffers()
        .buffer_line_count(bid)
        .ok_or_else(|| not_live_err("pane", bid))?;
    Ok(SteelVal::IntV(count as isize))
}

/// `(buffer-char-count pane)` → int: `pane`'s buffer's length in chars,
/// structural trailing `\n` included. O(1), with no `buffer-text` copy.
pub(crate) fn buffer_char_count(ctx: &mut SteelCtx, bid: BufferId) -> SteelResult {
    let count = ctx
        .host
        .buffers()
        .buffer_char_count(bid)
        .ok_or_else(|| not_live_err("pane", bid))?;
    Ok(SteelVal::IntV(count as isize))
}

/// `(%buffer-lines pane range)`: Rust half of the bootstrap-wrapped
/// `(buffer-lines pane #:range ..)`. `range` is a line range value (see
/// [`line_range_value`]) or `#f` for every content line. Content lines in
/// `[start-line, end-line)`, 0-based, end-exclusive, each with its trailing
/// line break stripped. The phantom line past the buffer's structural
/// trailing `\n` is never included (matches the statusline's and `:w`'s line
/// count). Raises rather than clamping on a missing end, `start-line >
/// end-line`, or `end-line` past the line count.
pub(crate) fn buffer_lines(ctx: &mut SteelCtx, bid: BufferId, range: SteelVal) -> SteelResult {
    let range = optional_hash_entry(range, "#:range", LINE_RANGE_KEYS)?
        .map(|range| range.line_range())
        .transpose()?;
    // One error message for both lookups below: the second is unreachable
    // in practice (nothing can close `bid` between two synchronous host
    // calls) but the trait returns `Option`, so it's handled, not assumed.
    let invalid_id = || not_live_err("pane", bid);
    let line_count = ctx
        .host
        .buffers()
        .buffer_line_count(bid)
        .ok_or_else(invalid_id)?;
    let (start, end) = range.unwrap_or((0, line_count));
    if start > end || end > line_count {
        return Err(generic_err(format!(
            "range {start}..{end} out of bounds for a {line_count}-line buffer"
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
pub(crate) fn open_buffer(ctx: &mut SteelCtx, path: AbsolutePath) -> SteelResult {
    let bid = ctx.host.buffers().open_buffer(&path).map_err(generic_err)?;
    SteelPane(PaneHandle::buffer_only(bid))
        .into_steelval()
        .map_err(generic_err)
}

/// `(%close-buffer! pane discard-unsaved)` → void, behind `close-buffer!`'s
/// `#:discard-unsaved` keyword wrapper.
///
/// Closes `pane`'s buffer. Raises a Steel error for an invalid or unknown
/// buffer, one with unsaved changes unless `discard-unsaved` is `#t`, or
/// one a pipe is running in.
pub(crate) fn close_buffer(
    ctx: &mut SteelCtx,
    bid: BufferId,
    discard_unsaved: SteelVal,
) -> SteelResult {
    let on_unsaved = OnUnsaved::from_force(bool_arg(discard_unsaved, "#:discard-unsaved")?);
    // The host applies the close (and its focus fallout) synchronously, not
    // as a deferred effect: `focused-pane`/`switch-to-buffer!`'s own
    // `focused_pane()` read already sees it on their very next call.
    ctx.host
        .buffers()
        .close_buffer(bid, on_unsaved)
        .map_err(generic_err)?;
    Ok(SteelVal::Void)
}

/// `(switch-to-buffer! pane target)` → the pane, now showing `target`'s
/// buffer.
///
/// Redirects `pane`'s own pane to `target`'s buffer, recording the current
/// position in the jump list. `pane` itself still names the buffer the pane
/// left. Raises a Steel error for an invalid or
/// unknown `target`, or when `pane` carries no pane, a closed one, or one
/// that no longer shows `pane`'s own buffer.
pub(crate) fn switch_to_buffer(
    ctx: &mut SteelCtx,
    pane: ShownHandle,
    target: BufferId,
) -> SteelResult {
    ctx.host
        .buffers()
        .switch_to_buffer(pane, target)
        .map(|shown| SteelPane::new(shown).into_steel_val())
        .map_err(generic_err)
}

/// `(show-read-only-buffer! pane name text)` → the pane, now showing the
/// view.
///
/// Shows `text` on `pane`'s own pane in a read-only buffer labelled
/// `[name]`, cursor on the first line, recording no jump. A later call with
/// the same `name` replaces the text in that same buffer. Raises for an
/// empty `name`, one the editor's own views use (`messages`,
/// `plugin-status`, `lsp-status`), or a `pane` that `switch-to-buffer!`
/// would also refuse.
pub(crate) fn show_read_only_buffer(
    ctx: &mut SteelCtx,
    pane: ShownHandle,
    name: String,
    text: String,
) -> SteelResult {
    ctx.host
        .buffers()
        .show_read_only_buffer(pane, &name, &text)
        .map(|shown| SteelPane::new(shown).into_steel_val())
        .map_err(generic_err)
}

// ── Live cursor/selection reads ───────────────────────────────────────────────

/// `(buffer-cursor-line pane)` → 0-indexed content line of the primary
/// cursor in `pane`'s own pane.
///
/// Reads live state: reflects any synchronous edits or motions that ran
/// earlier in the same Steel eval (e.g. after `(move-left)`).
pub(crate) fn buffer_cursor_line(ctx: &mut SteelCtx, pane: ShownHandle) -> SteelResult {
    Ok(SteelVal::IntV(
        ctx.host
            .cursor()
            .buffer_cursor_line(pane)
            .map_err(generic_err)?
            .index() as isize,
    ))
}

/// `(buffer-selections pane)` → list of `(anchor head start end primary?)`
/// per selection in `pane`'s own pane, each built by [`selection_entry`].
/// Sorted by selection start, with one `primary?` = `#t`.
pub(crate) fn buffer_selections(ctx: &mut SteelCtx, pane: ShownHandle) -> SteelResult {
    let sels = ctx
        .host
        .cursor()
        .buffer_selections(pane)
        .map_err(generic_err)?;
    Ok(list_of(sels.into_iter().map(selection_entry)))
}

/// One `buffer-selections` entry, `(anchor head start end primary?)`: a
/// positional list rather than a hash, since a hash costs a hasher and a trie
/// node per selection. The `selection-*` builtins are its readers, through
/// [`decode_selection`], its inverse. Offsets are 0-indexed chars, direction
/// preserved (anchor > head when backward); `start`..`end` is what the
/// selection covers, `end` exclusive.
fn selection_entry(sel: SelectionInfo) -> SteelVal {
    list_of([
        SteelVal::IntV(sel.anchor as isize),
        SteelVal::IntV(sel.head as isize),
        SteelVal::IntV(sel.start as isize),
        SteelVal::IntV(sel.end as isize),
        SteelVal::BoolV(sel.primary),
    ])
}

/// Reads back a [`selection_entry`]; raises for any other value.
fn decode_selection(entry: &SteelVal) -> Result<SelectionInfo, SteelErr> {
    let offset = |val: Option<&SteelVal>| match val {
        Some(SteelVal::IntV(n)) => usize::try_from(*n).ok(),
        _ => None,
    };
    if let SteelVal::ListV(list) = entry
        && let mut fields = list.iter()
        && let (
            Some(anchor),
            Some(head),
            Some(start),
            Some(end),
            Some(SteelVal::BoolV(primary)),
            None,
        ) = (
            offset(fields.next()),
            offset(fields.next()),
            offset(fields.next()),
            offset(fields.next()),
            fields.next(),
            fields.next(),
        )
    {
        return Ok(SelectionInfo {
            anchor,
            head,
            start,
            end,
            primary: *primary,
        });
    }
    steel::stop!(TypeMismatch =>
        "expected a buffer-selections entry (anchor head start end primary?), got {}", entry)
}

/// `(selection-anchor sel)`: the char offset `sel` is anchored at.
pub(crate) fn selection_anchor(sel: SteelVal) -> SteelResult {
    Ok(SteelVal::IntV(decode_selection(&sel)?.anchor as isize))
}

/// `(selection-head sel)`: the char offset of `sel`'s head, its cursor.
pub(crate) fn selection_head(sel: SteelVal) -> SteelResult {
    Ok(SteelVal::IntV(decode_selection(&sel)?.head as isize))
}

/// `(selection-start-char sel)`: the first char offset `sel` covers.
pub(crate) fn selection_start_char(sel: SteelVal) -> SteelResult {
    Ok(SteelVal::IntV(decode_selection(&sel)?.start as isize))
}

/// `(selection-end-char sel)`: the char offset one past the last one `sel`
/// covers.
pub(crate) fn selection_end_char(sel: SteelVal) -> SteelResult {
    Ok(SteelVal::IntV(decode_selection(&sel)?.end as isize))
}

/// `(selection-primary? sel)`: whether `sel` is the primary selection.
pub(crate) fn selection_primary(sel: SteelVal) -> SteelResult {
    Ok(SteelVal::BoolV(decode_selection(&sel)?.primary))
}

/// `(char-offset->line pane idx)` → 0-indexed content line containing 0-indexed
/// char offset `idx` in `pane`'s buffer's live text; `idx` equal to the
/// buffer length in chars is on the last content line. Raises when `idx` is
/// past the buffer length, and on a stale buffer, same
/// liveness contract as every other explicit-pane builtin, checked at
/// resolve time (`LiveBuffer`) after `idx` is decoded, so a malformed
/// `idx` still raises its own error on a stale buffer rather than being
/// masked behind the liveness one (steel-core decodes `idx`'s own `Usize`
/// argument type first regardless, left to right, but the ordering holds
/// either way).
pub(crate) fn char_offset_to_line(ctx: &mut SteelCtx, bid: BufferId, idx: usize) -> SteelResult {
    let line = ctx
        .host
        .cursor()
        .char_offset_to_line(bid, idx)
        .map_err(generic_err)?;
    Ok(SteelVal::IntV(line.index() as isize))
}

/// `(line->char-offset pane line)` → 0-based char offset where 0-based content
/// `line` starts in `pane`'s buffer's live text. Raises on a stale buffer or
/// a `line` past the content line count, same bounds contract as
/// `buffer-lines` (raises rather than clamping).
///
/// Named for the conversion family (`char-offset->line`, `lsp-position->char-offset`,
/// `lsp-range->char-range`, `path->display`), not the `buffer-text`/
/// `buffer-lines` accessor family.
pub(crate) fn line_to_char_offset(ctx: &mut SteelCtx, bid: BufferId, line: usize) -> SteelResult {
    let invalid_id = || not_live_err("pane", bid);
    let line_count = ctx
        .host
        .buffers()
        .buffer_line_count(bid)
        .ok_or_else(invalid_id)?;
    if line >= line_count {
        return Err(generic_err(format!(
            "line {line} is out of range (buffer has {line_count} content lines)"
        )));
    }
    // Trusted mint: the check above is what licenses ContentLine::new here.
    let line = hume_rope::line::ContentLine::new(line);
    let offset = ctx
        .host
        .buffers()
        .line_to_char_offset(bid, line)
        .ok_or_else(invalid_id)?;
    Ok(SteelVal::IntV(offset as isize))
}

/// `(viewport-range pane)` → `(hash 'start-line first-line 'end-line end-line)` visible in
/// `pane`'s own pane: 0-based, end-exclusive, matching `buffer-lines`'
/// range convention. Raises when `pane` carries no pane, a closed one, or
/// one that no longer shows `pane`'s buffer. Answers for a background-tab
/// pane too, but its scroll position may lag until that tab is next
/// focused (see `EditorHostImpl::viewport_range`'s own doc). Reads live
/// view state, which only exists at command dispatch, hook fire, or a
/// queued-call drain.
pub(crate) fn viewport_range(ctx: &mut SteelCtx, pane: ShownHandle) -> SteelResult {
    let range = ctx
        .host
        .buffers()
        .viewport_range(pane)
        .map_err(generic_err)?;
    Ok(line_range_value(range))
}

/// `(selections-linewise? pane)`.
pub(crate) fn selections_linewise(ctx: &mut SteelCtx, pane: ShownHandle) -> SteelResult {
    Ok(SteelVal::BoolV(
        ctx.host
            .cursor()
            .selections_linewise(pane)
            .map_err(generic_err)?,
    ))
}

/// `(selections-charwise? pane)`.
pub(crate) fn selections_charwise(ctx: &mut SteelCtx, pane: ShownHandle) -> SteelResult {
    Ok(SteelVal::BoolV(
        ctx.host
            .cursor()
            .selections_charwise(pane)
            .map_err(generic_err)?,
    ))
}

/// `(symbol-under-cursor pane)`.
pub(crate) fn symbol_under_cursor(ctx: &mut SteelCtx, pane: ShownHandle) -> SteelResult {
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
