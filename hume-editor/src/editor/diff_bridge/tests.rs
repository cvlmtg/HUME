use super::*;

/// Fail oracle: stop filtering `LineHunkKind::Equal` — this returns one
/// all-equal hunk instead of none, and every plugin decorates the whole
/// file as changed.
#[test]
fn equal_hunks_are_dropped() {
    assert_eq!(line_hunks("a\nb\nc\n", "a\nb\nc\n"), Vec::new());
}

/// Fail oracle: adopt `vim.diff`'s 1-based starts (or shift a zero-count
/// side's anchor) — `old_start`/`new_start` become `2`, and every
/// `set-signs!`/`set-virtual-lines!` call built from this lands one line
/// off from the plugin's own 0-based line numbering.
#[test]
fn pure_insert_is_zero_based_with_no_old_side() {
    assert_eq!(
        line_hunks("a\nb\n", "a\nx\nb\n"),
        vec![DiffHunk {
            old_start: 1,
            new_start: 1,
            old_lines: vec![],
            new_lines: vec!["x".to_string()],
        }]
    );
}

/// Mirror of `pure_insert_is_zero_based_with_no_old_side` — same oracle,
/// the deletion direction.
#[test]
fn pure_delete_is_zero_based_with_no_new_side() {
    assert_eq!(
        line_hunks("a\nx\nb\n", "a\nb\n"),
        vec![DiffHunk {
            old_start: 1,
            new_start: 1,
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
    assert_eq!(
        line_hunks("a\nb\nc\n", "a\nB\nc\n"),
        vec![DiffHunk {
            old_start: 1,
            new_start: 1,
            old_lines: vec!["b".to_string()],
            new_lines: vec!["B".to_string()],
        }]
    );
}

/// Fail oracle: build `old_lines` by splitting `LineHunkKind::Delete`'s
/// payload instead of re-slicing the tokenized input — `LineHunkKind`
/// carries no payload, so this pins the alternative: a multi-line delete
/// must come back as two separate lines, not one glued string.
#[test]
fn multi_line_delete_rebuilds_lines_by_slicing_the_tokenized_input() {
    assert_eq!(
        line_hunks("a\nx\ny\nb\n", "a\nb\n"),
        vec![DiffHunk {
            old_start: 1,
            new_start: 1,
            old_lines: vec!["x".to_string(), "y".to_string()],
            new_lines: vec![],
        }]
    );
}

/// Fail oracle: drop `BufferText::from`'s forced trailing newline (or tokenize on
/// a bare `split('\n')`) — a missing final newline on one side (routine for
/// a `git show` blob) would then report a phantom trailing-line hunk on
/// every file, on every refresh.
#[test]
fn missing_trailing_newline_is_not_a_change() {
    assert_eq!(line_hunks("a\nb", "a\nb\n"), Vec::new());
}

/// Fail oracle: normalize CRLF to LF before comparing (or leave `\r` in a
/// token) — either would make this either report zero hunks (masking a
/// real line-ending change some other way) or an unpredictable per-`\r`
/// diff. Pins the deliberate decision: `BufferText::from` already strips `\r` on
/// both sides, so a checked-in CRLF ref against HUME's own CRLF-normalized
/// buffer reports no difference — matching what a save would actually
/// produce, not `git diff`'s raw byte comparison.
#[test]
fn crlf_ref_is_normalized_like_the_buffer() {
    assert_eq!(line_hunks("a\r\nb\r\n", "a\nb\n"), Vec::new());
}

/// Fail oracle: strip only `'\n'` in `strip_newlines` — `BufferText::line_tokens`
/// is backed by `Rope::lines()`, which (ropey's default `unicode_lines`
/// feature) breaks on far more than LF, and `BufferText::from` only normalizes
/// `\r\n` pairs, so a form feed (U+000C) or bare `\r` reaches a `DiffHunk`
/// line string still carrying its terminator — exactly the control char a
/// plugin would then render straight into a `set-virtual-lines!` row.
#[test]
fn line_hunks_strips_non_lf_unicode_line_breaks() {
    // The change sits on the FF-terminated line itself (not a later
    // LF-terminated line) — otherwise `strip_suffix('\n')` alone would
    // still pass, since the *changed* line would happen to end in `\n`.
    assert_eq!(
        line_hunks("a\u{0C}b\n", "x\u{0C}b\n"),
        vec![DiffHunk {
            old_start: 0,
            new_start: 0,
            old_lines: vec!["a".to_string()],
            new_lines: vec!["x".to_string()],
        }]
    );
}

/// Mirror of `line_hunks_strips_non_lf_unicode_line_breaks` — a bare `\r`
/// (old Mac), which `BufferText::from` explicitly leaves untouched
/// (`hume-editing/src/text.rs`'s `normalize_crlf` doc) unlike a `\r\n` pair.
#[test]
fn line_hunks_strips_bare_cr_line_break() {
    // Same shape as `line_hunks_strips_non_lf_unicode_line_breaks`: the
    // change sits on the CR-terminated line itself.
    assert_eq!(
        line_hunks("a\rb\n", "x\rb\n"),
        vec![DiffHunk {
            old_start: 0,
            new_start: 0,
            old_lines: vec!["a".to_string()],
            new_lines: vec!["x".to_string()],
        }]
    );
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
    let (_, forced) = convert_word_diff(hume_editing::diff::diff_words_with_deadline(
        &old,
        &new,
        std::time::Duration::ZERO,
    ));
    assert!(forced, "a zero deadline must report deadline_hit");
    let (_, default) = word_hunks(&old, &new);
    assert!(!default, "the default deadline must complete on this input");
}
