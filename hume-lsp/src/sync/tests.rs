use super::*;
use hume_editing::ChangeSetBuilder;

/// Build a `ChangeSet` over `before` (LF-only) and return its emitted
/// content changes plus the string the oracle should reach.
fn check(before: &str, build: impl FnOnce(&mut ChangeSetBuilder), enc: PositionEncoding) {
    let rope = Rope::from_str(before);
    let mut builder = ChangeSetBuilder::new(rope.len_chars());
    build(&mut builder);
    let cs = builder.finish();

    let events = changeset_to_content_changes(&rope, &cs, enc);

    let expected = cs
        .apply(&hume_editing::Text::from(before))
        .expect("changeset applies cleanly")
        .to_string();
    let mirrored = apply_events_to_string_mirror(before.to_owned(), &events, enc);
    assert_eq!(
        mirrored, expected,
        "oracle mismatch for enc={enc:?}, events={events:?}"
    );
}

#[test]
fn empty_changeset_emits_no_events() {
    let rope = Rope::from_str("hello\n");
    let mut builder = ChangeSetBuilder::new(rope.len_chars());
    builder.retain_rest();
    let cs = builder.finish();

    let events = changeset_to_content_changes(&rope, &cs, PositionEncoding::Utf8);
    assert!(events.is_empty());
}

#[test]
fn single_insert() {
    for enc in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
        check(
            "hello\n",
            |b| {
                b.retain(5);
                b.insert(", world");
                b.retain_rest();
            },
            enc,
        );
    }
}

#[test]
fn single_delete() {
    for enc in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
        check(
            "hello world\n",
            |b| {
                b.retain(5);
                b.delete(6);
                b.retain_rest();
            },
            enc,
        );
    }
}

#[test]
fn replace_is_delete_then_insert() {
    for enc in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
        check(
            "hello world\n",
            |b| {
                b.retain(6);
                b.delete(5);
                b.insert("HUME");
                b.retain_rest();
            },
            enc,
        );
    }
}

#[test]
fn multiple_disjoint_ops_in_one_changeset() {
    for enc in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
        check(
            "aaa\nbbb\nccc\nddd\n",
            |b| {
                b.retain(4); // past "aaa\n"
                b.delete(4); // remove "bbb\n"
                b.retain(4); // past "ccc\n"
                b.insert("XYZ\n");
                b.retain_rest(); // "ddd\n"
            },
            enc,
        );
    }
}

#[test]
fn insert_containing_newline_shifts_line_count() {
    for enc in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
        check(
            "ab\ncd\n",
            |b| {
                b.retain(1); // after 'a'
                b.insert("XY\nZ");
                b.retain_rest();
            },
            enc,
        );
    }
}

#[test]
fn delete_spanning_a_line_boundary() {
    for enc in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
        check(
            "aaa\nbbb\nccc\n",
            |b| {
                b.retain(2); // "aa|a\nbbb\nccc\n"
                b.delete(4); // removes "a\nbb" (spans the line break)
                b.retain_rest();
            },
            enc,
        );
    }
}

#[test]
fn edits_adjacent_to_multi_byte_and_emoji_chars() {
    for enc in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
        check(
            "héllo \u{1F600} world\n",
            |b| {
                b.retain(1); // after 'h', before 'é'
                b.delete(1); // remove 'é'
                b.insert("E");
                b.retain(5); // "lo " + up to just before 😀
                b.delete(1); // remove 😀
                b.insert("!!");
                b.retain_rest();
            },
            enc,
        );
    }
}

// ── wire_version ─────────────────────────────────────────────────────────

#[test]
fn wire_version_passes_through_ordinary_values() {
    assert_eq!(wire_version(0), 0);
    assert_eq!(wire_version(42), 42);
}

#[test]
#[should_panic(expected = "overflowed i32")]
fn wire_version_panics_instead_of_silently_wrapping_past_i32_max() {
    wire_version(i32::MAX as u64 + 1);
}
