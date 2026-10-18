//! Comment toggling over the lines each selection covers.
//!
//! Selections sharing a line form one unit. A unit is commented when its
//! trimmed text is wrapped in one block comment pair, or when every line
//! that is not blank starts with a line token after its indent. The primary
//! selection's unit decides for all, or the first unit that is not blank when
//! the primary's lines are all blank: when it is commented, every unit is
//! uncommented, whichever kind it holds; otherwise every unit is commented
//! in its preferred kind.

use hume_editing::comment::{BlockCommentPair, CommentTokens};
use hume_editing::edit::{Edited, Landing, Landings, edit};
use hume_editing::selection::EditView;
use hume_editing::state::EditState;
use hume_editing::text::{BufferText, TextVersion};
use hume_rope::cluster::{ClusterBound, ClusterRange, ClusterStart};
use hume_rope::line::ContentLine;
use hume_rope::offset::InclusiveRange;

use crate::word_unit::{next_nonblank, prev_nonblank};

/// Which kind of comment a unit spanning several lines takes when its
/// language has both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultiLineComments {
    Block,
    Line,
}

/// Why [`plan_comment_toggle`] made no plan.
#[derive(Debug, PartialEq, Eq)]
pub enum CommentPlanError<E> {
    /// The resolver's error.
    Tokens(E),
    /// A unit's text holds the end token of the block comment it would
    /// take, which would close the comment early, and its language has no
    /// line comment.
    BlockEndInside(String),
}

/// The edits [`plan_comment_toggle`] computed, for the text it read.
#[derive(Debug)]
pub struct CommentPlan {
    version: TextVersion,
    ops: Vec<CommentOp>,
}

#[derive(Debug)]
enum CommentOp {
    Insert(ClusterBound, String),
    Delete(ClusterRange),
}

/// What a unit holds now.
enum Commented {
    /// The deletions that unwrap its block comment.
    Block(Vec<ClusterRange>),
    /// Every line that is not blank starts with a line token.
    Line,
    No,
}

/// Plan the toggle for `view`'s selections. `tokens_at` gives the tokens of
/// the language at a unit's trimmed text (its first line for a unit with
/// only blank lines).
pub fn plan_comment_toggle<'t, E>(
    view: EditView<'_>,
    multi_line: MultiLineComments,
    tab_width: u8,
    mut tokens_at: impl FnMut(ClusterRange) -> Result<CommentTokens<'t>, E>,
) -> Result<CommentPlan, CommentPlanError<E>> {
    let text = view.text();
    let primary_line = view.primary().lines().start;
    let mut units = Vec::new();
    for lines in merged_lines(view) {
        let span = trimmed_span(text, lines);
        let tokens = tokens_at(span.unwrap_or_else(|| text.lines().range(lines.start)))
            .map_err(CommentPlanError::Tokens)?;
        units.push(Unit {
            lines,
            span,
            tokens,
            commented: commented(text, lines, span, tokens),
        });
    }

    let primary = units
        .iter()
        .find(|unit| unit.lines.contains(primary_line))
        .expect("the primary selection's lines are in a unit");
    let has_text = |unit: &&Unit<'_>| unit.span.is_some();
    let decider = Some(primary)
        .filter(has_text)
        .or_else(|| units.iter().find(has_text));
    let uncomment = decider.is_some_and(|unit| !matches!(unit.commented, Commented::No));

    let mut ops = Vec::new();
    for unit in &units {
        if uncomment {
            unit.uncomment(text, &mut ops);
        } else {
            unit.comment(text, multi_line, tab_width, &mut ops)
                .map_err(CommentPlanError::BlockEndInside)?;
        }
    }
    Ok(CommentPlan {
        version: text.version(),
        ops,
    })
}

/// Apply `plan` to `state`. A linewise selection keeps its lines; any other
/// keeps its own clusters, so a token at its edge falls outside it.
///
/// # Panics
/// Panics if `plan` was computed for another text.
pub fn apply_comment_plan(state: EditState, plan: CommentPlan) -> Edited {
    assert!(
        state.text().version() == plan.version,
        "apply_comment_plan: the plan was computed for another text"
    );
    if plan.ops.is_empty() {
        return Edited::unchanged(state);
    }
    let view = state.view();
    let primary = view.primary().index();
    edit(&state, |b| {
        for op in &plan.ops {
            match op {
                CommentOp::Insert(at, token) => {
                    b.insert(*at, token);
                }
                CommentOp::Delete(range) => {
                    b.delete(*range);
                }
            }
        }
        let landings = view
            .iter()
            .map(|sel| {
                if sel.is_linewise() {
                    let lines = sel.lines();
                    Landing::at_lines(sel, lines.start, lines.end)
                } else {
                    Landing::kept(sel.selection())
                }
            })
            .collect();
        Landings::new(landings, primary)
    })
}

