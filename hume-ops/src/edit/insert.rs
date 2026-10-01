//! Character/string insertion, auto-indent on Enter/`o`/`O`, and Tab.

use hume_editing::edit::Edited;
use hume_editing::edit::Landing;
use hume_editing::state::EditState;
use hume_editing::tab_style::TabStyle;
use hume_editing::text::BufferText;
use hume_rope::cluster::{ClusterRange, ClusterStart};
use hume_rope::line::ContentLine;
use hume_rope::offset::{CharOffset, ExclusiveRange};
use hume_rope::width::{str_width, tab_advance};

use super::apply_edit;

/// Insert `ch` at every selection.
///
/// - **Cursor**: `ch` goes before the cursor's cluster, and the cursor stays
///   on that cluster.
/// - **Selection**: what it covers is deleted first (never the structural
///   `\n`), then `ch` goes in its place, and the cursor lands after it.
///
/// This covers single-cursor typing, multicursor typing, and "replace
/// selection with typed character", all via the same loop.
pub fn insert_char(state: EditState, ch: char) -> Edited {
    insert_str(state, ch.encode_utf8(&mut [0; 4]))
}

/// Insert `inserted` at every selection: the bulk-string counterpart of
/// [`insert_char`], used for pasted text so a paste is one edit rather than
/// one `insert_char` call per character.
pub fn insert_str(state: EditState, inserted: &str) -> Edited {
    apply_edit(state, |b, sel| {
        let mark = if sel.is_cursor() {
            b.insert(sel.start(), inserted)
        } else {
            b.replace(sel.covered(), inserted)
        };
        Landing::cursor(mark.end())
    })
}

/// Returns `true` if `line` has leading whitespace and nothing else before its
/// `\n`: a blank, auto-indented line with no real content.
fn is_blank_indented_line(indent: LineIndent) -> bool {
    indent.ws_end > indent.start && indent.ws_end == indent.line_break
}

/// Where a line starts, where its leading whitespace ends, and its `\n`.
#[derive(Clone, Copy)]
pub(in crate::edit) struct LineIndent {
    start: ClusterStart,
    ws_end: ClusterStart,
    line_break: ClusterStart,
}

impl LineIndent {
    /// The leading whitespace as chars; empty when the line has none.
    pub(in crate::edit) fn chars(self) -> ExclusiveRange<CharOffset> {
        ExclusiveRange::new(self.start.offset(), self.ws_end.offset())
    }
}

/// The indent of the line containing `pos`. Single source of truth for that
/// computation: every caller that needs a line's indent bounds (the
/// blank-line ownership check below, `O`'s own indent copy, and the sibling
/// test module's `owns_every_line` stand-in for `arm_autoindent`) goes
/// through this instead of re-deriving it.
pub(in crate::edit) fn line_indent_range(text: &BufferText, pos: CharOffset) -> LineIndent {
    let line_idx = text.char_to_line(pos);
    LineIndent {
        start: text.lines().start(line_idx),
        ws_end: text.lines().indent_end(line_idx),
        line_break: text.lines().newline(line_idx),
    }
}

