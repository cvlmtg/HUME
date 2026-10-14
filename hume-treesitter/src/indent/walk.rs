use std::borrow::Cow;
use std::ops::Range;

use hume_editing::indent::ElectricTrigger;
use hume_editing::text::BufferText;
use hume_editing::word::{CharClass, WordChars};
use hume_rope::column::BufferLineCol;
use hume_rope::line::RopeyLine;
use hume_rope::width::indent_levels;
use rustc_hash::FxHashMap;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, QueryCursor, QueryMatch};

use super::query::{IndentCapture, IndentQuery, Predicate};
use crate::highlight::RopeProvider;
use crate::layers::SyntaxLayer;

/// The most matches one query run holds in progress. The indents queries
/// are written and tested under this limit, so a pattern too costly to
/// finish drops the matches its authors saw dropped.
const MATCH_LIMIT: u32 = 256;

/// A line's indent: `net` levels from `align` when a scope aligns the
/// line, otherwise from the layer's base.
pub(super) struct Levels {
    pub(super) net: i32,
    pub(super) align: Option<BufferLineCol>,
}

/// The line an indent is computed for.
#[derive(Clone, Copy)]
pub(super) enum Target {
    /// The line a break inserted at this byte starts.
    NewLine(usize),
    /// The existing line whose first non-blank byte this is.
    Line(usize),
}

impl Target {
    pub(super) fn byte(self) -> usize {
        match self {
            Self::NewLine(byte) | Self::Line(byte) => byte,
        }
    }
}

/// What the model says about a target.
pub(super) enum Outcome {
    Levels(Levels),
    /// The target is inside an opaque node begun on an earlier line: its
    /// indent is the text's own.
    Opaque,
}

impl Outcome {
    pub(super) fn levels(&self) -> Option<&Levels> {
        match self {
            Self::Levels(levels) => Some(levels),
            Self::Opaque => None,
        }
    }
}

/// A line of the text as it reads once the planned break is inserted.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct PlannedLine(RopeyLine);

/// The lines of the tree's bytes as they read for a target: once a line
/// break is planned at `planned_break`, text from it on moves down one
/// line. `at` is the target's byte.
struct PlannedLines<'t> {
    text: &'t BufferText,
    at: usize,
    planned_break: Option<usize>,
}

impl PlannedLines<'_> {
    /// The line holding `byte` before the break.
    fn source_line(&self, byte: usize) -> RopeyLine {
        self.text.ropey_char_to_line(self.text.byte_to_char(byte))
    }

    fn start(&self, node: Node) -> PlannedLine {
        let byte = node.start_byte();
        let moved = self.planned_break.is_some_and(|split| byte >= split);
        PlannedLine(self.source_line(byte).advance(usize::from(moved)))
    }

    /// The line of the node's exclusive end. An end at the planned break
    /// stays on the line before it.
    fn end(&self, node: Node) -> PlannedLine {
        let byte = node.end_byte();
        let moved = self.planned_break.is_some_and(|split| byte > split);
        PlannedLine(self.source_line(byte).advance(usize::from(moved)))
    }

    /// The target line: the line a planned break starts, or the existing
    /// line holding `at`.
    fn target(&self) -> PlannedLine {
        let moved = self.planned_break.is_some();
        PlannedLine(self.source_line(self.at).advance(usize::from(moved)))
    }

    /// The indent level of the line holding `byte` before any break: the
    /// whole indent levels in the display width of its leading whitespace.
    fn level(&self, byte: usize, tab_width: u8) -> u32 {
        let at = self.text.byte_to_char(byte);
        let (_, width) = self
            .text
            .columns()
            .indent(self.text.char_to_line(at), tab_width);
        indent_levels(width.get(), tab_width)
    }
}

/// The capture flags one node collected across every match.
#[derive(Default)]
struct Flags {
    indent: bool,
    indent_always: bool,
    outdent: bool,
    outdent_always: bool,
    extend: bool,
    prevent_once: bool,
    /// Set by an `@indent`/`@indent.always` capture from a header-scoped
    /// pattern: the scope opens on the parent's first line.
    header_scope: bool,
    /// Set by an `@indent`, `@indent.always`, `@outdent`, `@outdent.always`
    /// or anchored `@align` capture from a header-scoped pattern: a new line
    /// before this node descends into it.
    header_any: bool,
}

