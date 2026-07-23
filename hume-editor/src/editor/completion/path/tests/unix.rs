//! Unix-only tests, gated once at the `mod unix;` declaration
//! in the parent.

use super::*;

#[test]
fn path_completer_tilde_expands_for_lookup_keeps_literal_replacement() {
    use std::borrow::Cow;

    let home_dir = tempfile::tempdir().unwrap();
    std::fs::write(home_dir.path().join("notes.md"), b"").unwrap();
    std::fs::create_dir(home_dir.path().join("code")).unwrap();

    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let cwd = Path::new("/tmp");
    let langs = LanguageRegistry::new();
    let ctx = CompletionCtx {
        registry: &reg,
        buffers: &store,
        cwd,
        languages: &langs,
    };

    let home = home_dir.path().to_path_buf();
    let input = "e ~/";
    let result = PathCompleter { dirs_only: false }.complete_with_expand(
        input,
        input.len(),
        &ctx,
        |s: &str| {
            if let Some(tail) = s.strip_prefix('~')
                && (tail.is_empty() || tail.starts_with('/'))
            {
                return Cow::Owned(format!("{}{tail}", home.display()));
            }
            Cow::Borrowed(s)
        },
    );

    // Candidates must be present (the temp home has files).
    assert!(
        !result.candidates.is_empty(),
        "tilde should resolve to home and list entries"
    );
    // Replacements must keep the literal `~/` prefix, not expand to the absolute path.
    assert!(
        result
            .candidates
            .iter()
            .all(|c| c.replacement.starts_with("~/")),
        "replacements must preserve the `~/` prefix"
    );
    let names: Vec<&str> = result
        .candidates
        .iter()
        .map(|c| c.display.as_str())
        .collect();
    assert!(names.contains(&"notes.md"), "notes.md should appear");
    assert!(
        names.contains(&"code/"),
        "code/ directory should appear with trailing /"
    );
}

#[test]
fn path_completer_dollar_var_expands_for_lookup() {
    use std::borrow::Cow;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.rs"), b"").unwrap();

    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let cwd = Path::new("/tmp");
    let langs = LanguageRegistry::new();
    let ctx = CompletionCtx {
        registry: &reg,
        buffers: &store,
        cwd,
        languages: &langs,
    };

    let expanded = dir.path().to_string_lossy().into_owned();
    let input = "e $MYDIR/";
    let result = PathCompleter { dirs_only: false }.complete_with_expand(
        input,
        input.len(),
        &ctx,
        |s: &str| {
            if let Some(rest) = s.strip_prefix("$MYDIR") {
                Cow::Owned(format!("{expanded}{rest}"))
            } else {
                Cow::Borrowed(s)
            }
        },
    );

    assert!(
        !result.candidates.is_empty(),
        "$MYDIR should expand and list entries"
    );
    assert!(
        result
            .candidates
            .iter()
            .all(|c| c.replacement.starts_with("$MYDIR/"))
    );
    let names: Vec<&str> = result
        .candidates
        .iter()
        .map(|c| c.display.as_str())
        .collect();
    assert!(names.contains(&"main.rs"));
}
