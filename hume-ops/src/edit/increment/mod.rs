//! `increment`/`decrement` and their sequence variants: step the number,
//! date or time at each selection.
//!
//! A cursor's target is the literal it touches, else the first one after it
//! on its line. A wider selection's is the literal touching its first
//! cluster, else the first one starting inside it. Literals are read cluster
//! by cluster, and only a cluster that is one ASCII char can belong to one,
//! so a digit carrying a combining mark is not a digit.
//!
//! A literal is rewritten as parts: a number is one, a date or time one per
//! field. Each selection asks for the part it touches and lands on it.

use std::ops::Range;

use hume_editing::edit::{Edited, Landing, Landings, edit};
use hume_editing::selection::{EditView, SelectionView};
use hume_editing::state::EditState;
use hume_editing::text::{BufferText, TextVersion};
use hume_editing::word::{CharClass, WordChars};
use hume_rope::cluster::ClusterRange;
use hume_rope::grapheme::Cluster;
use hume_rope::line::ContentLine;

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
    pieces: Vec<(ClusterRange, String)>,
    /// Per selection, in order, the index of the piece it lands on.
    landings: Vec<Option<usize>>,
}

/// Plan stepping each selection's target by `step`, `chars` deciding which
/// clusters are word chars. Selections sharing a target step it once, with
/// every part they asked for.
pub fn plan_increment(
    view: EditView<'_>,
    step: i128,
    stepping: Stepping,
    chars: WordChars<'_>,
) -> Result<IncrementPlan, IncrementRefusal> {
    let mut scan = LineScan::new(chars);
    let mut targets: Vec<Target> = Vec::new();
    // The index of the first piece of the last target in `targets`.
    let mut first_piece = 0;
    let mut landings = Vec::with_capacity(view.len());
    // Selections come in order and each targets the first literal ending at
    // or after its start, so the selections sharing a target follow it.
    for sel in view.iter() {
        let Some((target, touched)) = target(sel, &mut scan) else {
            landings.push(None);
            continue;
        };
        let part = target.literal.part_at(touched);
        match targets.last_mut() {
            Some(last) if last.parts[0] == target.parts[0] => last.asked.push(part),
            _ => {
                first_piece += targets.last().map_or(0, |last| last.parts.len());
                targets.push(Target {
                    asked: vec![part],
                    ..target
                });
            }
        }
        landings.push(Some(first_piece + part));
    }
    if targets.is_empty() {
        return Err(IncrementRefusal::NoTarget);
    }
    let mut pieces = Vec::new();
    for (n, target) in targets.into_iter().enumerate() {
        let amount = match stepping {
            Stepping::Uniform => Some(step),
            Stepping::Sequence => {
                step.checked_mul(i128::try_from(n + 1).expect("a selection count fits an i128"))
            }
        };
        let texts = amount
            .and_then(|amount| target.literal.stepped(&target.asked, amount))
            .ok_or(IncrementRefusal::OutOfRange)?;
        pieces.extend(target.parts.into_iter().zip(texts));
    }
    Ok(IncrementPlan {
        version: view.text().version(),
        pieces,
        landings,
    })
}

