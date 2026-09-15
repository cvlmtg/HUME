//! Character/string insertion, auto-indent on Enter/`o`/`O`, and Tab.

use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::grapheme::display_col_in_line;
use hume_editing::lines::{leading_whitespace_end, next_line_start};
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::tab_style::TabStyle;
use hume_editing::text::BufferText;
use hume_rope::offset::{CharOffset, ExclusiveRange};

use super::apply_edit;

/// Insert `ch` at every selection.
///
/// - **Single-character selection**: `ch` is inserted before the cursor
///   character; the cursor advances to land on the character that follows it.
/// - **Multi-character selection**: the selected region is deleted first, then
///   `ch` is inserted at the start of the former selection. The cursor lands
///   one past the inserted character.
///
/// This covers single-cursor typing, multicursor typing, and "replace
/// selection with typed character" — all via the same loop.
pub fn insert_char(
    text: BufferText,
    sels: SelectionSet,
    ch: char,
) -> (BufferText, SelectionSet, ChangeSet) {
    apply_edit(text, sels, |b, text, _i, sel, new_sels| {
        let start = sel.start();
        b.retain(start.chars_since(b.old_pos()));
        if !sel.is_collapsed() {
            b.delete(sel.content_end_exclusive(text).chars_since(start));
        }
        b.insert_char(ch);
        // new_pos() is one past the inserted char — the cursor sits on the
        // character that was originally at `start` (now shifted right by 1).
        let sel = Selection::collapsed(b.new_pos());
        new_sels.push(sel);
    })
}

/// Insert `text` at every selection — the bulk-string counterpart of
/// [`insert_char`], used for pasted text so a paste is one edit rather than
/// one `insert_char` call per character.
///
/// Same shape as `insert_char`: single-character selections get `inserted`
/// inserted before the cursor; non-collapsed selections are replaced. The
/// cursor lands at `new_pos()` (one past the inserted text) in both cases —
/// no manual position arithmetic, so a multi-char `inserted` can't land mid
/// grapheme-cluster.
pub fn insert_str(
    text: BufferText,
    sels: SelectionSet,
    inserted: &str,
) -> (BufferText, SelectionSet, ChangeSet) {
    apply_edit(text, sels, |b, text, _i, sel, new_sels| {
        let start = sel.start();
        b.retain(start.chars_since(b.old_pos()));
        if !sel.is_collapsed() {
            b.delete(sel.content_end_exclusive(text).chars_since(start));
        }
        b.insert(inserted);
        let sel = Selection::collapsed(b.new_pos());
        new_sels.push(sel);
    })
}

/// Returns `true` if `line` has leading whitespace and nothing else before its
/// structural newline — a blank, auto-indented line with no real content.
///
/// `ws_end` (from [`leading_whitespace_end`]) lands exactly on the line's `\n`
/// when the line is whitespace-only: the scan only stops early on a
/// non-whitespace char, and every line's char content ends in `\n` (buffer
/// invariant), so a whitespace-only line is the one case where the scan runs
/// all the way to that `\n` without finding one.
fn is_blank_indented_line(text: &BufferText, line_start: CharOffset, ws_end: CharOffset) -> bool {
    ws_end > line_start && text.char_at(ws_end) == Some('\n')
}

/// `[line_start, ws_end)` — the leading-whitespace range of the line
/// containing `pos`. Single source of truth for that computation: every
/// caller below that needs a line's indent bounds — the blank-line ownership
/// check, the "already consumed by a prior selection" guard, and `O`'s own
/// indent copy — goes through this instead of re-deriving it.
fn line_indent_range(text: &BufferText, pos: CharOffset) -> ExclusiveRange<CharOffset> {
    let line_idx = text.char_to_line(pos);
    let line_start = text.line_to_char(line_idx.into());
    let ws_end = leading_whitespace_end(text, line_idx);
    ExclusiveRange::new(line_start, ws_end)
}

