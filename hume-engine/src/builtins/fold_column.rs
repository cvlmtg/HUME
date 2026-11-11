//! The gutter column of fold carets: one beside each line a fold starts on.

use hume_rope::line::ContentLine;
use hume_rope::offset::InclusiveRange;

use crate::folds::HiddenLines;
use crate::providers::{
    FoldMark, GutterCell, GutterCellContent, GutterColumn, GutterCtx, GutterInputs, GutterRow,
    SyntaxSource,
};
use crate::types::{DisplayLineKind, ScopeId};

/// The caret beside a fold that is open.
const OPEN: &str = "▾";
/// The caret beside a fold that is closed.
const CLOSED: &str = "▸";

/// Built-in gutter column drawing each line's [`FoldMark`], while
/// [`GutterInputs::folds`] shows it: the caret and one padding cell.
pub struct FoldColumn {
    /// Interned `"ui.linenr"`: every line but the primary head line.
    default_scope: ScopeId,
    /// Interned `"ui.linenr.selected"`: the primary selection's head line.
    selected_scope: ScopeId,
}

impl FoldColumn {
    /// The column's width while shown.
    pub const WIDTH: u8 = 2;

    /// `default_scope`/`selected_scope` are interned by the caller, as for
    /// [`LineNumberColumn::new`](crate::builtins::line_number::LineNumberColumn::new).
    pub fn new(default_scope: ScopeId, selected_scope: ScopeId) -> Self {
        Self {
            default_scope,
            selected_scope,
        }
    }
}

impl GutterColumn for FoldColumn {
    fn width(&self, _last_line_idx: hume_rope::line::RopeyLine, inputs: GutterInputs) -> u8 {
        if inputs.folds { Self::WIDTH } else { 0 }
    }

    fn render_cells(&self, kind: DisplayLineKind, ctx: &GutterCtx) -> Vec<GutterCell> {
        let DisplayLineKind::LineStart { line_idx } = kind else {
            return vec![GutterCell::blank(self.default_scope)];
        };
        let is_head_line = matches!(
            ctx.row,
            GutterRow::Body { primary_head_line } if line_idx == primary_head_line.into()
        );
        let scope = if is_head_line {
            self.selected_scope
        } else {
            self.default_scope
        };
        let content = match ctx.fold {
            FoldMark::Open => GutterCellContent::Text(OPEN.into()),
            FoldMark::Closed => GutterCellContent::Text(CLOSED.into()),
            FoldMark::None => GutterCellContent::Blank,
        };
        vec![GutterCell { content, scope }]
    }
}

/// The fold marks of the lines a pane draws: its closed folds, and the
/// lines foldable regions start on among those read.
#[derive(Default)]
pub struct FoldStarts {
    hidden: HiddenLines,
    starts: Vec<ContentLine>,
}

impl FoldStarts {
    /// Start over for a pane whose closed folds hide `hidden`.
    pub fn reset(&mut self, hidden: HiddenLines) {
        self.hidden = hidden;
        self.starts.clear();
    }

    /// Add the region starts among the shown lines of `lines`, one `syntax`
    /// query per run of shown lines.
    pub fn read(
        &mut self,
        lines: InclusiveRange<ContentLine>,
        syntax: &dyn SyntaxSource,
        rope: &ropey::Rope,
    ) {
        for run in self.hidden.shown_runs(lines) {
            syntax.fold_starts(run, rope, &mut self.starts);
        }
        self.starts.sort_unstable();
        self.starts.dedup();
    }

    /// The mark of `line`, a line read: closed when a closed fold starts
    /// there, open when a region starts there, none otherwise.
    pub fn mark(&self, line: ContentLine) -> FoldMark {
        if self.hidden.closed_at(line) {
            FoldMark::Closed
        } else if self.starts.binary_search(&line).is_ok() {
            FoldMark::Open
        } else {
            FoldMark::None
        }
    }
}

#[cfg(test)]
mod tests;
