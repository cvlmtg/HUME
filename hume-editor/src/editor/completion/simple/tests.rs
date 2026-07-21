use super::super::testing::*;
use super::*;
use crate::editor::registry::CommandRegistry;

// ── CommandCompleter ──────────────────────────────────────────────────────

#[test]
fn command_completer_empty_prefix_returns_all() {
    let (reg, store, dir) = make_ctx_parts();
    let ctx = ctx(&reg, &store, dir.path());
    let result = CommandCompleter.complete("", 0, &ctx);
    // All registered names (canonicals + aliases) minus empty prefix match all.
    assert!(!result.candidates.is_empty());
    assert_eq!(result.span_start, 0);
}

#[test]
fn command_completer_prefix_filters() {
    let (reg, store, dir) = make_ctx_parts();
    let ctx = ctx(&reg, &store, dir.path());
    let result = CommandCompleter.complete("q", 1, &ctx);
    assert!(
        result
            .candidates
            .iter()
            .all(|c| c.replacement.starts_with('q'))
    );
    assert!(result.candidates.iter().any(|c| c.replacement == "quit"));
}

#[test]
fn command_completer_no_match_returns_empty() {
    let (reg, store, dir) = make_ctx_parts();
    let ctx = ctx(&reg, &store, dir.path());
    let result = CommandCompleter.complete("zzz", 3, &ctx);
    assert!(result.candidates.is_empty());
}

#[test]
fn command_completer_sorted_ascending() {
    let (reg, store, dir) = make_ctx_parts();
    let ctx = ctx(&reg, &store, dir.path());
    let result = CommandCompleter.complete("w", 1, &ctx);
    let names: Vec<&str> = result
        .candidates
        .iter()
        .map(|c| c.display.as_str())
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted);
}

#[test]
fn command_completer_alias_and_canonical_both_appear() {
    let (reg, store, dir) = make_ctx_parts();
    let ctx = ctx(&reg, &store, dir.path());
    // Typing "wr" matches "write" (canonical) and "write-quit" (canonical);
    // "w" (alias) is excluded because it doesn't start with "wr".
    // This verifies that both alias forms and canonical forms of other commands
    // starting with the same prefix are surfaced.
    let result = CommandCompleter.complete("wr", 2, &ctx);
    let names: Vec<&str> = result
        .candidates
        .iter()
        .map(|c| c.replacement.as_str())
        .collect();
    assert!(names.contains(&"write"), "canonical 'write' should appear");
    assert!(
        names.contains(&"write-quit"),
        "canonical 'write-quit' should appear"
    );
    // Verify aliases also surface: "wq" is an alias, starts with "w" not "wr".
    let result2 = CommandCompleter.complete("w", 1, &ctx);
    let names2: Vec<&str> = result2
        .candidates
        .iter()
        .map(|c| c.replacement.as_str())
        .collect();
    assert!(
        names2.contains(&"write"),
        "canonical 'write' should appear with prefix 'w'"
    );
    assert!(
        names2.contains(&"wq"),
        "'wq' alias should appear with prefix 'w'"
    );
}

#[test]
fn command_completer_exact_prefix_not_included() {
    // Typing the exact name should not complete to itself.
    let (reg, store, dir) = make_ctx_parts();
    let ctx = ctx(&reg, &store, dir.path());
    let result = CommandCompleter.complete("quit", 4, &ctx);
    assert!(!result.candidates.iter().any(|c| c.replacement == "quit"));
}

#[test]
fn command_completer_non_ascii_name_does_not_panic() {
    // A byte-slice `name[..prefix.len()]` panics when `prefix.len()` lands
    // mid-codepoint in a non-ASCII command/alias name. Here "naïve-cmd"
    // has 'ï' at bytes 2-3; a 1-byte prefix "n" must not panic.
    use std::borrow::Cow;
    let mut reg = CommandRegistry::with_defaults();
    fn noop(
        _ed: &mut crate::editor::Editor,
        _arg: Option<&str>,
        _force: bool,
    ) -> Result<(), crate::editor::error::CommandError> {
        Ok(())
    }
    reg.register_typed(crate::editor::registry::TypedCommand {
        name: Cow::Borrowed("naïve-cmd"),
        doc: Cow::Borrowed(""),
        aliases: &[],
        fun: noop,
    });
    let store = crate::editor::buffer::store::BufferStore::new();
    let dir = tempfile::tempdir().unwrap();
    let ctx = ctx(&reg, &store, dir.path());
    // "n" has byte-length 1; "ï" at bytes 2-3 means name[..1] would panic.
    // Must not panic and must return the non-ASCII command as a candidate.
    let result = CommandCompleter.complete("n", 1, &ctx);
    assert!(
        result
            .candidates
            .iter()
            .any(|c| c.replacement == "naïve-cmd"),
        "non-ASCII command must appear in completions"
    );
}