/// `true` if `[line_start, ws_end)` is a blank, auto-indented line (see
/// [`is_blank_indented_line`]) AND that whitespace lies entirely within
/// `allowed` — the range some insert session recorded as its own
/// auto-inserted indent, in `pos`'s coordinate space.
///
/// Containment, not equality of the whole range: `line_start == allowed.start`
/// pins this to the *same* line the record was armed for — a cursor motion
/// off that line leaves `allowed` pointing at a now-unrelated offset, so the
/// check fails without anything having to invalidate the record — while
/// `ws_end <= allowed.end` lets the whitespace *shrink* (a Backspace back
/// toward `line_start`) without losing ownership, but never lets it exceed
/// what the session itself inserted (typed content stays outside `allowed`
/// once `ChangeSet::map_ranges`' `Assoc::Before` end-mapping pins the record
/// short of it — see `apply_doc_edit_grouped`'s own comment).
///
/// Single source of truth for "is this whitespace the session's own to
/// vacate" — [`owned_blank_indent`] (the editor's exit pre-flight check) and
/// [`try_trim_blank_line`] (the trim itself) both read this, so gate and trim
/// can never drift on what counts as owned.
fn is_owned_blank_line(
    text: &BufferText,
    line_start: CharOffset,
    ws_end: CharOffset,
    allowed: Option<ExclusiveRange<CharOffset>>,
) -> bool {
    let Some(allowed) = allowed else {
        return false;
    };
    is_blank_indented_line(text, line_start, ws_end)
        && line_start == allowed.start
        && ws_end <= allowed.end
}

/// `Some(range)` — `[line_start, ws_end)` — if `pos` sits on a blank line
/// whose whitespace is owned by `allowed` (see `is_owned_blank_line`, this
/// module) — `None` otherwise.
pub fn owned_blank_indent(
    text: &BufferText,
    pos: CharOffset,
    allowed: Option<ExclusiveRange<CharOffset>>,
) -> Option<ExclusiveRange<CharOffset>> {
    let range = line_indent_range(text, pos);
    is_owned_blank_line(text, range.start, range.end, allowed).then_some(range)
}

/// Shared per-selection prelude for [`insert_newline_indent`] and
/// [`clear_blank_line_indent`]: `pos`'s line info as `[line_start, ws_end)`,
/// or `None` if a prior selection's blank-line clear already consumed past
/// `pos` (two cursors on the same whitespace-only line) — in that case the
/// caller should land the cursor at `b.new_pos()` and emit nothing further,
/// rather than retaining backwards past what the builder already emitted.
fn line_context_if_unconsumed(
    b: &ChangeSetBuilder,
    text: &BufferText,
    pos: CharOffset,
) -> Option<ExclusiveRange<CharOffset>> {
    if pos < b.old_pos() {
        return None;
    }
    Some(line_indent_range(text, pos))
}

/// Attempts the blank-line whitespace-vacate trim for a collapsed selection.
///
/// Returns `true` (and emits `retain` + `delete` into `b`) when `sel` is
/// collapsed, its line's whitespace is owned by `allowed` (see
/// [`is_owned_blank_line`]), and `line_start` has not already been passed by
/// a prior selection's edits in this pass (`line_start >= b.old_pos()`): two
/// cursors can land on the *same* blank line (one mid-whitespace, one on the
/// trailing `\n`), and the first cursor's delete can advance `old_pos()` past
/// this cursor's `line_start`, which would otherwise underflow the `retain`.
/// When that happens, the caller falls back to its non-blank arm instead
/// (retaining forward to its own position, which is always safe since `pos >=
/// b.old_pos()` per [`line_context_if_unconsumed`]).
fn try_trim_blank_line(
    b: &mut ChangeSetBuilder,
    text: &BufferText,
    sel: &Selection,
    line_start: CharOffset,
    ws_end: CharOffset,
    allowed: Option<ExclusiveRange<CharOffset>>,
) -> bool {
    if !sel.is_collapsed()
        || !is_owned_blank_line(text, line_start, ws_end, allowed)
        || line_start < b.old_pos()
    {
        return false;
    }
    b.retain(line_start.chars_since(b.old_pos()));
    b.delete(ws_end.chars_since(line_start));
    true
}

