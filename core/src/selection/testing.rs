//! Test helper: parse the DSL marker format into `(Text, SelectionSet)`.
//!
//! The format is identical to the one used in `editor/src/testing.rs`; this
//! copy lives here so that `core` crate unit tests can use it without
//! depending on the editor crate.

use crate::selection::{Selection, SelectionSet};
use crate::text::Text;

fn char_count(s: &str) -> usize {
    s.chars().count()
}

/// Parse a marker-annotated string into `(Text, SelectionSet)`.
///
/// Marker syntax: `-[anchor…head]>` (forward), `<[head…anchor]-` (backward).
/// Every DSL string must end with `\n` and contain at least one selection.
pub fn parse_state(input: &str) -> (Text, SelectionSet) {
    let mut text = String::with_capacity(input.len());
    let mut selections: Vec<Selection> = Vec::new();

    #[derive(Debug)]
    enum State {
        Normal,
        InForward { anchor_offset: usize },
        InBackward { head_offset: usize },
    }

    let mut state = State::Normal;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match (&state, ch) {
            (State::Normal, '-') if chars.peek() == Some(&'[') => {
                chars.next();
                state = State::InForward { anchor_offset: char_count(&text) };
            }
            (State::Normal, '<') if chars.peek() == Some(&'[') => {
                chars.next();
                state = State::InBackward { head_offset: char_count(&text) };
            }
            (State::InForward { anchor_offset }, ']') if chars.peek() == Some(&'>') => {
                chars.next();
                let count = char_count(&text);
                assert!(count > *anchor_offset, "parse_state: empty selection in {input:?}");
                selections.push(Selection::new(*anchor_offset, count - 1));
                state = State::Normal;
            }
            (State::InBackward { head_offset }, ']') if chars.peek() == Some(&'-') => {
                chars.next();
                let count = char_count(&text);
                assert!(count > *head_offset, "parse_state: empty selection in {input:?}");
                selections.push(Selection::new(count - 1, *head_offset));
                state = State::Normal;
            }
            (_, ']') | (_, '-') | (_, '<') => text.push(ch),
            (_, c) => text.push(c),
        }
    }
    assert!(
        !selections.is_empty(),
        "parse_state: no selection markers in {input:?}"
    );
    assert!(text.ends_with('\n'), "parse_state: buffer must end with '\\n'");
    (Text::from(text.as_str()), SelectionSet::from_vec(selections, 0))
}
