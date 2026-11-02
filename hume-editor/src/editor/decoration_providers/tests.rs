//! Unit tests for `first_per_line`, the per-line fold the EOL text and line
//! background bridges share. Full-`Editor` coverage for those bridges lives
//! in `tests/lsp_diagnostics_inline.rs` and `tests/lsp_decorations.rs`.

use hume_rope::line::ContentLine;

use super::first_per_line;

#[test]
fn first_per_line_keeps_the_first_entry_for_a_line() {
    let entries = vec![
        (ContentLine::new(4), "first"),
        (ContentLine::new(4), "second"),
    ];
    let result = first_per_line(entries);
    assert_eq!(result.get(&ContentLine::new(4)), Some(&"first"));
}

#[test]
fn first_per_line_keeps_entries_on_distinct_lines_independent() {
    let entries = vec![(ContentLine::new(1), "one"), (ContentLine::new(2), "two")];
    let result = first_per_line(entries);
    assert_eq!(result.get(&ContentLine::new(1)), Some(&"one"));
    assert_eq!(result.get(&ContentLine::new(2)), Some(&"two"));
}
