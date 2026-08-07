use super::*;

/// Fail oracle: stop filtering `LineHunkKind::Equal` — this returns one
/// all-equal hunk instead of none, and every plugin decorates the whole
/// file as changed.
#[test]
fn equal_hunks_are_dropped() {
    let old = Text::from("a\nb\nc\n");
    let new = Text::from("a\nb\nc\n");
    assert_eq!(line_hunks(&old, &new), Vec::new());
}

/// Fail oracle: adopt `vim.diff`'s 1-based starts (or shift a zero-count
/// side's anchor) — `old_start`/`new_start` become `2`, and every
/// `set-signs!`/`set-virtual-lines!` call built from this lands one line
/// off from the plugin's own 0-based line numbering.
#[test]
fn pure_insert_is_zero_based_with_no_old_side() {
    let old = Text::from("a\nb\n");
    let new = Text::from("a\nx\nb\n");
    assert_eq!(
        line_hunks(&old, &new),
        vec![DiffHunk {
            old_start: 1,
            old_count: 0,
            new_start: 1,
            new_count: 1,
            old_lines: vec![],
            new_lines: vec!["x".to_string()],
        }]
    );
}

/// Mirror of `pure_insert_is_zero_based_with_no_old_side` — same oracle,
/// the deletion direction.
#[test]
fn pure_delete_is_zero_based_with_no_new_side() {
    let old = Text::from("a\nx\nb\n");
    let new = Text::from("a\nb\n");
    assert_eq!(
        line_hunks(&old, &new),
        vec![DiffHunk {
            old_start: 1,
            old_count: 1,
            new_start: 1,
            new_count: 0,
            old_lines: vec!["x".to_string()],
            new_lines: vec![],
        }]
    );
}

/// Fail oracle: a changed line reports non-empty line lists on both sides
/// with matching counts — catches accidentally treating a `Replace` as a
/// `Delete` + `Insert` pair (which would emit two hunks instead of one).
#[test]
fn replace_carries_both_sides() {
    let old = Text::from("a\nb\nc\n");
    let new = Text::from("a\nB\nc\n");
    assert_eq!(
        line_hunks(&old, &new),
        vec![DiffHunk {
            old_start: 1,
            old_count: 1,
            new_start: 1,
            new_count: 1,
            old_lines: vec!["b".to_string()],
            new_lines: vec!["B".to_string()],
        }]
    );
}

/// Fail oracle: build `old_lines` by splitting `LineHunkKind::Delete`'s
/// payload instead of re-slicing the tokenized input — the payload joins
/// covered lines with no separator (`hume-editing/src/diff.rs`'s
/// `ops_to_line_hunks`), so a multi-line delete would come back as one
/// glued `["xy"]` instead of two separate lines.
#[test]
fn multi_line_delete_rebuilds_lines_by_slicing_not_from_the_payload() {
    let old = Text::from("a\nx\ny\nb\n");
    let new = Text::from("a\nb\n");
    assert_eq!(
        line_hunks(&old, &new),
        vec![DiffHunk {
            old_start: 1,
            old_count: 2,
            new_start: 1,
            new_count: 0,
            old_lines: vec!["x".to_string(), "y".to_string()],
            new_lines: vec![],
        }]
    );
}

/// Fail oracle: drop `Text::from`'s forced trailing newline (or tokenize on
/// a bare `split('\n')`) — a missing final newline on one side (routine for
/// a `git show` blob) would then report a phantom trailing-line hunk on
/// every file, on every refresh.
#[test]
fn missing_trailing_newline_is_not_a_change() {
    let old = Text::from("a\nb");
    let new = Text::from("a\nb\n");
    assert_eq!(line_hunks(&old, &new), Vec::new());
}

/// Fail oracle: normalize CRLF to LF before comparing (or leave `\r` in a
/// token) — either would make this either report zero hunks (masking a
/// real line-ending change some other way) or an unpredictable per-`\r`
/// diff. Pins the deliberate decision: `Text::from` already strips `\r` on
/// both sides, so a checked-in CRLF ref against HUME's own CRLF-normalized
/// buffer reports no difference — matching what a save would actually
/// produce, not `git diff`'s raw byte comparison.
#[test]
fn crlf_ref_is_normalized_like_the_buffer() {
    let old = Text::from("a\r\nb\r\n");
    let new = Text::from("a\nb\n");
    assert_eq!(line_hunks(&old, &new), Vec::new());
}

/// Fail oracle: stop stripping the trailing `\n` from a hunk's line
/// payloads — a plugin splicing one straight into `set-virtual-lines!`'s
/// row text would embed a newline mid-row.
#[test]
fn payload_lines_never_carry_a_trailing_newline() {
    let old = Text::from("a\nx\ny\nb\n");
    let new = Text::from("a\nb\n");
    let hunks = line_hunks(&old, &new);
    for hunk in &hunks {
        for line in hunk.old_lines.iter().chain(hunk.new_lines.iter()) {
            assert!(!line.contains('\n'), "line {line:?} carries a newline");
        }
    }
}
