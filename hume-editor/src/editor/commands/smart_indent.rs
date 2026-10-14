//! The editor's one consumer of `hume_treesitter::indent`: Enter, `o` and
//! `O` plan their line breaks here, and a typed character or Enter dedents
//! the line it closes here.

use hume_editing::indent::ElectricTrigger;
use hume_engine::pipeline::{BufferId, EngineView};
use hume_ops::edit::{
    LineBreak, NewLineIndent, apply_electric_plan, apply_line_break_plan, electric_candidates,
    plan_electric_dedent, plan_line_breaks,
};
use hume_rope::offset::{CharOffset, ExclusiveRange};
use hume_treesitter::indent::{electric_indent, indent_for_new_line};

use super::{
    FocusedPane, apply_focused_edit_grouped, autoindent_armed, doc, effective_word_chars,
    pane_view, rearm_autoindent_lines, tab_format,
};
use crate::editor::EditorState;
use crate::editor::input_stack::InsertEntry;
use crate::editor::syntax::{SyntaxUse, ensure_syntax_current};

/// Whether `smart-indent` is on for `bid`; when it is, brings the tree up
/// to date with the text first.
fn indent_tree(state: &mut EditorState, bid: BufferId) -> bool {
    let on = state
        .buffers
        .get(bid)
        .overrides
        .smart_indent(&state.settings);
    if on {
        ensure_syntax_current(state, bid, SyntaxUse::Indent);
    }
    on
}

/// Break the line at every selection of `fp`'s pane as one grouped edit,
/// the single path Enter, `o` and `O` share. `allowed` is the session's
/// autoindent ownership record (see `autoindent_owned`).
///
/// With `smart-indent` off every new line starts at the left margin.
/// Otherwise the tree is brought current and a new line gets the width it
/// computes, or a copy of the current line's indent where it computes none
/// (no grammar, no indents query). Every break is planned against the
/// pre-edit text before the edit borrows the buffer mutably.
pub(super) fn break_line(
    state: &mut EditorState,
    view: &EngineView,
    fp: FocusedPane,
    place: LineBreak,
    allowed: &[ExclusiveRange<CharOffset>],
) {
    let smart = indent_tree(state, fp.bid(view));
    let buf = doc(state, view, fp.pane());
    let (style, tab_width) = tab_format(buf, &state.settings);
    let (pairs_enabled, pairs) = buf.overrides.auto_pairs_ref(&state.settings);
    let pairs = if pairs_enabled { pairs } else { &[] };
    let new_indent = if smart {
        let layers = buf.current_layers();
        NewLineIndent::Smart(move |split| {
            layers.and_then(|l| indent_for_new_line(l, buf.text(), split, tab_width))
        })
    } else {
        NewLineIndent::LeftMargin
    };
    let plan = plan_line_breaks(
        pane_view(state, view, fp.pane()),
        place,
        allowed,
        pairs,
        style,
        tab_width,
        new_indent,
    );
    apply_focused_edit_grouped(state, view, fp, move |s| apply_line_break_plan(s, plan));
}

/// Dedent each line of `fp`'s pane that the session `entry` proves open
/// auto-indented, still holds that indent, and starts with a closing token
/// `trigger` completes, as one grouped edit. The single path electric
/// dedent takes: right after a typed character lands, and right before
/// Enter breaks the line, so the new line is indented from the dedented
/// one.
///
/// The candidates are found before anything else, so a keystroke on any
/// other line costs no reparse. A rewritten line is re-armed with its new
/// indent.
pub(super) fn electric_dedent(
    state: &mut EditorState,
    view: &EngineView,
    entry: &InsertEntry,
    fp: FocusedPane,
    trigger: ElectricTrigger,
) {
    let candidates = {
        let sels = pane_view(state, view, fp.pane());
        let armed = autoindent_armed(state.input.entered(entry), &sels);
        electric_candidates(sels, &armed, trigger)
    };
    if candidates.is_empty() || !indent_tree(state, fp.bid(view)) {
        return;
    }
    let buf = doc(state, view, fp.pane());
    let (style, tab_width) = tab_format(buf, &state.settings);
    let word_chars = effective_word_chars(buf, &state.settings);
    let layers = buf.current_layers();
    let plan = plan_electric_dedent(
        buf.text(),
        candidates,
        style,
        tab_width,
        |line, cursor, trigger| {
            layers.and_then(|l| {
                electric_indent(l, buf.text(), line, cursor, trigger, word_chars, tab_width)
            })
        },
    );
    let rewritten = plan.rewritten().to_vec();
    if rewritten.is_empty() {
        return;
    }
    apply_focused_edit_grouped(state, view, fp, move |s| apply_electric_plan(s, plan));
    rearm_autoindent_lines(state, view, entry, fp, &rewritten);
}
