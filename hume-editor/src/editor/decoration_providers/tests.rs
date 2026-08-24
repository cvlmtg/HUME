//! Direct unit tests for the pure per-line helpers `decoration_providers.rs`
//! shares across its render bridges — `last_writer_per_line` (the
//! per-line collapse policy) and `resolve_decoration_line` (the position
//! contract fix). Full-`Editor` integration coverage for the bridges that
//! call these lives in `tests/lsp_decorations.rs` and
//! `tests/lsp_inlay_hints.rs`; these tests isolate the shared logic itself
//! so a future kind reusing it inherits the same guarantees without redoing
//! the bridge-level plumbing.

use super::{last_writer_per_line, resolve_decoration_line};

// ── `resolve_decoration_line` ───────────────────────────────────────────────

#[test]
fn resolve_decoration_line_returns_the_line_for_a_content_position() {
    let text = hume_editing::text::BufferText::from("aaa\nbbb\nccc\n");
    // Line 2 ("ccc") starts at char offset 8.
    assert_eq!(resolve_decoration_line(&text, 8), Some(2));
}

#[test]
fn resolve_decoration_line_drops_a_position_on_the_trailing_phantom_line() {
    // "aaa\nbbb\nccc\n" is 12 chars; char offset 12 is `len_chars()`, which
    // resolves to the buffer's trailing phantom line (line 3, always empty
    // — see `host_impl.rs`'s `line_start_offset`). A fresh `set-*!` call can
    // never store this position (the host boundary now rejects it), but a
    // remap can produce it transiently when an edit deletes everything
    // after an anchor up to end-of-buffer — the entry must disappear, not
    // get relocated onto the preceding line.
    let text = hume_editing::text::BufferText::from("aaa\nbbb\nccc\n");
    assert_eq!(resolve_decoration_line(&text, 12), None);
}

// ── `last_writer_per_line` ──────────────────────────────────────────────────

#[test]
fn last_writer_per_line_keeps_the_later_entry_within_one_source() {
    // Two entries from the same source collapsed onto line 4 by a remap —
    // within one source, the last entry wins.
    let entries = vec![
        ("diagnostics".to_string(), 4, "first"),
        ("diagnostics".to_string(), 4, "second"),
    ];
    let result = last_writer_per_line(entries);
    assert_eq!(result.get(&4), Some(&"second"));
}

#[test]
fn last_writer_per_line_breaks_cross_source_ties_alphabetically_first() {
    // Across sources, ties break by source name —
    // mirroring the sign pipeline's tie-break, which resolves to the
    // alphabetically *first* source (`update_sign_providers`'s ascending
    // pre-sort + a stable priority sort keep same-priority ties in that
    // order). Input order deliberately does not match sort order, so a
    // fix that just returned "whichever came last in the input" would
    // pass by accident.
    let entries = vec![
        ("z-marks".to_string(), 7, "from-z"),
        ("a-marks".to_string(), 7, "from-a"),
    ];
    let result = last_writer_per_line(entries);
    assert_eq!(
        result.get(&7),
        Some(&"from-a"),
        "the alphabetically first source (\"a-marks\") must win, matching \
         the sign pipeline's tie-break"
    );
}

#[test]
fn last_writer_per_line_keeps_entries_on_distinct_lines_independent() {
    let entries = vec![("a".to_string(), 1, "one"), ("b".to_string(), 2, "two")];
    let result = last_writer_per_line(entries);
    assert_eq!(result.get(&1), Some(&"one"));
    assert_eq!(result.get(&2), Some(&"two"));
}
