//! Electric dedent: a line an auto-indent opened is pulled back once the
//! token typed on it closes a scope.

use hume_editing::edit::{Edited, Landing, Landings, edit};
use hume_editing::indent::ElectricTrigger;
use hume_editing::selection::EditView;
use hume_editing::state::EditState;
use hume_editing::tab_style::TabStyle;
use hume_editing::text::{BufferText, TextVersion};
use hume_rope::cluster::{ClusterRange, ClusterStart};
use hume_rope::column::BufferLineCol;
use hume_rope::line::ContentLine;
use hume_rope::offset::{CharOffset, ExclusiveRange};

use super::insert::line_indent_range;

/// One selection's auto-indent record: the whitespace range an insert
/// session inserted, and the text it inserted there.
#[derive(Clone, Copy, Debug)]
pub struct ArmedIndent<'a> {
    pub range: ExclusiveRange<CharOffset>,
    pub text: &'a str,
}

/// The selections whose line [`plan_electric_dedent`] may rewrite, for the
/// text [`electric_candidates`] read.
#[derive(Debug)]
pub struct ElectricCandidates {
    version: TextVersion,
    trigger: ElectricTrigger,
    /// Each candidate's selection index, the line its trigger lands on, and
    /// where it lands.
    lines: Vec<(usize, ContentLine, ClusterStart)>,
}

impl ElectricCandidates {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// The selections of `view` whose line `trigger` lands on is still as its
/// auto-indent left it, with content typed after it: `armed` (by sorted
/// selection index) has a record starting at that line's start whose text
/// equals the line's leading whitespace, and the line is not blank.
/// Whitespace changed by hand, a Backspace included, disqualifies the line.
///
/// A typed character lands at the head. A line break lands at the
/// selection's start, where it replaces what the selection covers.
pub fn electric_candidates(
    view: EditView<'_>,
    armed: &[ArmedIndent<'_>],
    trigger: ElectricTrigger,
) -> ElectricCandidates {
    let text = view.text();
    let lines = view
        .iter()
        .filter_map(|sel| {
            let armed = armed.get(sel.index())?;
            let at = match trigger {
                ElectricTrigger::Typed => sel.head(),
                ElectricTrigger::LineBreak => sel.start(),
            };
            let line = text.char_to_line(at.offset());
            let indent = line_indent_range(text, at.offset());
            let qualifies = armed.range.start == indent.chars().start
                && !indent.is_blank()
                && text.slice(indent.chars()) == armed.text;
            qualifies.then_some((sel.index(), line, at))
        })
        .collect();
    ElectricCandidates {
        version: text.version(),
        trigger,
        lines,
    }
}

/// The indent rewrites [`plan_electric_dedent`] computed, for the text it
/// read.
#[derive(Debug)]
pub struct ElectricPlan {
    version: TextVersion,
    rewrites: Vec<Rewrite>,
    rewritten: Vec<(usize, ContentLine)>,
}

impl ElectricPlan {
    /// The candidate selections whose line the plan rewrites, in order: each
    /// one's index and the line its trigger landed on.
    pub fn rewritten(&self) -> &[(usize, ContentLine)] {
        &self.rewritten
    }
}

/// One line's new indent: `inserted` goes in at `removed`'s start, then
/// `removed` is deleted. `removed` is the old indent past the clusters it
/// shares with the new one.
#[derive(Debug)]
struct Rewrite {
    inserted: String,
    removed: ClusterRange,
}

/// Plan the dedent of each candidate's line to the width `width_of` gives
/// it, asked once per line with where the trigger lands for its first
/// candidate, and the trigger. A line is rewritten only when that width is
/// smaller than its current indent, so typing on after a dedent changes
/// nothing. The new indent is rendered in `style`.
///
/// # Panics
/// Panics if `candidates` were read from another text.
pub fn plan_electric_dedent(
    text: &BufferText,
    candidates: ElectricCandidates,
    style: TabStyle,
    tab_width: u8,
    mut width_of: impl FnMut(ContentLine, ClusterStart, ElectricTrigger) -> Option<BufferLineCol>,
) -> ElectricPlan {
    assert!(
        text.version() == candidates.version,
        "plan_electric_dedent: the candidates were computed for another text"
    );
    let mut rewrites = Vec::new();
    let mut rewritten = Vec::new();
    // Candidates come in selection order, so one line's are adjacent.
    let mut previous: Option<(ContentLine, bool)> = None;
    for (index, line, at) in candidates.lines {
        let changed = match previous {
            Some((previous_line, changed)) if previous_line == line => changed,
            _ => {
                let rewrite = width_of(line, at, candidates.trigger)
                    .and_then(|width| rewrite(text, line, width, style, tab_width));
                let changed = rewrite.is_some();
                rewrites.extend(rewrite);
                previous = Some((line, changed));
                changed
            }
        };
        if changed {
            rewritten.push((index, line));
        }
    }
    ElectricPlan {
        version: text.version(),
        rewrites,
        rewritten,
    }
}

/// `line`'s indent rewritten to `width`, when that is smaller than its
/// current width. The old and new indents are compared cluster by cluster
/// from the line start, and only what follows their common prefix is
/// replaced.
fn rewrite(
    text: &BufferText,
    line: ContentLine,
    width: BufferLineCol,
    style: TabStyle,
    tab_width: u8,
) -> Option<Rewrite> {
    let (ws_end, current) = text.columns().indent(line, tab_width);
    if width >= current {
        return None;
    }
    let rendered = style.render_indent(width, tab_width);
    let start = text.lines().start(line);
    let mut new_indent = rendered.chars();
    let removed_start = text
        .clusters()
        .graphemes_at(start.into())
        .take_while(|cluster| cluster.start() < ws_end)
        .find(|cluster| {
            let single = cluster.end().offset().chars_since(cluster.start().offset()) == 1;
            new_indent
                .next()
                .is_none_or(|ch| !(single && cluster.first() == ch))
        })
        .map(|cluster| cluster.start())
        .expect("a narrower indent shares less than the whole old indent");
    let kept = removed_start.offset().chars_since(start.offset());
    let inserted = rendered.chars().skip(kept).collect();
    let removed = ClusterRange::between(text.full_slice(), removed_start, ws_end.into())
        .expect("the removed part of the old indent ends past its start");
    Some(Rewrite { inserted, removed })
}

/// Apply `plan` to `state`. Each rewrite inserts its new indent before
/// deleting the old, so a position at the old indent's end maps to the new
/// indent's end. Every selection is carried through the edit.
///
/// # Panics
/// Panics if `plan` was computed for another text.
pub fn apply_electric_plan(state: EditState, plan: ElectricPlan) -> Edited {
    assert!(
        state.text().version() == plan.version,
        "apply_electric_plan: the plan was computed for another text"
    );
    if plan.rewrites.is_empty() {
        return Edited::unchanged(state);
    }
    let view = state.view();
    let primary = view.primary().index();
    edit(&state, |b| {
        for rewrite in &plan.rewrites {
            if !rewrite.inserted.is_empty() {
                b.insert(rewrite.removed.start(), &rewrite.inserted);
            }
            b.delete(rewrite.removed);
        }
        let landings = view
            .iter()
            .map(|sel| Landing::kept(sel.selection()))
            .collect();
        Landings::new(landings, primary)
    })
}