/// Insert a newline followed by the current line's leading whitespace at every
/// selection.
///
/// This is auto-indent on Enter: the indent of the line containing each
/// selection's `start` is copied verbatim onto the new line (no smart
/// indent). Computed on the pre-edit buffer.
///
/// Cursor placement matches `insert_char`'s "stay on the original char" rule:
/// collapsed cursor lands on the first char after the inserted indent (the
/// original char at `start`, shifted down); non-collapsed selection has no
/// original char to land on, so the cursor ends up on the structural `\n`
/// left at the original position.
///
/// `allowed`: per-selection (by sorted index) range of whitespace some
/// earlier auto-indent recorded as its own — see `is_owned_blank_line` (this
/// module). If a collapsed cursor's blank line is owned by its entry, that
/// whitespace is vacated instead of retained, matching vim's `:help
/// autoindent` behavior on Enter. Empty (or an index with no entry) for the
/// first Enter on an already-blank line — nothing to vacate yet, since no
/// earlier session inserted it.
pub fn insert_newline_indent(
    text: BufferText,
    sels: SelectionSet,
    allowed: &[ExclusiveRange<CharOffset>],
) -> (BufferText, SelectionSet, ChangeSet) {
    apply_edit(text, sels, |b, text, i, sel, new_sels| {
        let start = sel.start();
        let Some(line) = line_context_if_unconsumed(b, text, start) else {
            new_sels.push(Selection::collapsed(b.new_pos()));
            return;
        };

        if !try_trim_blank_line(b, text, sel, line.start, line.end, allowed.get(i).copied()) {
            b.retain(start.chars_since(b.old_pos()));
            if !sel.is_collapsed() {
                b.delete(sel.content_end_exclusive(text).chars_since(start));
            }
        }
        let indent = text.slice(line).to_string();
        b.insert_char('\n');
        b.insert(&indent);
        new_sels.push(Selection::collapsed(b.new_pos()));
    })
}

/// Open a blank, auto-indented line above each selection's line.
///
/// The `O` counterpart of [`insert_newline_indent`]: both split a line and
/// copy its leading whitespace, but `O` keeps the copy on the line *above*
/// the break, so the indent is inserted before the `\n`, not after it. The
/// cursor lands on that inserted `\n` — the new blank line.
///
/// Unlike `insert_newline_indent`, this never deletes: `apply_edit` visits
/// selections in ascending-`start()` order, and each iteration only retains
/// forward to its own line's start, so an earlier selection's edit can never
/// advance `old_pos()` past a later selection's line start the way a delete
/// could — no "already consumed" guard is needed.
///
/// Two preconditions its only caller (`cmd_open_line_above`) satisfies but
/// this function does not enforce: every selection must already be
/// collapsed — unlike every sibling insertion op in this module, a
/// non-collapsed selection here is neither deleted nor preserved, it is
/// simply orphaned by the pushed `Selection::collapsed` — and at most one
/// selection per line — two cursors on the same line each open their own
/// blank line above it, rather than sharing one the way vim/Helix do. The
/// caller supplies both: `cmd_goto_line_start` collapses every selection to
/// its line start first, and `SelectionSet::map`'s overlap merge folds
/// same-line cursors into one before this ever runs.
pub fn open_line_above(
    text: BufferText,
    sels: SelectionSet,
) -> (BufferText, SelectionSet, ChangeSet) {
    apply_edit(text, sels, |b, text, _i, sel, new_sels| {
        let line = line_indent_range(text, sel.start());
        b.retain(line.start.chars_since(b.old_pos()));
        let indent = text.slice(line).to_string();
        b.insert(&indent);
        new_sels.push(Selection::collapsed(b.new_pos()));
        b.insert_char('\n');
    })
}