/// Apply `plan` to `state`. A stepped selection covers the new text of the
/// part it asked for, keeping its facing; any other is kept.
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
            .pieces
            .iter()
            .map(|(range, replacement)| b.replace(*range, replacement))
            .collect();
        let landings = view
            .iter()
            .zip(&plan.landings)
            .map(|(sel, piece)| match piece {
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
    fn of(cluster: &Cluster, chars: WordChars<'_>) -> Self {
        let first = cluster.first();
        let single = cluster.end().offset().chars_since(cluster.start().offset()) == 1;
        Self {
            ascii: (single && first.is_ascii()).then_some(first),
            word: chars.classify(first) == CharClass::Word,
        }
    }
}

/// The ASCII char of `syms[i]`, if any.
fn ascii_at(syms: &[Sym], i: usize) -> Option<char> {
    syms.get(i).and_then(|s| s.ascii)
}

fn is_digit(syms: &[Sym], i: usize) -> bool {
    ascii_at(syms, i).is_some_and(|c| c.is_ascii_digit())
}

#[derive(Clone)]
enum Literal {
    Number(Number),
    Moment(Moment),
}

impl Literal {
    /// Where each part lies, as indices into a literal `len` clusters long.
    fn parts(&self, len: usize) -> Vec<Range<usize>> {
        match self {
            Self::Number(_) => vec![0..len],
            Self::Moment(moment) => moment.parts().collect(),
        }
    }

    /// The part holding index `touched` of the literal, when the literal
    /// holds the selection's first cluster.
    fn part_at(&self, touched: Option<usize>) -> usize {
        match self {
            Self::Number(_) => 0,
            Self::Moment(moment) => moment.part_at(touched),
        }
    }

    /// The text of each part once the literal is moved by `amount` at the
    /// parts `asked` for.
    fn stepped(&self, asked: &[usize], amount: i128) -> Option<Vec<String>> {
        match self {
            Self::Number(number) => number.stepped(amount).map(|text| vec![text]),
            Self::Moment(moment) => moment.stepped(asked, amount),
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
    let (literal, end) = match date::scan(syms, i) {
        Some((moment, end)) => (Literal::Moment(moment), end),
        None if number::is_sign(syms, i)
            && (date::scan(syms, i + 1).is_some() || number::prefixed(syms, i + 1).is_some()) =>
        {
            return None;
        }
        None => {
            let (number, end) = number::scan(syms, i)?;
            (Literal::Number(number), end)
        }
    };
    Some(Found {
        start: i,
        end,
        literal,
    })
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

/// The clusters and literals of the line [`target`] read last. Selections
/// come sorted by start, so the ones sharing a line read it once.
struct LineScan<'w> {
    chars: WordChars<'w>,
    line: Option<ContentLine>,
    clusters: Vec<Cluster>,
    found: Vec<Found>,
}

impl<'w> LineScan<'w> {
    fn new(chars: WordChars<'w>) -> Self {
        Self {
            chars,
            line: None,
            clusters: Vec::new(),
            found: Vec::new(),
        }
    }

    fn read(&mut self, text: &BufferText, line: ContentLine) {
        if self.line == Some(line) {
            return;
        }
        let newline = text.lines().newline(line);
        self.clusters = text
            .clusters()
            .graphemes_at(text.lines().start(line).into())
            .take_while(|c| c.start() < newline)
            .collect();
        let syms: Vec<Sym> = self
            .clusters
            .iter()
            .map(|cluster| Sym::of(cluster, self.chars))
            .collect();
        self.found = literals(&syms).collect();
        self.line = Some(line);
    }
}

struct Target {
    /// The ranges of the literal's parts.
    parts: Vec<ClusterRange>,
    literal: Literal,
    /// The parts the selections on this literal asked for.
    asked: Vec<usize>,
}

/// The selection's target, and the index within the literal of the
/// selection's first cluster when the literal holds it.
fn target(sel: SelectionView<'_>, scan: &mut LineScan<'_>) -> Option<(Target, Option<usize>)> {
    let text = sel.text();
    let start = sel.start().offset();
    let lines = sel.lines();
    let limit = if sel.is_cursor() {
        text.lines().newline(lines.start).offset()
    } else {
        sel.covered().end().offset()
    };
    for line in lines.iter() {
        scan.read(text, line);
        let clusters = &scan.clusters;
        let reaching = scan
            .found
            .partition_point(|found| clusters[found.end - 1].start().offset() < start);
        if let Some(found) = scan.found.get(reaching) {
            let first_start = clusters[found.start].start().offset();
            if first_start >= limit {
                return None;
            }
            let touched = (first_start <= start).then(|| start.chars_since(first_start));
            let parts = found
                .literal
                .parts(found.end - found.start)
                .into_iter()
                .map(|part| {
                    let first = &clusters[found.start + part.start];
                    let last = &clusters[found.start + part.end - 1];
                    first.range().hull(last.range())
                })
                .collect();
            let target = Target {
                parts,
                literal: found.literal.clone(),
                asked: Vec::new(),
            };
            return Some((target, touched));
        }
    }
    None
}