#[derive(Default)]
struct Captured {
    flags: FxHashMap<usize, Flags>,
    /// Each aligning node's anchor column.
    aligns: FxHashMap<usize, BufferLineCol>,
    opaque: Vec<Range<usize>>,
}

impl Captured {
    fn flags_of(&mut self, node: Node) -> &mut Flags {
        self.flags.entry(node.id()).or_default()
    }
}

/// What `layer`'s tree says about `target`. `None` when no node holds the
/// target's byte.
///
/// # Panics
/// If `layer`'s tree reaches past `text`'s end: it was not parsed from
/// `text`.
pub(super) fn compute(
    query: &IndentQuery,
    layer: &SyntaxLayer,
    text: &BufferText,
    target: Target,
    tab_width: u8,
) -> Option<Outcome> {
    let root = root_of(layer, text);
    let at = target.byte();
    let node = root.descendant_for_byte_range(at, at)?;
    let planned_break = match target {
        Target::NewLine(split) => Some(split),
        Target::Line(_) => None,
    };
    let lines = PlannedLines {
        text,
        at,
        planned_break,
    };
    let preceding = deepest_preceding(node, at);
    let candidate = planned_break
        .and(preceding)
        .and_then(|preceding| candidate_body(preceding, at));
    let start = preceding.map_or(at, |prev| prev.end_byte().saturating_sub(1));
    let end = candidate.map_or(at + 1, |candidate| candidate.end_byte());
    let captured = run(query, root, &lines, start..end, tab_width);
    let at_line = lines.source_line(at);
    if captured
        .opaque
        .iter()
        .any(|bytes| bytes.contains(&at) && lines.source_line(bytes.start) < at_line)
    {
        return Some(Outcome::Opaque);
    }
    let (start, extended) = start_node(node, preceding, candidate, &captured, &lines, tab_width);
    Some(Outcome::Levels(contain(
        start, &extended, &captured, &lines,
    )))
}

/// What an electric dedent answers: `trigger` landing at byte `cursor` of
/// a buffer whose word characters are `word_chars`.
#[derive(Clone, Copy)]
pub(super) struct ElectricInput<'w> {
    pub(super) cursor: usize,
    pub(super) trigger: ElectricTrigger,
    pub(super) word_chars: WordChars<'w>,
}

/// Whether the existing line whose first non-blank byte is `at` starts
/// with a complete token `layer`'s query captures as an outdent: the node
/// at `at` or an ancestor starting there. [`is_complete`] decides
/// completeness from `input`.
///
/// # Panics
/// If `layer`'s tree reaches past `text`'s end: it was not parsed from
/// `text`.
pub(super) fn starts_with_outdent(
    query: &IndentQuery,
    layer: &SyntaxLayer,
    text: &BufferText,
    at: usize,
    input: ElectricInput<'_>,
    tab_width: u8,
) -> bool {
    let root = root_of(layer, text);
    let Some(node) = root.descendant_for_byte_range(at, at) else {
        return false;
    };
    let lines = PlannedLines {
        text,
        at,
        planned_break: None,
    };
    let captured = run(query, root, &lines, at..at + 1, tab_width);
    std::iter::successors(Some(node), |node| node.parent())
        .take_while(|node| node.start_byte() == at)
        .any(|node| {
            captured
                .flags
                .get(&node.id())
                .is_some_and(|flags| flags.outdent || flags.outdent_always)
                && is_complete(query, text, node, input)
        })
}

/// Whether the token `node` spans is complete once `input`'s trigger
/// lands at its cursor. A line break completes a token ending at or before
/// the cursor and cuts one reaching past it. After a typed character, a
/// token ending before the cursor is complete; one reaching it is complete
/// only when typing on cannot extend it: its last character is not one of
/// the buffer's word characters, and its text is not a proper prefix of
/// another of the grammar's tokens.
fn is_complete(
    query: &IndentQuery,
    text: &BufferText,
    node: Node,
    input: ElectricInput<'_>,
) -> bool {
    let ElectricInput {
        cursor,
        trigger,
        word_chars,
    } = input;
    match trigger {
        ElectricTrigger::LineBreak => node.end_byte() <= cursor,
        ElectricTrigger::Typed if node.end_byte() < cursor => true,
        ElectricTrigger::Typed => {
            let Some(token) = text.covering_bytes(node.byte_range()) else {
                return false;
            };
            let ends_in_word = text
                .char_at(token.last().offset())
                .is_some_and(|last| word_chars.classify(last) == CharClass::Word);
            !ends_in_word && !query.is_token_prefix(&Cow::from(text.slice(token.chars())))
        }
    }
}