struct Unit<'t> {
    lines: InclusiveRange<ContentLine>,
    /// From the first cluster that is not blank to the last; `None` when
    /// every line is blank.
    span: Option<ClusterRange>,
    tokens: CommentTokens<'t>,
    commented: Commented,
}

impl Unit<'_> {
    fn uncomment(&self, text: &BufferText, ops: &mut Vec<CommentOp>) {
        match &self.commented {
            Commented::Block(deletions) => {
                ops.extend(deletions.iter().copied().map(CommentOp::Delete));
            }
            Commented::Line | Commented::No => {
                ops.extend(
                    self.lines
                        .iter()
                        .filter_map(|line| line_token_deletion(text, line, self.tokens))
                        .map(CommentOp::Delete),
                );
            }
        }
    }

    /// A unit whose text holds the end token of the block comment it would
    /// take gets line comments instead; `Err` carries that end token when
    /// its language has none.
    fn comment(
        &self,
        text: &BufferText,
        multi_line: MultiLineComments,
        tab_width: u8,
        ops: &mut Vec<CommentOp>,
    ) -> Result<(), String> {
        let Some(span) = self.span else {
            return Ok(());
        };
        let single = self.lines.start == self.lines.end;
        let prefer_block = !single && multi_line == MultiLineComments::Block;
        let fits = |pair: &BlockCommentPair| {
            !occurs_between(text, span.start().into(), span.end(), pair.end())
        };
        match self.tokens {
            CommentTokens::Block(block) => {
                let pair = block.primary();
                if !fits(pair) {
                    return Err(pair.end().to_owned());
                }
                block_comment(span, pair, ops);
            }
            CommentTokens::Both(_, block) if prefer_block && fits(block.primary()) => {
                block_comment(span, block.primary(), ops);
            }
            CommentTokens::Line(line) | CommentTokens::Both(line, _) => {
                let token = format!("{} ", line.primary());
                let content: Vec<ContentLine> = self
                    .lines
                    .iter()
                    .filter(|&l| !text.lines().is_blank(l))
                    .collect();
                // The least indent can fall inside another line's tab, so it
                // is lowered to a column every line has a cluster boundary at.
                let reach = |column| {
                    content
                        .iter()
                        .map(|&l| {
                            let at = text.columns().pos_at_display_col(l, column, tab_width);
                            text.columns().display_col(l, at.offset(), tab_width)
                        })
                        .min()
                        .expect("a unit with a span has a line that is not blank")
                };
                let mut column = content
                    .iter()
                    .map(|&l| text.columns().indent(l, tab_width).1)
                    .min()
                    .expect("a unit with a span has a line that is not blank");
                loop {
                    let lowered = reach(column);
                    if lowered == column {
                        break;
                    }
                    column = lowered;
                }
                ops.extend(content.into_iter().map(|l| {
                    let at = text.columns().pos_at_display_col(l, column, tab_width);
                    CommentOp::Insert(at.into(), token.clone())
                }));
            }
        }
        Ok(())
    }
}

fn block_comment(span: ClusterRange, pair: &BlockCommentPair, ops: &mut Vec<CommentOp>) {
    ops.push(CommentOp::Insert(
        span.start().into(),
        format!("{} ", pair.start()),
    ));
    ops.push(CommentOp::Insert(span.end(), format!(" {}", pair.end())));
}

/// The selections' line ranges, merged where two share a line.
fn merged_lines(view: EditView<'_>) -> Vec<InclusiveRange<ContentLine>> {
    let mut merged: Vec<InclusiveRange<ContentLine>> = Vec::new();
    for lines in view.iter().map(|sel| sel.lines()) {
        match merged.last_mut() {
            Some(last) if lines.start <= last.end => last.end = last.end.max(lines.end),
            _ => merged.push(lines),
        }
    }
    merged
}

fn trimmed_span(text: &BufferText, lines: InclusiveRange<ContentLine>) -> Option<ClusterRange> {
    let from = text.lines().start(lines.start).into();
    let to = text.lines().newline(lines.end).into();
    let start = next_nonblank(text, from, to)?;
    let last = prev_nonblank(text, to, from)?;
    ClusterRange::through(text.full_slice(), start, last)
}

fn commented(
    text: &BufferText,
    lines: InclusiveRange<ContentLine>,
    span: Option<ClusterRange>,
    tokens: CommentTokens<'_>,
) -> Commented {
    let Some(span) = span else {
        return Commented::No;
    };
    if let Some(deletions) = tokens
        .block()
        .and_then(|block| block_deletions(text, span, block.iter()))
    {
        return Commented::Block(deletions);
    }
    let line_commented = lines
        .iter()
        .filter(|&l| !text.lines().is_blank(l))
        .all(|l| line_token_deletion(text, l, tokens).is_some());
    if line_commented {
        Commented::Line
    } else {
        Commented::No
    }
}

