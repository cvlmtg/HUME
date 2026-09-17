use super::super::testing::*;
use super::*;

// ── complete_command ────────────────────────────────────────────────────
//
// `complete_command` is a `MatchKind::String` source: it returns the whole
// universe of canonical command names, unfiltered — prefix filtering and
// non-ASCII boundary safety are the session's own job
// (`CompletionSession::update_filter`'s `String` branch, backed by the
// boundary-safe `prefix_matches`), exercised end-to-end by the `:` Tab
// integration tests in `editor/tests/completion.rs`. Unlike the old
// `CommandCompleter`, `prefix_matches` does not exclude an exact match —
// harmless in practice (a fully-typed command name completes to itself,
// a byte-identical no-op replace), so it isn't special-cased.

#[test]
fn command_completer_returns_every_canonical_name() {
    let (reg, store, dir) = make_ctx_parts();
    let ctx = ctx(&reg, &store, dir.path());
    let items = complete_command(&ctx);
    assert!(!items.is_empty());
    let names: Vec<&str> = items.iter().map(|c| c.insert_text.as_str()).collect();
    assert!(names.contains(&"quit"));
    assert!(names.contains(&"write"));
}

/// `:` Tab completion offers only typed commands — an editor (key-bindable)
/// command's name must never appear, even though it's a real registered
/// name. See `registry/mod.rs`'s module doc.
#[test]
fn command_completer_excludes_editor_commands() {
    let (reg, store, dir) = make_ctx_parts();
    let ctx = ctx(&reg, &store, dir.path());
    let items = complete_command(&ctx);
    let names: Vec<&str> = items.iter().map(|c| c.insert_text.as_str()).collect();
    for editor_cmd in ["select-next-word", "move-left", "clear-search"] {
        assert!(
            !names.contains(&editor_cmd),
            "{editor_cmd} is an editor command and must not appear in : completion"
        );
    }
}

#[test]
fn command_completer_sorted_ascending_and_deduped() {
    let (reg, store, dir) = make_ctx_parts();
    let ctx = ctx(&reg, &store, dir.path());
    let items = complete_command(&ctx);
    let names: Vec<&str> = items.iter().map(|c| c.insert_text.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(names, sorted, "must be sorted ascending with no duplicates");
}

#[test]
fn command_completer_excludes_aliases() {
    let (reg, store, dir) = make_ctx_parts();
    let ctx = ctx(&reg, &store, dir.path());
    let items = complete_command(&ctx);
    let names: Vec<&str> = items.iter().map(|c| c.insert_text.as_str()).collect();
    assert!(names.contains(&"write"), "canonical 'write' should appear");
    assert!(
        names.contains(&"write-quit"),
        "canonical 'write-quit' should appear"
    );
    // Aliases ("w", "wq") must not surface as their own candidates.
    assert!(
        !names.contains(&"wq"),
        "'wq' alias must not appear in completions"
    );
    assert!(
        !names.contains(&"w"),
        "'w' alias must not appear in completions"
    );
}

// ── complete_buffer_name ───────────────────────────────────────────────
//
// Also a `MatchKind::String` source — same "returns the whole universe"
// contract as `complete_command` above.

#[test]
fn buffer_name_completer_lists_open_buffers_by_basename_full_path_insert_text() {
    let mut ev = ev();
    let (reg, mut store, dir) = make_ctx_parts();
    let id = make_id(&mut ev);
    store.open(id, buf_with_path("/tmp/foo.txt"));
    let ctx = ctx(&reg, &store, dir.path());
    let items = complete_buffer_name(&ctx);
    let item = items
        .iter()
        .find(|c| c.insert_text == "/tmp/foo.txt")
        .expect("foo.txt must appear");
    assert_eq!(item.label, "foo.txt");
}

#[test]
fn buffer_name_completer_scratch_buffer() {
    let mut ev = ev();
    let (reg, mut store, dir) = make_ctx_parts();
    let id = make_id(&mut ev);
    store.open(id, make_buf()); // no path → scratch
    let ctx = ctx(&reg, &store, dir.path());
    let items = complete_buffer_name(&ctx);
    assert!(items.iter().any(|c| c.insert_text == "*scratch*"));
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

    let items = complete_buffer_name(&ctx);
    assert_eq!(items.len(), 3);

    // The two foo.txt entries must have parent-dir suffixes in their label.
    let foo_entries: Vec<&str> = items
        .iter()
        .filter(|c| c.label.contains("foo.txt"))
        .map(|c| c.label.as_str())
        .collect();
    assert_eq!(foo_entries.len(), 2, "both foo.txt entries must appear");
    assert!(
        foo_entries.iter().all(|d| d.contains('(')),
        "duplicate basenames must include a parent-dir suffix: {foo_entries:?}"
    );

    // The unique bar.txt entry must NOT have a suffix.
    let bar_entry = items
        .iter()
        .find(|c| c.label.contains("bar.txt"))
        .expect("bar.txt must appear");
    assert!(
        !bar_entry.label.contains('('),
        "unique basename must not have a suffix: {}",
        bar_entry.label
    );

    // Insert text is always the full path.
    assert!(items.iter().any(|c| c.insert_text == "/a/foo.txt"));
    assert!(items.iter().any(|c| c.insert_text == "/b/foo.txt"));
    assert!(items.iter().any(|c| c.insert_text == "/tmp/bar.txt"));
}
