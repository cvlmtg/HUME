//! The marker notation tests write a text and its selections in.
//!
//! | Marker | Meaning |
//! |--------|---------|
//! | `-[`   | Anchor side opening a forward selection. |
//! | `]>`   | Head side closing a forward selection. |
//! | `<[`   | Head side opening a backward selection. |
//! | `]-`   | Anchor side closing a backward selection. |
//!
//! With two or more selections, the primary one is written with braces
//! (`-{`/`}>`, `<{`/`}-`) and every other with brackets. A lone selection
//! is always primary and always bracketed.
//!
//! ```text
//! -[hell]>o\n      forward: anchor on 'h', head on the second 'l'
//! <[hell]-o\n      backward: head on 'h', anchor on the second 'l'
//! hel-[l]>o\n      a cursor on the second 'l'
//! a-[e\u{301}]>b\n a cursor on the whole accented letter
//! -[a]>b-{c}>\n    two cursors, the one on 'c' primary
//! ```
//!
//! The text between the markers is what the selection covers. A
//! marker must sit between two clusters; one inside a cluster panics. Every
//! notation ends with the text's structural `\n` and holds at least one
//! selection. A closing marker with no selection open is literal text.

use hume_rope::cluster::{ClusterBound, ClusterRange, ClusterStart};
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::selection::{EditView, Facing, Selection};
use crate::state::EditState;
use crate::text::BufferText;

/// The text and selections `input` describes.
///
/// # Panics
/// Panics on malformed notation: no selection, an empty, unterminated or
/// nested selection, a selection closed by the other kind of marker, a
/// braced lone selection, several selections with no primary or with two,
/// a `\r`, a missing final `\n`, or a marker inside a cluster.
pub fn parse(input: &str) -> EditState {
    // Marker offsets count the notation's chars, and the text normalizes
    // line endings, so a `\r` would shift every marker after it.
    assert!(
        !input.contains('\r'),
        "marked::parse: {input:?} holds a `\\r`; notation is LF-only"
    );
    let mut text = String::with_capacity(input.len());
    let mut chars_so_far = 0usize;
    // (open marker offset, close marker offset, facing, mark)
    let mut spans: Vec<(usize, usize, Facing, Mark)> = Vec::new();
    let mut open: Option<(usize, Facing, Mark)> = None;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        let next = chars.peek().copied();
        let opener = match (ch, next) {
            ('-', Some('[')) => Some((Facing::Forward, Mark::Plain)),
            ('-', Some('{')) => Some((Facing::Forward, Mark::Primary)),
            ('<', Some('[')) => Some((Facing::Backward, Mark::Plain)),
            ('<', Some('{')) => Some((Facing::Backward, Mark::Primary)),
            _ => None,
        };
        match (open, opener) {
            (None, Some((facing, mark))) => {
                chars.next();
                open = Some((chars_so_far, facing, mark));
                continue;
            }
            (Some(_), Some(_)) => {
                panic!("marked::parse: a selection opened inside another in {input:?}")
            }
            _ => {}
        }
        if let Some((start, facing, mark)) = open
            && next == Some(close_char(facing))
            && let Some(closer) = Mark::closed_by(ch)
        {
            assert!(
                closer == mark,
                "marked::parse: a selection opened with {:?} is closed by {:?} in {input:?}",
                mark.bracket(),
                closer.bracket()
            );
            chars.next();
            assert!(
                chars_so_far > start,
                "marked::parse: empty selection in {input:?}; a selection covers at least one cluster"
            );
            spans.push((start, chars_so_far, facing, mark));
            open = None;
            continue;
        }
        text.push(ch);
        chars_so_far += 1;
    }

    assert!(
        open.is_none(),
        "marked::parse: unterminated selection in {input:?}"
    );
    assert!(
        !spans.is_empty(),
        "marked::parse: no selection in {input:?}; add a `-[x]>` cursor"
    );
    assert!(
        text.ends_with('\n'),
        "marked::parse: {input:?} must end with the text's structural '\\n'"
    );
    let mut primaries = spans
        .iter()
        .enumerate()
        .filter(|(_, (.., mark))| *mark == Mark::Primary)
        .map(|(i, _)| i);
    let primary = match (spans.len(), primaries.next(), primaries.next()) {
        (1, None, _) => 0,
        (1, Some(_), _) => panic!(
            "marked::parse: a single selection is primary; write it with `[`, not `{{`, in {input:?}"
        ),
        (_, None, _) => panic!(
            "marked::parse: no primary in {input:?}; write the primary selection with `{{` and `}}`"
        ),
        (_, Some(i), None) => i,
        (_, Some(_), Some(_)) => panic!("marked::parse: two primaries in {input:?}"),
    };

    let text = BufferText::from(text.as_str());
    let selections = spans
        .into_iter()
        .map(|(start, end, facing, _)| {
            let chars = ExclusiveRange::new(CharOffset::new(start), CharOffset::new(end));
            let range = text
                .covering(chars)
                .filter(|range| range.chars() == chars)
                .unwrap_or_else(|| {
                    panic!(
                        "marked::parse: a selection marker splits a grapheme cluster in {input:?}"
                    )
                });
            Selection::covering(range, facing)
        })
        .collect();
    EditState::from_text(text, selections, primary)
}

