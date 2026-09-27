//! # Comment vocabulary
//!
//! Comments describe the code as it is. Notes about how a test was
//! validated ("Fail oracle:", "Flip:", "red run"), how a bug was fixed
//! ("before the fix"), or which internal lesson a rule came from belong in
//! commit messages, where they stay attached to the change that made them
//! true. This lint rejects those phrases in any `//` comment in the
//! workspace, tests included.
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
    " red run",
    "before the fix",
    "Before the fix",
    "With the fix",
    "Without the fix",
    "LESSONS.md",
    "⚠️",
];

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
            if let Some(hit) = FORBIDDEN.iter().find(|p| comment.contains(**p)) {
                let file = path.strip_prefix(&root).unwrap_or(path).display();
                violations.push(format!("{file}:{}: {hit:?}", i + 1));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "\nWorkflow vocabulary in comments. State the behaviour instead, and\n\
         put validation notes or fix history in the commit message.\n{}\n",
        violations.join("\n")
    );
}