/// Clear a blank, auto-indented line's leading whitespace at every collapsed
/// selection sitting on one — leaves the cursor on the line's structural `\n`.
///
/// The Esc/Ctrl-c half of vim autoindent parity: [`insert_newline_indent`]
/// handles trimming on Enter, this handles trimming when Insert mode exits
/// with the cursor still on a blank auto-indented line (`:help autoindent`:
/// "type `<Esc>` ... the indent is deleted again"). Selections not on a blank
/// line are left untouched (identity edit).
///
/// `allowed`: see [`insert_newline_indent`]'s own doc — same per-selection
/// ownership record, read here instead of armed.
pub fn clear_blank_line_indent(
    text: BufferText,
    sels: SelectionSet,
    allowed: &[ExclusiveRange<CharOffset>],
) -> (BufferText, SelectionSet, ChangeSet) {
    apply_edit(text, sels, |b, text, i, sel, new_sels| {
        if sel.is_collapsed() {
            let head = sel.head();
            let Some(line) = line_context_if_unconsumed(b, text, head) else {
                new_sels.push(Selection::collapsed(b.new_pos()));
                return;
            };
            if !try_trim_blank_line(b, text, sel, line.start, line.end, allowed.get(i).copied()) {
                b.retain(head.chars_since(b.old_pos()));
            }
            new_sels.push(Selection::collapsed(b.new_pos()));
            return;
        }

        // Non-collapsed selections are never trimmed: identity edit that
        // preserves anchor and head. `start < b.old_pos()` can only happen if
        // a prior collapsed cursor's blank-line trim reached into this
        // selection's own line; fall back to landing the cursor at
        // `new_pos()` rather than underflowing the retain.
        let start = sel.start();
        if start < b.old_pos() {
            new_sels.push(Selection::collapsed(b.new_pos()));
            return;
        }
        b.retain(start.chars_since(b.old_pos()));
        let delta = b.new_pos().index() as isize - b.old_pos().index() as isize;
        b.retain(sel.end_exclusive(text).chars_since(start));
        let new_anchor = sel.anchor().shift(delta);
        let new_head = sel.head().shift(delta);
        new_sels.push(Selection::new(new_anchor, new_head));
    })
}

/// Insert a tab at every selection, governed by `style` and `tab_width`.
///
/// - **`TabStyle::Hard`**: delegates to `insert_char(.., '\t')` — same as
///   typing any other character.
/// - **`TabStyle::Soft`**: inserts enough spaces to reach the next tab stop.
///   The display column of the cursor is computed with tab expansion (see
///   [`hume_editing::grapheme::display_col_in_line`]); `spaces = tab_width -
///   (display_col % tab_width)`, so a cursor already on a stop gets a full
///   tab-width of spaces.
///
/// Non-collapsed selections are deleted first, same as `insert_char` — Tab
/// over a selection replaces it, just like typing any other key.
pub fn insert_tab(
    text: BufferText,
    sels: SelectionSet,
    style: TabStyle,
    tab_width: u8,
) -> (BufferText, SelectionSet, ChangeSet) {
    if style == TabStyle::Hard {
        return insert_char(text, sels, '\t');
    }
    // Track the accumulated display-column shift from insertions/deletions made by
    // earlier cursors on the same line. Without this, the second cursor on a line
    // would compute its tab-stop offset from the original-buffer column, missing the
    // spaces the first cursor already inserted.
    let mut prev_line: Option<hume_rope::line::ContentLine> = None;
    let mut display_col_shift: isize = 0;
    apply_edit(text, sels, move |b, text, _i, sel, new_sels| {
        let start = sel.start();
        b.retain(start.chars_since(b.old_pos()));
        let line_idx = text.char_to_line(start);
        if prev_line != Some(line_idx) {
            display_col_shift = 0;
            prev_line = Some(line_idx);
        }
        // Compute the effective display column of the cursor after all prior
        // same-line edits. `shift_saturating` is signed (a selection deletion
        // can decrease it) and saturates at 0 rather than underflowing.
        // Walks the line prefix grapheme by grapheme, so it's measured once
        // and reused by the deletion-width computation below.
        let start_display_col = display_col_in_line(text, line_idx, start, tab_width);
        let display_col = start_display_col.shift_saturating(display_col_shift);
        if !sel.is_collapsed() {
            let del_end = sel.content_end_exclusive(text);
            // Clamp del_end to the line boundary before computing the display-column
            // width to keep display_col_shift accurate. A multi-line selection
            // (del_end on a different line) would otherwise walk past the '\n'
            // when counting columns, making display_col_shift wrong for later
            // same-line cursors.
            let line_end = next_line_start(text, line_idx.into());
            let del_end_clamped = del_end.min(line_end);
            let del_width = display_col_in_line(text, line_idx, del_end_clamped, tab_width)
                .cells_since(start_display_col);
            b.delete(del_end.chars_since(start));
            display_col_shift -= del_width as isize;
        }
        let n = hume_rope::width::tab_advance(display_col.get() as usize, tab_width);
        b.insert(&" ".repeat(n));
        display_col_shift += n as isize;
        new_sels.push(Selection::collapsed(b.new_pos()));
    })
}
