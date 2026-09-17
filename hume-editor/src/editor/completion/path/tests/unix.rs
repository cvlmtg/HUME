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
    let (_, candidates) = complete_path_with_expand(input, input.len(), &ctx, false, |s: &str| {
        if let Some(tail) = s.strip_prefix('~')
            && (tail.is_empty() || tail.starts_with('/'))
        {
            return Cow::Owned(format!("{}{tail}", home.display()));
        }
        Cow::Borrowed(s)
    });

    // Candidates must be present (the temp home has files).
    assert!(
        !candidates.is_empty(),
        "tilde should resolve to home and list entries"
    );
    // Replacements must keep the literal `~/` prefix, not expand to the absolute path.
    assert!(
        candidates.iter().all(|c| c.insert_text.starts_with("~/")),
        "replacements must preserve the `~/` prefix"
    );
    let names: Vec<&str> = candidates.iter().map(|c| c.label.as_str()).collect();
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
    let (_, candidates) = complete_path_with_expand(input, input.len(), &ctx, false, |s: &str| {
        if let Some(rest) = s.strip_prefix("$MYDIR") {
            Cow::Owned(format!("{expanded}{rest}"))
        } else {
            Cow::Borrowed(s)
        }
    });

    assert!(
        !candidates.is_empty(),
        "$MYDIR should expand and list entries"
    );
    assert!(
        candidates
            .iter()
            .all(|c| c.insert_text.starts_with("$MYDIR/"))
    );
    let names: Vec<&str> = candidates.iter().map(|c| c.label.as_str()).collect();
    assert!(names.contains(&"main.rs"));
}

#[test]
fn path_completer_dirs_only_mode() {
    let dir = tempfile::tempdir().unwrap();
    let subdir = dir.path().join("mysubdir");
    let file = dir.path().join("myfile.txt");
    std::fs::create_dir(&subdir).unwrap();
    std::fs::write(&file, "x\n").unwrap();

    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let langs = LanguageRegistry::new();
    let ctx = CompletionCtx {
        registry: &reg,
        buffers: &store,
        cwd: &canonical,
        languages: &langs,
    };

    // dirs_only — files must be excluded.
    let (_, dirs) = complete_path_dirs_only("cd m", 4, &ctx);
    let dir_names: Vec<&str> = dirs.iter().map(|c| c.label.as_str()).collect();
    assert!(
        dir_names.contains(&"mysubdir/"),
        "dirs_only must include subdirectory"
    );
    assert!(
        !dir_names.contains(&"myfile.txt"),
        "dirs_only must exclude files"
    );

    // Plain complete_path — both dirs and files must appear.
    let (_, all) = complete_path("e m", 3, &ctx);
    let all_names: Vec<&str> = all.iter().map(|c| c.label.as_str()).collect();
    assert!(
        all_names.contains(&"mysubdir/"),
        "complete_path must include subdirectory"
    );
    assert!(
        all_names.contains(&"myfile.txt"),
        "complete_path must include files"
    );
}
