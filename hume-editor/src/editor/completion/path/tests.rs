use super::super::testing::*;
use super::*;
use crate::editor::buffer::store::BufferStore;
use crate::editor::registry::CommandRegistry;
use hume_treesitter::registry::LanguageRegistry;

#[test]
fn path_completer_lists_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("alpha.txt"), b"").unwrap();
    std::fs::write(dir.path().join("beta.txt"), b"").unwrap();
    std::fs::create_dir(dir.path().join("gamma")).unwrap();

    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let ctx = ctx(&reg, &store, dir.path());
    let input = "e ";
    let result = PathCompleter { dirs_only: false }.complete(input, input.len(), &ctx);

    let names: Vec<&str> = result
        .candidates
        .iter()
        .map(|c| c.display.as_str())
        .collect();
    assert!(names.contains(&"alpha.txt"), "alpha.txt should appear");
    assert!(names.contains(&"beta.txt"), "beta.txt should appear");
    assert!(names.contains(&"gamma/"), "directory gets trailing /");
    assert_eq!(result.span_start, 2);
}

#[test]
fn path_completer_filters_by_prefix() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("foo.txt"), b"").unwrap();
    std::fs::write(dir.path().join("bar.txt"), b"").unwrap();

    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let ctx = ctx(&reg, &store, dir.path());
    let input = "e foo";
    let result = PathCompleter { dirs_only: false }.complete(input, input.len(), &ctx);

    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].replacement, "foo.txt");
}

#[test]
fn path_completer_excludes_hidden_unless_dot_prefix() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".hidden"), b"").unwrap();
    std::fs::write(dir.path().join("visible"), b"").unwrap();

    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let ctx = ctx(&reg, &store, dir.path());

    // Without dot prefix: hidden excluded.
    let result = PathCompleter { dirs_only: false }.complete("e ", 2, &ctx);
    assert!(!result.candidates.iter().any(|c| c.display.starts_with('.')));
    assert!(result.candidates.iter().any(|c| c.display == "visible"));

    // With dot prefix: hidden included.
    let input = "e .";
    let result = PathCompleter { dirs_only: false }.complete(input, input.len(), &ctx);
    assert!(result.candidates.iter().any(|c| c.display == ".hidden"));
}

#[test]
fn path_completer_multi_segment() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(sub.join("file.rs"), b"").unwrap();

    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let ctx = ctx(&reg, &store, dir.path());

    // Completing "sub/f" — should find "sub/file.rs".
    let input = "e sub/f";
    let result = PathCompleter { dirs_only: false }.complete(input, input.len(), &ctx);
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].replacement, "sub/file.rs");
}

#[test]
fn path_completer_missing_dir_returns_empty() {
    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let cwd = Path::new("/nonexistent/path/that/does/not/exist");
    let langs = LanguageRegistry::new();
    let ctx = CompletionCtx {
        registry: &reg,
        buffers: &store,
        cwd,
        languages: &langs,
    };
    let result = PathCompleter { dirs_only: false }.complete("e foo", 5, &ctx);
    assert!(result.candidates.is_empty());
}

#[test]
fn path_completer_sorted_ascending() {
    let dir = tempfile::tempdir().unwrap();
    for name in &["zz.txt", "aa.txt", "mm.txt"] {
        std::fs::write(dir.path().join(name), b"").unwrap();
    }
    let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
    let ctx = ctx(&reg, &store, dir.path());
    let result = PathCompleter { dirs_only: false }.complete("e ", 2, &ctx);
    let names: Vec<&str> = result
        .candidates
        .iter()
        .map(|c| c.display.as_str())
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted, "results must be sorted alphabetically");
}

#[cfg(unix)]
mod unix;
