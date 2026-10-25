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
    let cwd = &AbsolutePath::rooted("/tmp");
    let langs = LanguageRegistry::new();
    let ctx = CompletionCtx {
        registry: &reg,
        buffers: &store,
        cwd,
        languages: &langs,
        dirs: &hume_platform::dirs::Dirs::none(),
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
    let names: Vec<&str> = candidates.iter().map(|c| c.label.as_ref()).collect();
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
    let cwd = &AbsolutePath::rooted("/tmp");
    let langs = LanguageRegistry::new();
    let ctx = CompletionCtx {
        registry: &reg,
        buffers: &store,
        cwd,
        languages: &langs,
        dirs: &hume_platform::dirs::Dirs::none(),
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
    let names: Vec<&str> = candidates.iter().map(|c| c.label.as_ref()).collect();
    assert!(names.contains(&"main.rs"));
}

#[test]
fn path_completer_dirs_only_mode() {
    let dir = tempfile::tempdir().unwrap();
    let subdir = dir.path().join("mysubdir");
    let file = dir.path().join("myfile.txt");
    std::fs::create_dir(&subdir).unwrap();
    std::fs::write(&file, "x\n").unwrap();

    let canonical = AbsolutePath::canonicalize(dir.path()).unwrap();
    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let langs = LanguageRegistry::new();
    let ctx = CompletionCtx {
        registry: &reg,
        buffers: &store,
        cwd: &canonical,
        languages: &langs,
        dirs: &hume_platform::dirs::Dirs::none(),
    };

    // dirs_only: files must be excluded.
    let (_, dirs) = complete_path_dirs_only("cd m", 4, &ctx);
    let dir_names: Vec<&str> = dirs.iter().map(|c| c.label.as_ref()).collect();
    assert!(
        dir_names.contains(&"mysubdir/"),
        "dirs_only must include subdirectory"
    );
    assert!(
        !dir_names.contains(&"myfile.txt"),
        "dirs_only must exclude files"
    );

    // Plain complete_path: both dirs and files must appear.
    let (_, all) = complete_path("e m", 3, &ctx);
    let all_names: Vec<&str> = all.iter().map(|c| c.label.as_ref()).collect();
    assert!(
        all_names.contains(&"mysubdir/"),
        "complete_path must include subdirectory"
    );
    assert!(
        all_names.contains(&"myfile.txt"),
        "complete_path must include files"
    );
}

/// A `..` after a symlink completes the directory `:e` would open from:
/// read lexically, not the symlink target's parent.
#[test]
fn path_completer_resolves_dot_dot_after_a_symlink_lexically() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("a")).unwrap();
    std::fs::create_dir_all(dir.path().join("b/c")).unwrap();
    std::fs::write(dir.path().join("a/in_a.txt"), b"").unwrap();
    std::fs::write(dir.path().join("b/in_b.txt"), b"").unwrap();
    std::os::unix::fs::symlink(dir.path().join("b/c"), dir.path().join("a/link")).unwrap();

    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let cwd = AbsolutePath::rooted(dir.path());
    let ctx = ctx(&reg, &store, &cwd);
    let input = "e a/link/../";
    let (_, candidates) = complete_path(input, input.len(), &ctx);

    let names: Vec<&str> = candidates.iter().map(|c| c.label.as_ref()).collect();
    assert!(names.contains(&"in_a.txt"), "{names:?}");
    assert!(!names.contains(&"in_b.txt"), "{names:?}");
}
