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

// ── word_hunks (Phase 2b) ────────────────────────────────────────────────

/// Fail oracle: swap `old_start`/`old_end` for a pure-insert side (or emit a
/// non-empty `old_text`) — a plugin anchoring a `set-virtual-lines!` insert
/// marker at the wrong offset, or rendering phantom "removed" text.
#[test]
fn word_hunks_pure_insert_has_zero_width_old_side() {
    let (hunks, deadline_hit) = word_hunks("foo bar", "foo big bar");
    assert!(!deadline_hit);
    // "foo bar" tokens: ["foo", " ", "bar"] (offsets 0,3,4,7).
    // "foo big bar" tokens: ["foo", " ", "big", " ", "bar"] (offsets 0,3,4,7,8,11).
    assert_eq!(
        hunks,
        vec![WordDiffHunk {
            old_start: 4,
            old_end: 4,
            new_start: 4,
            new_end: 8,
            old_text: String::new(),
            new_text: "big ".to_string(),
        }]
    );
}

/// Mirror of `word_hunks_pure_insert_has_zero_width_old_side` — the deletion
/// direction, same oracle.
#[test]
fn word_hunks_pure_delete_has_zero_width_new_side() {
    let (hunks, deadline_hit) = word_hunks("foo big bar", "foo bar");
    assert!(!deadline_hit);
    assert_eq!(
        hunks,
        vec![WordDiffHunk {
            old_start: 4,
            old_end: 8,
            new_start: 4,
            new_end: 4,
            old_text: "big ".to_string(),
            new_text: String::new(),
        }]
    );
}

/// Fail oracle: a changed word reports non-empty text on both sides with
/// matching ranges — catches accidentally treating a `Replace` as a
/// `Delete` + `Insert` pair (two hunks instead of one).
#[test]
fn word_hunks_replace_carries_both_sides() {
    let (hunks, deadline_hit) = word_hunks("foo bar", "foo baz");
    assert!(!deadline_hit);
    assert_eq!(
        hunks,
        vec![WordDiffHunk {
            old_start: 4,
            old_end: 7,
            new_start: 4,
            new_end: 7,
            old_text: "bar".to_string(),
            new_text: "baz".to_string(),
        }]
    );
}

/// Fail oracle: stop filtering `WordHunkKind::Equal` — this returns an
/// all-equal hunk carrying no text instead of an empty list, and every
/// plugin decorates the whole line as changed.
#[test]
fn word_hunks_equal_runs_are_dropped() {
    let (hunks, deadline_hit) = word_hunks("foo bar", "foo bar");
    assert!(!deadline_hit);
    assert_eq!(hunks, Vec::new());
}

/// Fail oracle: tokenize on `split_whitespace` (collapsing runs) instead of
/// `split_word_bounds` — a whitespace-only edit inside a line would vanish
/// instead of surfacing as a hunk.
#[test]
fn word_hunks_whitespace_run_change_is_diffed() {
    let (hunks, deadline_hit) = word_hunks("a  b", "a b");
    assert!(!deadline_hit);
    // "a  b" tokens: ["a", "  ", "b"] (offsets 0,1,3,4).
    // "a b" tokens: ["a", " ", "b"] (offsets 0,1,2,3).
    assert_eq!(
        hunks,
        vec![WordDiffHunk {
            old_start: 1,
            old_end: 3,
            new_start: 1,
            new_end: 2,
            old_text: "  ".to_string(),
            new_text: " ".to_string(),
        }]
    );
}

/// Fail oracle: hardcode `word_hunks`'s `deadline_hit` return to `false` —
/// this pins that a forced `Duration::ZERO` deadline surfaces as `true`,
/// while the same input through the default-deadline path stays `false`.
#[test]
fn word_hunks_with_deadline_forwards_the_timeout_flag() {
    let old = "word ".repeat(100);
    let new = "term ".repeat(100);
    let (_, forced) = word_hunks_with_deadline(&old, &new, std::time::Duration::ZERO);
    assert!(forced, "a zero deadline must report deadline_hit");
    let (_, default) = word_hunks(&old, &new);
    assert!(!default, "the default deadline must complete on this input");
}
