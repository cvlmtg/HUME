//! Character/string insertion, auto-indent on Enter/`o`/`O`, and Tab.

use hume_editing::edit::Edited;
use hume_editing::edit::Landing;
use hume_editing::indent::BreakIndent;
use hume_editing::selection::EditView;
use hume_editing::state::EditState;
use hume_editing::tab_style::TabStyle;
use hume_editing::text::{BufferText, TextVersion};
use hume_rope::cluster::{ClusterRange, ClusterStart};
use hume_rope::line::ContentLine;
use hume_rope::offset::{CharOffset, ExclusiveRange};
use hume_rope::width::{indent_stop, str_width, tab_advance};

use super::apply_edit;
use crate::auto_pairs::{Pair, pair_around};

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
    indent.ws_end > indent.start && indent.is_blank()
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

    /// Whether the line holds nothing but blanks before its `\n`, an empty
    /// line included.
    pub(in crate::edit) fn is_blank(self) -> bool {
        self.ws_end == self.line_break
    }
}

/// The indent of the line containing `pos`. Single source of truth for that
/// computation: every caller that needs a line's indent bounds (the
/// blank-line ownership check below, the indent a line break copies and
/// the line start a break above goes in at, electric dedent's check that a
/// line still holds its armed indent, and the sibling test module's
/// `owns_every_line` stand-in for `arm_autoindent`) goes through this
/// instead of re-deriving it.
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
/// vacate": [`owned_blank_indent`] (the editor's exit pre-flight check),
/// [`plan_line_breaks`] (a break vacating it) and [`clear_blank_line_indent`]
/// (the trim itself) all read this, so gate, break and trim can never drift
/// on what counts as owned.
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

/// Where a line break goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineBreak {
    /// Split the line at each cursor, or replace each selection with a
    /// break (Enter, `o`).
    AtCursor,
    /// Open a blank line above each selection's line (`O`). Every
    /// selection must be collapsed and on its own line.
    AboveLine,
}

/// The line breaks [`plan_line_breaks`] computed, for the text it read.
#[derive(Debug)]
pub struct LineBreakPlan {
    version: TextVersion,
    breaks: Vec<PlannedBreak>,
}

/// One selection's break: `removed` is deleted, then `lead` and `tail` are
/// inserted at `at`, and the cursor lands between them.
#[derive(Debug)]
struct PlannedBreak {
    removed: Option<ClusterRange>,
    at: ClusterStart,
    lead: String,
    tail: String,
}

/// How [`plan_line_breaks`] indents a new line.
pub enum NewLineIndent<F> {
    /// Every new line starts at the left margin.
    LeftMargin,
    /// A new line copies the leading whitespace of the line it breaks,
    /// unless the provider answers for the split point: the cursor for
    /// [`LineBreak::AtCursor`], the previous line's `\n` for
    /// [`LineBreak::AboveLine`]. It is asked about the pre-edit text, in
    /// selection order, and never for:
    /// - a non-collapsed selection;
    /// - a cursor at or before the first non-blank character of a line with
    ///   content;
    /// - the first line, for `AboveLine`.
    Smart(F),
}

impl<F: FnMut(ClusterStart) -> Option<BreakIndent>> NewLineIndent<F> {
    /// The provider's answer for `split`; always `None` for
    /// [`NewLineIndent::LeftMargin`].
    fn computed(&mut self, split: ClusterStart) -> Option<BreakIndent> {
        match self {
            Self::LeftMargin => None,
            Self::Smart(indent_at) => indent_at(split),
        }
    }

    /// What a new line starts with when nothing is computed for it: the
    /// leading whitespace of `broken`, the line it breaks, or nothing.
    fn fallback(&self, text: &BufferText, broken: LineIndent) -> String {
        match self {
            Self::LeftMargin => String::new(),
            Self::Smart(_) => text.slice(broken.chars()).to_string(),
        }
    }
}

