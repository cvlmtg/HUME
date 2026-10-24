//! Direct unit tests for the pure per-line helper `decoration_providers.rs`
//! shares across its render bridges: `last_writer_per_line` (the per-line
//! collapse policy). Full-`Editor` integration coverage for the bridges that
//! call it lives in `tests/lsp_decorations.rs`, `tests/lsp_inlay_hints.rs`,
//! and `tests/lsp_signs.rs`; these tests isolate the shared logic itself so a
//! future kind reusing it inherits the same guarantees without redoing the
//! bridge-level plumbing.

use hume_rope::line::ContentLine;

use super::last_writer_per_line;

// ── `last_writer_per_line` ──────────────────────────────────────────────────

#[test]
fn last_writer_per_line_keeps_the_later_entry_within_one_source() {
    // Two entries from the same source on line 4: within one source, the
    // last entry wins.
    let entries = vec![
        ("diagnostics", ContentLine::new(4), "first"),
        ("diagnostics", ContentLine::new(4), "second"),
    ];
    let result = last_writer_per_line(entries);
    assert_eq!(result.get(&ContentLine::new(4)), Some(&"second"));
}

#[test]
fn last_writer_per_line_breaks_cross_source_ties_alphabetically_first() {
    // Across sources, ties break by source name: the alphabetically
    // *first* source wins. Input order does not match sort
    // order, so a fix that just returned "whichever came last in the
    // input" would pass by accident.
    let entries = vec![
        ("z-marks", ContentLine::new(7), "from-z"),
        ("a-marks", ContentLine::new(7), "from-a"),
    ];
    let result = last_writer_per_line(entries);
    assert_eq!(
        result.get(&ContentLine::new(7)),
        Some(&"from-a"),
        "the alphabetically first source (\"a-marks\") must win, matching \
         the sign pipeline's tie-break"
    );
}

#[test]
fn last_writer_per_line_keeps_entries_on_distinct_lines_independent() {
    let entries = vec![
        ("a", ContentLine::new(1), "one"),
        ("b", ContentLine::new(2), "two"),
    ];
    let result = last_writer_per_line(entries);
    assert_eq!(result.get(&ContentLine::new(1)), Some(&"one"));
    assert_eq!(result.get(&ContentLine::new(2)), Some(&"two"));
}
