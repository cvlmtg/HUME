//! The marker notation tests write a text and its selections in.
//!
//! | Marker | Meaning |
//! |--------|---------|
//! | `-[`   | Anchor side opening a forward selection. |
//! | `]>`   | Head side closing a forward selection. |
//! | `<[`   | Head side opening a backward selection. |
//! | `]-`   | Anchor side closing a backward selection. |
//!
//! ```text
//! -[hell]>o\n      forward: anchor on 'h', head on the second 'l'
//! <[hell]-o\n      backward: head on 'h', anchor on the second 'l'
//! hel-[l]>o\n      a cursor on the second 'l'
//! a-[e\u{301}]>b\n a cursor on the whole accented letter
//! ```
//!
//! The text between the brackets is what the selection covers. A
//! marker must sit between two clusters; one inside a cluster panics. Every
//! notation ends with the text's structural `\n` and holds at least one
//! selection.

use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::selection::{EditView, Facing, Selection};
use crate::state::EditState;
use crate::text::BufferText;

/// The text and selections `input` describes. The first selection is primary.
///
/// # Panics
/// Panics on malformed notation: no selection, an empty, unterminated or
/// nested selection, a `\r`, a missing final `\n`, or a marker inside a
/// cluster.
pub fn parse(input: &str) -> EditState {
    // Marker offsets count the notation's chars, and the text normalizes
    // line endings, so a `\r` would shift every marker after it.
    assert!(
        !input.contains('\r'),
        "marked::parse: {input:?} holds a `\\r`; notation is LF-only"
    );
    let mut text = String::with_capacity(input.len());
    let mut chars_so_far = 0usize;
    // (open marker offset, close marker offset, facing)
    let mut spans: Vec<(usize, usize, Facing)> = Vec::new();
    let mut open: Option<(usize, Facing)> = None;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        let next = chars.peek().copied();
        match (open, ch, next) {
            (None, '-', Some('[')) => {
                chars.next();
                open = Some((chars_so_far, Facing::Forward));
            }
            (None, '<', Some('[')) => {
                chars.next();
                open = Some((chars_so_far, Facing::Backward));
            }
            (Some(_), '-' | '<', Some('[')) => {
                panic!("marked::parse: a selection opened inside another in {input:?}")
            }
            (Some((start, Facing::Forward)), ']', Some('>'))
            | (Some((start, Facing::Backward)), ']', Some('-')) => {
                chars.next();
                assert!(
                    chars_so_far > start,
                    "marked::parse: empty selection in {input:?}; a selection covers at least one cluster"
                );
                let facing = open.map(|(_, f)| f).expect("matched an open selection");
                spans.push((start, chars_so_far, facing));
                open = None;
            }
            (_, c, _) => {
                text.push(c);
                chars_so_far += 1;
            }
        }
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

    let text = BufferText::from(text.as_str());
    let selections = spans
        .into_iter()
        .map(|(start, end, facing)| {
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
    EditState::from_text(text, selections, 0)
}

/// `view` in marker notation: the inverse of [`parse`].
pub fn render(view: EditView<'_>) -> String {
    let text = view.text().to_string();
    let len = view.text().len_chars();
    let mut markers: Vec<Vec<&'static str>> = vec![Vec::new(); len + 1];
    for sel in view.iter() {
        let range = sel.covered();
        let (open, close) = match sel.facing() {
            Facing::Forward => ("-[", "]>"),
            Facing::Backward => ("<[", "]-"),
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

#[cfg(test)]
mod tests;