// ── BufferNameCompleter ───────────────────────────────────────────────────

#[test]
fn buffer_name_completer_matches_basename() {
    let mut ev = ev();
    let (reg, mut store, dir) = make_ctx_parts();
    let id = make_id(&mut ev);
    store.open(id, buf_with_path("/tmp/foo.txt"));
    let ctx = ctx(&reg, &store, dir.path());
    let result = BufferNameCompleter.complete("bd f", 4, &ctx);
    assert_eq!(result.span_start, 3);
    assert!(
        result
            .candidates
            .iter()
            .any(|c| c.replacement == "/tmp/foo.txt")
    );
}

#[test]
fn buffer_name_completer_scratch_buffer() {
    let mut ev = ev();
    let (reg, mut store, dir) = make_ctx_parts();
    let id = make_id(&mut ev);
    store.open(id, make_buf()); // no path → scratch
    let ctx = ctx(&reg, &store, dir.path());
    let result = BufferNameCompleter.complete("bd *", 4, &ctx);
    assert_eq!(result.span_start, 3);
    assert!(
        result
            .candidates
            .iter()
            .any(|c| c.replacement == "*scratch*")
    );
}

#[test]
fn buffer_name_completer_no_match() {
    let mut ev = ev();
    let (reg, mut store, dir) = make_ctx_parts();
    let id = make_id(&mut ev);
    store.open(id, buf_with_path("/tmp/foo.txt"));
    let ctx = ctx(&reg, &store, dir.path());
    let result = BufferNameCompleter.complete("bd z", 4, &ctx);
    assert!(result.candidates.is_empty());
}

#[test]
fn buffer_name_completer_duplicate_basename_adds_parent_suffix() {
    let mut ev = ev();
    let (reg, mut store, dir) = make_ctx_parts();
    let id1 = make_id(&mut ev);
    let id2 = make_id(&mut ev);
    let id3 = make_id(&mut ev);
    store.open(id1, buf_with_path("/a/foo.txt"));
    store.open(id2, buf_with_path("/b/foo.txt"));
    store.open(id3, buf_with_path("/tmp/bar.txt"));
    let ctx = ctx(&reg, &store, dir.path());

    let result = BufferNameCompleter.complete("b ", 2, &ctx);
    // All three buffers should appear (prefix "" matches all).
    assert_eq!(result.candidates.len(), 3);

    // The two foo.txt entries must have parent-dir suffixes in their display.
    let foo_entries: Vec<&str> = result
        .candidates
        .iter()
        .filter(|c| c.display.contains("foo.txt"))
        .map(|c| c.display.as_str())
        .collect();
    assert_eq!(foo_entries.len(), 2, "both foo.txt entries must appear");
    assert!(
        foo_entries.iter().all(|d| d.contains('(')),
        "duplicate basenames must include a parent-dir suffix: {foo_entries:?}"
    );

    // The unique bar.txt entry must NOT have a suffix.
    let bar_entry = result
        .candidates
        .iter()
        .find(|c| c.display.contains("bar.txt"))
        .expect("bar.txt must appear");
    assert!(
        !bar_entry.display.contains('('),
        "unique basename must not have a suffix: {}",
        bar_entry.display
    );

    // Replacements are always the full paths.
    assert!(
        result
            .candidates
            .iter()
            .any(|c| c.replacement == "/a/foo.txt")
    );
    assert!(
        result
            .candidates
            .iter()
            .any(|c| c.replacement == "/b/foo.txt")
    );
    assert!(
        result
            .candidates
            .iter()
            .any(|c| c.replacement == "/tmp/bar.txt")
    );
}