/// The indent `[line_start, ws_end)` when it is a blank, auto-indented line (see
/// [`is_blank_indented_line`]) AND that whitespace lies entirely within
/// `allowed`, the range some insert session recorded as its own
/// auto-inserted indent, in `pos`'s coordinate space.
///
/// Containment, not equality of the whole range: `line_start == allowed.start`
/// pins this to the *same* line the record was armed for: a cursor motion
/// off that line leaves `allowed` pointing at a now-unrelated offset, so the
/// check fails without anything having to invalidate the record. Meanwhile
/// `ws_end <= allowed.end` lets the whitespace *shrink* (a Backspace back
/// toward `line_start`) without losing ownership, but never lets it exceed
/// what the session itself inserted (typed content stays outside `allowed`
/// once `ChangeSet::map_ranges`' `Assoc::Before` end-mapping pins the record
/// short of it; see `apply_doc_edit_grouped`'s own comment).
///
/// Single source of truth for "is this whitespace the session's own to
/// vacate": [`owned_blank_indent`] (the editor's exit pre-flight check) and
/// [`clear_blank_line_indent`] (the trim itself) both read this, so gate and trim
/// can never drift on what counts as owned.
fn owned_indent(
    text: &BufferText,
    indent: LineIndent,
    allowed: Option<ExclusiveRange<CharOffset>>,
) -> Option<ClusterRange> {
    let allowed = allowed?;
    let owned = is_blank_indented_line(indent)
        && indent.start.offset() == allowed.start
        && indent.ws_end.offset() <= allowed.end;
    if !owned {
        return None;
    }
    ClusterRange::between(text.full_slice(), indent.start, indent.ws_end.into())
}

/// `Some(range)` (`[line_start, ws_end)`) if `pos` sits on a blank line
/// whose whitespace is owned by `allowed` (see `owned_indent`, this
/// module), `None` otherwise.
pub fn owned_blank_indent(
    text: &BufferText,
    pos: CharOffset,
    allowed: Option<ExclusiveRange<CharOffset>>,
) -> Option<ExclusiveRange<CharOffset>> {
    owned_indent(text, line_indent_range(text, pos), allowed).map(ClusterRange::chars)
}

/// Insert a newline followed by the current line's leading whitespace at every
/// selection.
///
/// This is auto-indent on Enter: the indent of the line containing each
/// selection's `start` is copied verbatim onto the new line (no smart
/// indent). Computed on the pre-edit buffer.
///
/// Cursor placement matches `insert_char`'s "stay on the original char" rule:
/// the cursor lands on the first char after the inserted indent. For a
/// collapsed cursor that is the char that was at `start`; for a selection,
/// which the line break replaces, it is the char that followed the selection
/// (the structural `\n` when the selection reached the end of the buffer).
///
/// `allowed`: per-selection (by sorted index) range of whitespace some
/// earlier auto-indent recorded as its own; see `owned_indent` (this
/// module). If a collapsed cursor's blank line is owned by its entry, that
/// whitespace is vacated instead of retained, matching vim's `:help
/// autoindent` behavior on Enter. Empty (or an index with no entry) for the
/// first Enter on an already-blank line: nothing to vacate yet, since no
/// earlier session inserted it.
pub fn insert_newline_indent(state: EditState, allowed: &[ExclusiveRange<CharOffset>]) -> Edited {
    apply_edit(state, |b, sel| {
        let start = sel.start();
        let indent = line_indent_range(b.text(), start.offset());
        let inserted = format!("\n{}", b.text().slice(indent.chars()));
        let mark = if sel.is_cursor() {
            let owned = owned_indent(b.text(), indent, allowed.get(sel.index()).copied());
            if let Some(owned) = owned {
                b.delete(owned);
            }
            b.insert(start, &inserted)
        } else {
            b.replace(sel.covered(), &inserted)
        };
        Landing::cursor(mark.end())
    })
}

/// Open a blank, auto-indented line above each selection's line.
///
/// The `O` counterpart of [`insert_newline_indent`]: both split a line and
/// copy its leading whitespace, but `O` keeps the copy on the line *above*
/// the break, so the indent is inserted before the `\n`, not after it. The
/// cursor lands on that inserted `\n`, the new blank line.
///
/// Unlike `insert_newline_indent`, this never deletes: each selection only
/// inserts at its own line's start.
///
/// Two preconditions its only caller (`cmd_open_line_above`) satisfies but
/// this function does not enforce: every selection must already be
/// collapsed. Unlike every sibling insertion op in this module, a
/// non-collapsed selection here is neither deleted nor preserved: the
/// resulting cursor orphans it. Also at most one
/// selection per line: two cursors on the same line each open their own
/// blank line above it, rather than sharing one the way vim/Helix do. The
/// caller supplies both: `cmd_goto_line_start` collapses every selection to
/// its line start first, and the selection set's overlap merge folds
/// same-line cursors into one before this ever runs.
pub fn open_line_above(state: EditState) -> Edited {
    apply_edit(state, |b, sel| {
        let indent = line_indent_range(b.text(), sel.start().offset());
        let copied = b.text().slice(indent.chars()).to_string();
        let mark = b.insert(indent.start, &copied);
        b.insert(indent.start, "\n");
        Landing::cursor(mark.end())
    })
}

