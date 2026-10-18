//! `increment`/`decrement` and their sequence variants: step the number,
//! date or time at each selection.
//!
//! A cursor's target is the literal it touches, else the first one after it
//! on its line. A wider selection's is the literal touching its first
//! cluster, else the first one starting inside it. Literals are read cluster
//! by cluster, and only a cluster that is one ASCII char can belong to one,
//! so a digit carrying a combining mark is not a digit.

use hume_editing::edit::{Edited, Landing, Landings, edit};
use hume_editing::selection::{EditView, SelectionView};
use hume_editing::state::EditState;
use hume_editing::text::TextVersion;
use hume_rope::cluster::ClusterRange;
use hume_rope::grapheme::Cluster;
use hume_rope::offset::CharOffset;
use rustc_hash::FxHashMap;

mod date;
mod number;

use date::Moment;
use number::Number;

/// How [`plan_increment`] spreads its step over the targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stepping {
    /// Every target moves by the step.
    Uniform,
    /// The n-th target, in selection order, moves by n times the step.
    Sequence,
}

/// Why [`plan_increment`] made no plan.
#[derive(Debug, PartialEq, Eq)]
pub enum IncrementRefusal {
    /// No selection has a number, date or time to step.
    NoTarget,
    /// A stepped value does not fit its literal: past the integer range, a
    /// year outside 0000 to 9999.
    OutOfRange,
}

/// The replacements [`plan_increment`] computed, for the text it read.
#[derive(Debug)]
pub struct IncrementPlan {
    version: TextVersion,
    changes: Vec<(ClusterRange, String)>,
    /// Per selection, in order, the index of the change it lands on.
    landings: Vec<Option<usize>>,
}

/// Plan stepping each selection's target by `step`. Selections sharing a
/// target step it once.
pub fn plan_increment(
    view: EditView<'_>,
    step: i128,
    stepping: Stepping,
) -> Result<IncrementPlan, IncrementRefusal> {
    let mut changes: Vec<(ClusterRange, String)> = Vec::new();
    let mut by_start = FxHashMap::<CharOffset, usize>::default();
    let mut landings = Vec::with_capacity(view.len());
    for sel in view.iter() {
        let Some(target) = target(sel) else {
            landings.push(None);
            continue;
        };
        let start = target.range.start().offset();
        if let Some(&index) = by_start.get(&start) {
            landings.push(Some(index));
            continue;
        }
        let index = changes.len();
        let amount = match stepping {
            Stepping::Uniform => Some(step),
            Stepping::Sequence => i128::try_from(index + 1)
                .ok()
                .and_then(|n| step.checked_mul(n)),
        };
        let replacement = amount
            .and_then(|amount| target.literal.stepped(target.touched, amount))
            .ok_or(IncrementRefusal::OutOfRange)?;
        by_start.insert(start, index);
        landings.push(Some(index));
        changes.push((target.range, replacement));
    }
    if changes.is_empty() {
        return Err(IncrementRefusal::NoTarget);
    }
    Ok(IncrementPlan {
        version: view.text().version(),
        changes,
        landings,
    })
}

/// Apply `plan` to `state`. A stepped selection covers its new literal,
/// keeping its facing; any other is kept.
///
/// # Panics
/// Panics if `plan` was computed for another text.
pub fn apply_increment_plan(state: EditState, plan: IncrementPlan) -> Edited {
    assert!(
        state.text().version() == plan.version,
        "apply_increment_plan: the plan was computed for another text"
    );
    let view = state.view();
    let primary = view.primary().index();
    edit(&state, |b| {
        let marks: Vec<_> = plan
            .changes
            .iter()
            .map(|(range, replacement)| b.replace(*range, replacement))
            .collect();
        let landings = view
            .iter()
            .zip(&plan.landings)
            .map(|(sel, change)| match change {
                Some(index) => Landing::covering(marks[*index], sel.facing()),
                None => Landing::kept(sel.selection()),
            })
            .collect();
        Landings::new(landings, primary)
    })
}

/// One cluster of a line, as the literal scanners read it.
#[derive(Clone, Copy)]
struct Sym {
    /// The cluster's char when it is one ASCII char.
    ascii: Option<char>,
    /// Whether the cluster is a letter, digit or `_`.
    word: bool,
}

impl Sym {
    fn of(cluster: &Cluster) -> Self {
        let first = cluster.first();
        let single = cluster.end().offset().chars_since(cluster.start().offset()) == 1;
        Self {
            ascii: (single && first.is_ascii()).then_some(first),
            word: first.is_alphanumeric() || first == '_',
        }
    }
}

/// The ASCII char of `syms[i]`, if any.
fn ascii_at(syms: &[Sym], i: usize) -> Option<char> {
    syms.get(i).and_then(|s| s.ascii)
}

enum Literal {
    Number(Number),
    Moment(Moment),
}

impl Literal {
    /// The literal moved by `amount`. `touched` is the index within the
    /// literal of the selection's first cluster, when the literal holds it.
    fn stepped(&self, touched: Option<usize>, amount: i128) -> Option<String> {
        match self {
            Self::Number(number) => number.stepped(amount),
            Self::Moment(moment) => moment.stepped(moment.field_at(touched), amount),
        }
    }
}

/// A literal over `syms[start..end]`.
struct Found {
    start: usize,
    end: usize,
    literal: Literal,
}

/// The literal starting at `syms[i]`. Dates and times win over the numbers
/// they are made of, and a minus before a date or a prefixed number is not
/// its sign.
fn literal_at(syms: &[Sym], i: usize) -> Option<Found> {
    let found = |(literal, end)| Found {
        start: i,
        end,
        literal,
    };
    if let Some((moment, end)) = date::scan(syms, i) {
        return Some(found((Literal::Moment(moment), end)));
    }
    if number::is_sign(syms, i)
        && (date::scan(syms, i + 1).is_some() || number::prefixed(syms, i + 1).is_some())
    {
        return None;
    }
    number::scan(syms, i).map(|(number, end)| found((Literal::Number(number), end)))
}

/// The literals of `syms`, left to right.
fn literals(syms: &[Sym]) -> impl Iterator<Item = Found> + '_ {
    let mut i = 0;
    std::iter::from_fn(move || {
        while i < syms.len() {
            if let Some(found) = literal_at(syms, i) {
                i = found.end;
                return Some(found);
            }
            i += 1;
        }
        None
    })
}

struct Target {
    range: ClusterRange,
    literal: Literal,
    touched: Option<usize>,
}

fn target(sel: SelectionView<'_>) -> Option<Target> {
    let text = sel.text();
    let start = sel.start().offset();
    let lines = sel.lines();
    let limit = if sel.is_cursor() {
        text.lines().newline(lines.start).offset()
    } else {
        sel.covered().end().offset()
    };
    for line in lines.iter() {
        let newline = text.lines().newline(line);
        let clusters: Vec<Cluster> = text
            .clusters()
            .graphemes_at(text.lines().start(line).into())
            .take_while(|c| c.start() < newline)
            .collect();
        let syms: Vec<Sym> = clusters.iter().map(Sym::of).collect();
        for found in literals(&syms) {
            let first = &clusters[found.start];
            let last = &clusters[found.end - 1];
            if last.start().offset() < start {
                continue;
            }
            let first_start = first.start().offset();
            if first_start >= limit {
                return None;
            }
            let touched = (first_start <= start).then(|| start.chars_since(first_start));
            return Some(Target {
                range: first.range().hull(last.range()),
                literal: found.literal,
                touched,
            });
        }
    }
    None
}