/// `layer`'s root node.
///
/// # Panics
/// If the tree reaches past `text`'s end: it was not parsed from `text`.
fn root_of<'t>(layer: &'t SyntaxLayer, text: &BufferText) -> Node<'t> {
    let root = layer.tree.root_node();
    assert!(
        root.end_byte() <= text.len_bytes(),
        "indent tree ends at byte {} past the buffer's {}: tree is stale",
        root.end_byte(),
        text.len_bytes()
    );
    root
}

/// The deepest last descendant of `node`'s last child ending at or before
/// `at`.
fn deepest_preceding(node: Node, at: usize) -> Option<Node> {
    let mut cursor = node.walk();
    let mut deepest = node
        .children(&mut cursor)
        .take_while(|child| child.end_byte() <= at)
        .last()?;
    while let Some(last) = deepest
        .child_count()
        .checked_sub(1)
        .and_then(|index| deepest.child(index))
    {
        deepest = last;
    }
    Some(deepest)
}

/// The node after the split that a construct ending before it is waiting
/// for, such as the body of `while (c)`: the first next sibling starting at
/// or after `split` on the way up from `preceding`, while the nodes passed
/// end at or before `split`.
fn candidate_body(preceding: Node, split: usize) -> Option<Node> {
    let mut x = preceding;
    while let Some(parent) = x.parent() {
        if x.end_byte() > split {
            return None;
        }
        if let Some(next) = x.next_sibling()
            && next.start_byte() >= split
        {
            return Some(next);
        }
        x = parent;
    }
    None
}

/// The node the containment walk starts from, and the nodes whose scope
/// extends over the new line.
///
/// A candidate a header-scoped pattern captured is the start. Otherwise
/// the walk from `preceding` up to `node` (exclusive) marks each `@extend`
/// node that ends on the target's source line, or starts on a line of
/// lower level than that line, as extended; an `@extend.prevent-once` node cancels
/// the next `@extend` above it. The innermost extended node is the start,
/// and `node` when there is none.
fn start_node<'t>(
    node: Node<'t>,
    preceding: Option<Node<'t>>,
    candidate: Option<Node<'t>>,
    captured: &Captured,
    lines: &PlannedLines,
    tab_width: u8,
) -> (Node<'t>, Vec<usize>) {
    let flags = |node: Node| captured.flags.get(&node.id());
    if let Some(candidate) = candidate
        && flags(candidate).is_some_and(|flags| flags.header_any)
    {
        return (candidate, Vec::new());
    }
    let at_line = lines.source_line(lines.at);
    let at_level = lines.level(lines.at, tab_width);
    let mut extended = Vec::new();
    let mut start = None;
    let mut stop = false;
    let walk = std::iter::successors(preceding, |x| x.parent()).take_while(|x| x.id() != node.id());
    for x in walk {
        stop |= flags(x).is_some_and(|flags| flags.prevent_once);
        if !flags(x).is_some_and(|flags| flags.extend) {
            continue;
        }
        if stop {
            stop = false;
        } else if lines.source_line(x.end_byte()) == at_line
            || at_level > lines.level(x.start_byte(), tab_width)
        {
            extended.push(x.id());
            start.get_or_insert(x);
        }
    }
    (start.unwrap_or(node), extended)
}