/// Clear a blank, auto-indented line's leading whitespace at every collapsed
/// selection sitting on one. Leaves the cursor on the line's structural `\n`.
///
/// The Esc/Ctrl-c half of vim autoindent parity: [`insert_newline_indent`]
/// handles trimming on Enter, this handles trimming when Insert mode exits
/// with the cursor still on a blank auto-indented line (`:help autoindent`:
/// "type `<Esc>` ... the indent is deleted again"). Selections not on a blank
/// line are left untouched (identity edit).
///
/// `allowed`: see [`insert_newline_indent`]'s own doc. Same per-selection
/// ownership record, read here instead of armed.
pub fn clear_blank_line_indent(state: EditState, allowed: &[ExclusiveRange<CharOffset>]) -> Edited {
    apply_edit(state, |b, sel| {
        if !sel.is_cursor() {
            return Landing::kept(sel.selection());
        }
        let indent = line_indent_range(b.text(), sel.head().offset());
        match owned_indent(b.text(), indent, allowed.get(sel.index()).copied()) {
            Some(owned) => Landing::cursor(b.delete(owned)),
            None => Landing::kept(sel.selection()),
        }
    })
}

/// Insert a tab at every selection, governed by `style` and `tab_width`.
///
/// - **`TabStyle::Hard`**: delegates to `insert_char(.., '\t')`, same as
///   typing any other character.
/// - **`TabStyle::Soft`**: inserts enough spaces to reach the next tab stop.
///   The display column of the cursor is computed with tab expansion (see
///   [`hume_editing::text::ColumnView::display_col`]); `spaces = tab_width -
///   (display_col % tab_width)`, so a cursor already on a stop gets a full
///   tab-width of spaces.
///
/// Non-collapsed selections are deleted first, same as `insert_char`: Tab
/// over a selection replaces it, just like typing any other key.
pub fn insert_tab(state: EditState, style: TabStyle, tab_width: u8) -> Edited {
    if style == TabStyle::Hard {
        return insert_char(state, '\t');
    }
    // A selection's deletion can join lines, so tab stops are read from the
    // text left once every selection is cleared.
    insert_str(state, "").then(|state| spaces_to_stops(state, tab_width))
}

/// Spaces to the next tab stop at every cursor. An earlier cursor on the same
/// line leaves its column on a tab stop, so a later one is measured from
/// there across the text between them: a tab in between expands from that
/// stop, not from where it stood before the spaces went in.
fn spaces_to_stops(state: EditState, tab_width: u8) -> Edited {
    // The previous cursor's line, where it inserted, and the column it left.
    let mut prev: Option<(ContentLine, CharOffset, usize)> = None;
    apply_edit(state, |b, sel| {
        let text = b.text();
        let at = sel.start();
        let line = text.char_to_line(at.offset());
        let col = match prev {
            Some((prev_line, prev_at, prev_col)) if prev_line == line => {
                let between = text.slice(ExclusiveRange::new(prev_at, at.offset()));
                prev_col + str_width(&between.to_string(), prev_col, tab_width)
            }
            _ => text
                .columns()
                .display_col(line, at.offset(), tab_width)
                .get() as usize,
        };
        let n = tab_advance(col, tab_width);
        prev = Some((line, at.offset(), col + n));
        let mark = b.insert(at, &" ".repeat(n));
        Landing::cursor(mark.end())
    })
}
