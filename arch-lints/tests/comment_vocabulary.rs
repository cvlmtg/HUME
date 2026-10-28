//! # Comment vocabulary
//!
//! Comments describe the code as it is. Notes about how a test was
//! validated ("Fail oracle:", "Flip:", "Validity:", "red run"), how a bug was fixed
//! ("before the fix"), or which internal lesson a rule came from belong in
//! commit messages, where they stay attached to the change that made them
//! true. Emphasis words ("deliberately", "load-bearing") add no fact a
//! plain sentence lacks. This lint rejects those phrases in any `//`
//! comment in the workspace, tests included.
//!
//! The retired kind-A/B/C names for what a builtin needs from a pane are
//! rejected in either spelling ([`retired_kind_term`]): the pane argument
//! types of the `builtins!` table name those needs.
//!
//! Only unambiguous phrases are listed. History wording such as "no longer"
//! or "used to" also has legitimate present-tense uses, so it is left to
//! review.

use arch_lints::{workspace_all_rs_paths, workspace_root};

const FORBIDDEN: &[&str] = &[
    "Fail oracle",
    "Independent oracle",
    "Flip:",
    "Verification:",
    "Validity:",
    " red run",
    "before the fix",
    "Before the fix",
    "With the fix",
    "Without the fix",
    "LESSONS.md",
    "⚠️",
];

/// Matched in any case, so a sentence may not open with one either.
const EMPHASIS: &[&str] = &[
    "load-bearing",
    "deliberately",
    "genuinely",
    "by construction",
];

/// `kind-` and a/b/c in any case, or `kind` in any case, a space and an
/// uppercase A/B/C, as a whole word. A lowercase letter after the space is
/// ordinary English ("the gate kind a builtin declares").
fn retired_kind_term(comment: &str) -> Option<&str> {
    let bytes = comment.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    bytes
        .windows(4)
        .enumerate()
        .filter(|(_, window)| window.eq_ignore_ascii_case(b"kind"))
        .find_map(|(at, _)| {
            let starts_word = at == 0 || !is_word(bytes[at - 1]);
            let (separator, letter) = (*bytes.get(at + 4)?, *bytes.get(at + 5)?);
            let ends_word = bytes.get(at + 6).is_none_or(|&b| !is_word(b));
            let names_a_kind = match separator {
                b'-' => matches!(letter.to_ascii_uppercase(), b'A'..=b'C'),
                b' ' => matches!(letter, b'A'..=b'C'),
                _ => false,
            };
            (starts_word && ends_word && names_a_kind).then(|| &comment[at..at + 6])
        })
}

#[test]
fn comments_carry_no_workflow_vocabulary() {
    let root = workspace_root();
    let this_file = root.join("arch-lints/tests/comment_vocabulary.rs");

    let mut paths = workspace_all_rs_paths(&root);
    assert!(
        paths.len() > 100,
        "found only {} .rs files; the member scan is broken",
        paths.len()
    );
    paths.retain(|p| *p != this_file);

    let mut violations = Vec::new();
    for path in &paths {
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        for (i, line) in src.lines().enumerate() {
            let Some(comment) = line.find("//").map(|at| &line[at..]) else {
                continue;
            };
            let lower = comment.to_lowercase();
            let hit = FORBIDDEN
                .iter()
                .find(|p| comment.contains(**p))
                .or_else(|| EMPHASIS.iter().find(|p| lower.contains(**p)))
                .copied()
                .or_else(|| retired_kind_term(comment));
            if let Some(hit) = hit {
                let file = path.strip_prefix(&root).unwrap_or(path).display();
                violations.push(format!("{file}:{}: {hit:?}", i + 1));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "\nWorkflow or emphasis vocabulary in comments. State the behaviour\n\
         instead, and put validation notes or fix history in the commit message.\n{}\n",
        violations.join("\n")
    );
}

#[test]
fn retired_kind_terms_match_in_either_spelling() {
    for hit in ["a kind B builtin", "Kind-b", "(kind-A)", "KIND C."] {
        assert!(retired_kind_term(hit).is_some(), "{hit:?}");
    }
    for miss in [
        "the gate kind a builtin declares",
        "kind-color",
        "kind Ab",
        "unkind-a",
    ] {
        assert_eq!(retired_kind_term(miss), None, "{miss:?}");
    }
}