/// Runs `query` over `bytes` and collects what the matches whose
/// predicates hold capture, skipping MISSING nodes.
fn run(
    query: &IndentQuery,
    root: Node,
    lines: &PlannedLines,
    bytes: Range<usize>,
    tab_width: u8,
) -> Captured {
    let text = lines.text;
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(bytes).set_match_limit(MATCH_LIMIT);
    let mut captured = Captured::default();
    let mut matches = cursor.matches(&query.query, root, RopeProvider(text.rope()));
    while let Some(m) = matches.next() {
        if !query
            .predicates(m.pattern_index)
            .iter()
            .all(|predicate| holds(predicate, m, lines))
        {
            continue;
        }
        let header = query.is_header(m.pattern_index);
        let live = || {
            m.captures()
                .iter()
                .filter(|capture| !capture.node.is_missing())
        };
        let anchor = live()
            .find(|capture| query.capture(capture.index) == Some(IndentCapture::Anchor))
            .map(|capture| capture.node);
        for capture in live() {
            let node = capture.node;
            match query.capture(capture.index) {
                Some(IndentCapture::Indent) => {
                    let flags = captured.flags_of(node);
                    flags.indent = true;
                    flags.header_scope |= header;
                    flags.header_any |= header;
                }
                Some(IndentCapture::IndentAlways) => {
                    let flags = captured.flags_of(node);
                    flags.indent_always = true;
                    flags.header_scope |= header;
                    flags.header_any |= header;
                }
                Some(IndentCapture::Outdent) => {
                    let flags = captured.flags_of(node);
                    flags.outdent = true;
                    flags.header_any |= header;
                }
                Some(IndentCapture::OutdentAlways) => {
                    let flags = captured.flags_of(node);
                    flags.outdent_always = true;
                    flags.header_any |= header;
                }
                Some(IndentCapture::Extend) => captured.flags_of(node).extend = true,
                Some(IndentCapture::ExtendPreventOnce) => {
                    captured.flags_of(node).prevent_once = true;
                }
                Some(IndentCapture::Align) => {
                    if let Some(anchor) = anchor {
                        captured
                            .aligns
                            .entry(node.id())
                            .or_insert_with(|| anchor_column(text, anchor, tab_width));
                        captured.flags_of(node).header_any |= header;
                    }
                }
                Some(IndentCapture::Opaque) => captured.opaque.push(node.byte_range()),
                Some(IndentCapture::Anchor) | None => {}
            }
        }
    }
    captured
}

/// Whether `predicate` holds for `m`, reading lines as planned.
fn holds(predicate: &Predicate, m: &QueryMatch<'_, '_>, lines: &PlannedLines) -> bool {
    let first = |capture: u32| m.nodes_for_capture_index(capture).next();
    match predicate {
        Predicate::NotKindEq { capture, kind } => {
            first(*capture).is_none_or(|node| node.kind() != &**kind)
        }
        Predicate::SameLine { a, b, same } => match (first(*a), first(*b)) {
            (Some(a), Some(b)) => (lines.start(a) == lines.start(b)) == *same,
            _ => false,
        },
        Predicate::OneLine { capture, one } => {
            first(*capture).is_some_and(|node| (lines.start(node) == lines.end(node)) == *one)
        }
    }
}

/// The display column `anchor` starts at, in the text before the break.
fn anchor_column(text: &BufferText, anchor: Node, tab_width: u8) -> BufferLineCol {
    let at = text.byte_to_char(anchor.start_byte());
    text.columns()
        .display_col(text.char_to_line(at), at, tab_width)
}

/// Walks from `start` to the root, adding up the scopes that contain the
/// target line and the outdents that open it. An `extended` node contains
/// every line after its start. Plain outdents opening the line count once
/// together; each `@outdent.always` node counts. Under the innermost
/// containing alignment, only scopes opening on a line below the aligned
/// node's start line count: the anchor column covers the enclosing ones.
fn contain(start: Node, extended: &[usize], captured: &Captured, lines: &PlannedLines) -> Levels {
    let target = lines.target();
    let (mut counted, mut always) = (Vec::new(), Vec::new());
    let (mut outdented, mut outdents_always) = (false, 0);
    let mut align = None;
    for node in std::iter::successors(Some(start), |node| node.parent()) {
        let flags = captured.flags.get(&node.id());
        let aligned = captured.aligns.get(&node.id());
        if flags.is_none() && aligned.is_none() {
            continue;
        }
        let node_start = lines.start(node);
        let scope_start = match (flags, node.parent()) {
            (Some(flags), Some(parent)) if flags.header_scope => lines.start(parent),
            _ => node_start,
        };
        let contains =
            scope_start < target && (extended.contains(&node.id()) || target <= lines.end(node));
        if let Some(flags) = flags
            && !((flags.indent || flags.indent_always) && (flags.outdent || flags.outdent_always))
        {
            if flags.indent && contains && !counted.contains(&scope_start) {
                counted.push(scope_start);
            }
            if flags.indent_always && contains {
                always.push(scope_start);
            }
            if node_start == target {
                outdented |= flags.outdent;
                outdents_always += i32::from(flags.outdent_always);
            }
        }
        if align.is_none() && contains {
            align = aligned.map(|&column| (column, node_start));
        }
    }
    let indents: i32 = counted
        .iter()
        .chain(&always)
        .filter(|&&scope| align.is_none_or(|(_, aligned)| scope > aligned))
        .map(|_| 1)
        .sum();
    Levels {
        net: indents - i32::from(outdented) - outdents_always,
        align: align.map(|(column, _)| column),
    }
}