/// Whether a selection's markers name it the primary one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    Plain,
    Primary,
}

impl Mark {
    /// The mark a closing `]` or `}` belongs to.
    fn closed_by(ch: char) -> Option<Mark> {
        match ch {
            ']' => Some(Mark::Plain),
            '}' => Some(Mark::Primary),
            _ => None,
        }
    }

    fn bracket(self) -> char {
        match self {
            Mark::Plain => '[',
            Mark::Primary => '{',
        }
    }
}

/// The char after `]`/`}` that closes a selection facing `facing`.
fn close_char(facing: Facing) -> char {
    match facing {
        Facing::Forward => '>',
        Facing::Backward => '-',
    }
}

/// `view` in marker notation: the inverse of [`parse`].
pub fn render(view: EditView<'_>) -> String {
    let text = view.text().to_string();
    let len = view.text().len_chars();
    let mut markers: Vec<Vec<&'static str>> = vec![Vec::new(); len + 1];
    let several = view.len() > 1;
    for sel in view.iter() {
        let range = sel.covered();
        let (open, close) = match (sel.facing(), several && sel.is_primary()) {
            (Facing::Forward, false) => ("-[", "]>"),
            (Facing::Backward, false) => ("<[", "]-"),
            (Facing::Forward, true) => ("-{", "}>"),
            (Facing::Backward, true) => ("<{", "}-"),
        };
        markers[range.start().offset().index()].push(open);
        markers[range.end().offset().index()].push(close);
    }

    let mut out = String::with_capacity(text.len() + view.len() * 4);
    let mut chars = text.chars();
    for at in markers {
        for marker in at {
            out.push_str(marker);
        }
        if let Some(c) = chars.next() {
            out.push(c);
        }
    }
    out
}

/// The cluster starting at char `n` of `text`, so a test's literal offset
/// names the position it means.
///
/// # Panics
/// Panics if char `n` is not a cluster start.
pub fn start_at(text: &BufferText, n: usize) -> ClusterStart {
    let start = text.snap(CharOffset::new(n));
    assert_eq!(
        start.offset().index(),
        n,
        "marked::start_at: char {n} is not a cluster start"
    );
    start
}

/// The cluster boundary at char `n` of `text`.
///
/// # Panics
/// Panics if char `n` is inside a cluster or past the text end.
pub fn bound_at(text: &BufferText, n: usize) -> ClusterBound {
    if n == text.len_chars() {
        return hume_rope::grapheme::text_end(text.full_slice());
    }
    start_at(text, n).into()
}

/// The clusters covering chars `a..b` of `text`, clamped to the text.
///
/// # Panics
/// Panics if the range covers no cluster.
pub fn clusters(text: &BufferText, a: usize, b: usize) -> ClusterRange {
    text.covering(ExclusiveRange::new(CharOffset::new(a), CharOffset::new(b)))
        .expect("marked::clusters: the range covers no cluster")
}

#[cfg(test)]
mod tests;