/// Plan a line break for each selection of `view`, each new line indented
/// as `new_indent` says.
///
/// A computed width is rendered in `style`. With `after_token` set and a
/// cursor between the halves of an asymmetric pair of `pairs`, the break
/// opens a block: an inner line one level deeper, then the closer on a line
/// of its own.
///
/// `allowed`: per-selection (by sorted index) range of whitespace some
/// earlier auto-indent recorded as its own; see `owned_indent`. A collapsed
/// cursor on a blank line whose whitespace is owned vacates it. Empty (or an
/// index with no entry) when nothing is owned yet.
pub fn plan_line_breaks(
    view: EditView<'_>,
    place: LineBreak,
    allowed: &[ExclusiveRange<CharOffset>],
    pairs: &[Pair],
    style: TabStyle,
    tab_width: u8,
    mut new_indent: NewLineIndent<impl FnMut(ClusterStart) -> Option<BreakIndent>>,
) -> LineBreakPlan {
    let text = view.text();
    let breaks = view
        .iter()
        .map(|sel| {
            let indent = line_indent_range(text, sel.start().offset());
            match place {
                LineBreak::AtCursor if !sel.is_cursor() => PlannedBreak {
                    removed: Some(sel.covered()),
                    at: sel.start(),
                    lead: format!("\n{}", new_indent.fallback(text, indent)),
                    tail: String::new(),
                },
                LineBreak::AtCursor => {
                    let head = sel.head();
                    let before_content =
                        !indent.is_blank() && head.offset() <= indent.ws_end.offset();
                    let computed = if before_content {
                        None
                    } else {
                        new_indent.computed(head)
                    };
                    let (lead, tail) = match computed {
                        None => (
                            format!("\n{}", new_indent.fallback(text, indent)),
                            String::new(),
                        ),
                        Some(BreakIndent { width, after_token }) => {
                            let new_line =
                                |width| format!("\n{}", style.render_indent(width, tab_width));
                            let opens_block = after_token
                                && pair_around(text, head, pairs)
                                    .is_some_and(|p| !p.is_symmetric());
                            if opens_block {
                                let inner = width.advance_saturating(indent_stop(1, tab_width));
                                (new_line(inner), new_line(width))
                            } else {
                                (new_line(width), String::new())
                            }
                        }
                    };
                    PlannedBreak {
                        removed: owned_indent(text, indent, allowed.get(sel.index()).copied()),
                        at: sel.start(),
                        lead,
                        tail,
                    }
                }
                LineBreak::AboveLine => {
                    debug_assert!(
                        sel.is_cursor(),
                        "plan_line_breaks: a line break above needs collapsed selections"
                    );
                    let line = text.char_to_line(sel.start().offset());
                    let computed = (line != ContentLine::default())
                        .then(|| {
                            new_indent.computed(text.lines().newline(line.retreat_saturating(1)))
                        })
                        .flatten();
                    PlannedBreak {
                        removed: None,
                        at: indent.start,
                        lead: computed.map_or_else(
                            || new_indent.fallback(text, indent),
                            |c| style.render_indent(c.width, tab_width),
                        ),
                        tail: "\n".to_owned(),
                    }
                }
            }
        })
        .collect();
    LineBreakPlan {
        version: text.version(),
        breaks,
    }
}

/// Apply `plan` to `state`. Each cursor lands at the end of its break's
/// leading part: on the first char after a copied or computed indent, or on
/// the new blank line's `\n` for [`LineBreak::AboveLine`].
///
/// # Panics
/// Panics if `plan` was computed for another text or another number of
/// selections.
pub fn apply_line_break_plan(state: EditState, plan: LineBreakPlan) -> Edited {
    assert!(
        state.text().version() == plan.version,
        "apply_line_break_plan: the plan was computed for another text"
    );
    assert!(
        state.view().len() == plan.breaks.len(),
        "apply_line_break_plan: the plan has {} breaks for {} selections",
        plan.breaks.len(),
        state.view().len()
    );
    let mut breaks = plan.breaks.into_iter();
    apply_edit(state, |b, _| {
        let planned = breaks.next().expect("one break per selection");
        if let Some(removed) = planned.removed {
            b.delete(removed);
        }
        let mark = b.insert(planned.at, &planned.lead);
        if !planned.tail.is_empty() {
            b.insert(planned.at, &planned.tail);
        }
        Landing::cursor(mark.end())
    })
}

/// Clear a blank, auto-indented line's leading whitespace at every collapsed
/// selection sitting on one. Leaves the cursor on the line's structural `\n`.
///
/// The Esc/Ctrl-c half of vim autoindent parity: [`plan_line_breaks`]
/// handles trimming on Enter, this handles trimming when Insert mode exits
/// with the cursor still on a blank auto-indented line (`:help autoindent`:
/// "type `<Esc>` ... the indent is deleted again"). Selections not on a blank
/// line are left untouched (identity edit).
///
/// `allowed`: the per-selection ownership record [`plan_line_breaks`]
/// reads, read here too.
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