/// The deletions that unwrap `span` when it starts with a pair's start token
/// and ends with its end token, with no copy of that end token between
/// them. Of the pairs that match, the longest start wins, then the longest
/// end. Each deletion takes one U+0020 inside its token along.
fn block_deletions<'p>(
    text: &BufferText,
    span: ClusterRange,
    pairs: impl Iterator<Item = &'p BlockCommentPair>,
) -> Option<Vec<ClusterRange>> {
    let (open_end, close_start) = pairs
        .filter_map(|pair| {
            let open_end = match_forward(text, span.start(), pair.start())?;
            let close_start = match_backward(text, span.end(), pair.end())?;
            let inner_ok = open_end.offset() <= close_start.offset()
                && !occurs_between(text, open_end, close_start.into(), pair.end());
            inner_ok.then_some((open_end, close_start))
        })
        .max_by_key(|(open_end, close_start)| {
            (open_end.offset(), std::cmp::Reverse(close_start.offset()))
        })?;

    let slice = text.full_slice();
    let open_end = single_space_after(text, open_end, close_start.into()).unwrap_or(open_end);
    let close_start = single_space_before(text, close_start, open_end).unwrap_or(close_start);
    Some(
        [
            ClusterRange::between(slice, span.start(), open_end),
            ClusterRange::between(slice, close_start, span.end()),
        ]
        .into_iter()
        .flatten()
        .collect(),
    )
}

/// The deletion that uncomments `line`: from its indent's end through the
/// longest line token there, and one U+0020 after it.
fn line_token_deletion(
    text: &BufferText,
    line: ContentLine,
    tokens: CommentTokens<'_>,
) -> Option<ClusterRange> {
    let start = text.lines().indent_end(line);
    let token_end = tokens
        .line()?
        .iter()
        .filter_map(|token| match_forward(text, start, token))
        .max_by_key(|end| end.offset())?;
    let line_end = text.lines().newline(line).into();
    let end = single_space_after(text, token_end, line_end).unwrap_or(token_end);
    ClusterRange::between(text.full_slice(), start, end)
}

/// The end of the cluster at `at` when it is one U+0020 ending by `limit`.
fn single_space_after(
    text: &BufferText,
    at: ClusterBound,
    limit: ClusterBound,
) -> Option<ClusterBound> {
    let cluster = text.clusters().graphemes_at(at).next()?;
    (cluster.as_char() == Some(' ') && cluster.end().offset() <= limit.offset())
        .then(|| cluster.end())
}

/// The start of the cluster before `at` when it is one U+0020 starting at or
/// after `limit`.
fn single_space_before(
    text: &BufferText,
    at: ClusterStart,
    limit: ClusterBound,
) -> Option<ClusterStart> {
    let cluster = text.clusters().before(at.into()).next()?;
    (cluster.as_char() == Some(' ') && cluster.start().offset() >= limit.offset())
        .then(|| cluster.start())
}

/// Whether `token` starts at a cluster in `[from, to)` and ends by `to`.
fn occurs_between(text: &BufferText, from: ClusterBound, to: ClusterBound, token: &str) -> bool {
    text.clusters()
        .graphemes_at(from)
        .take_while(|cluster| cluster.start().offset() < to.offset())
        .filter(|cluster| token.starts_with(cluster.first()))
        .any(|cluster| {
            match_forward(text, cluster.start(), token)
                .is_some_and(|end| end.offset() <= to.offset())
        })
}

/// The bound after `token` when the text at `at` spells it and it ends on a
/// cluster boundary.
fn match_forward(
    text: &BufferText,
    at: impl Into<ClusterBound>,
    token: &str,
) -> Option<ClusterBound> {
    let at = at.into();
    let mut chars = text.chars_at(at.offset());
    if !token
        .chars()
        .all(|t| chars.next().is_some_and(|(_, c)| c == t))
    {
        return None;
    }
    let len = token.chars().count();
    text.clusters()
        .graphemes_at(at)
        .map(|cluster| cluster.end())
        .find(|end| end.offset().chars_since(at.offset()) >= len)
        .filter(|end| end.offset().chars_since(at.offset()) == len)
}

/// The cluster start where `token` begins when the text ending at `end`
/// spells it and it starts on a cluster boundary.
fn match_backward(text: &BufferText, end: ClusterBound, token: &str) -> Option<ClusterStart> {
    let mut chars = text.chars_at(end.offset());
    if !token
        .chars()
        .rev()
        .all(|t| chars.prev().is_some_and(|(_, c)| c == t))
    {
        return None;
    }
    let len = token.chars().count();
    text.clusters()
        .before(end)
        .map(|cluster| cluster.start())
        .find(|start| end.offset().chars_since(start.offset()) >= len)
        .filter(|start| end.offset().chars_since(start.offset()) == len)
}
