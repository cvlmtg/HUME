use super::*;
use pretty_assertions::assert_eq;

// ── Builder tests ────────────────────────────────────────────────────────

#[test]
fn builder_simple() {
    let mut b = ChangeSetBuilder::new(10);
    b.retain(3);
    b.delete(2);
    b.insert("xyz");
    b.retain_rest(); // retain remaining 5
    let cs = b.finish();

    assert_eq!(cs.len_before, 10);
    assert_eq!(cs.len_after, 11); // 10 - 2 + 3 = 11
    assert_eq!(
        cs.ops,
        vec![
            Operation::Retain(3),
            Operation::Delete(2),
            Operation::Insert("xyz".into()),
            Operation::Retain(5),
        ]
    );
}

#[test]
fn builder_merges_adjacent_retains() {
    let mut b = ChangeSetBuilder::new(10);
    b.retain(3);
    b.retain(5);
    b.retain_rest();
    let cs = b.finish();

    // 3 + 5 + 2 = 10, all merged into one Retain.
    assert_eq!(cs.ops, vec![Operation::Retain(10)]);
}

#[test]
fn builder_merges_adjacent_deletes() {
    let mut b = ChangeSetBuilder::new(10);
    b.delete(3);
    b.delete(2);
    b.retain_rest();
    let cs = b.finish();

    assert_eq!(cs.ops, vec![Operation::Delete(5), Operation::Retain(5)]);
}

#[test]
fn builder_merges_adjacent_inserts() {
    let mut b = ChangeSetBuilder::new(5);
    b.insert("ab");
    b.insert("cd");
    b.retain_rest();
    let cs = b.finish();

    assert_eq!(
        cs.ops,
        vec![Operation::Insert("abcd".into()), Operation::Retain(5)]
    );
}

#[test]
fn builder_zero_length_noop() {
    let mut b = ChangeSetBuilder::new(5);
    b.retain(0);
    b.delete(0);
    b.insert("");
    b.retain_rest();
    let cs = b.finish();

    // All zero-length ops were dropped; only the final Retain remains.
    assert_eq!(cs.ops, vec![Operation::Retain(5)]);
}

#[test]
fn builder_empty_document() {
    let mut b = ChangeSetBuilder::new(0);
    b.insert("hello");
    let cs = b.finish();

    assert_eq!(cs.len_before, 0);
    assert_eq!(cs.len_after, 5);
    assert_eq!(cs.ops, vec![Operation::Insert("hello".into())]);
}

#[test]
fn builder_delete_then_insert_not_merged() {
    // Delete followed by Insert is a "replace" — they must stay separate
    // so that invert and compose work correctly.
    let mut b = ChangeSetBuilder::new(5);
    b.delete(3);
    b.insert("xyz");
    b.retain_rest();
    let cs = b.finish();

    assert_eq!(
        cs.ops,
        vec![
            Operation::Delete(3),
            Operation::Insert("xyz".into()),
            Operation::Retain(2),
        ]
    );
}

#[test]
fn builder_tracks_positions() {
    let mut b = ChangeSetBuilder::new(10);
    assert_eq!(b.old_pos(), 0);
    assert_eq!(b.new_pos(), 0);

    b.retain(3);
    assert_eq!(b.old_pos(), 3);
    assert_eq!(b.new_pos(), 3);

    b.delete(2);
    assert_eq!(b.old_pos(), 5);
    assert_eq!(b.new_pos(), 3); // didn't advance

    b.insert("xyz");
    assert_eq!(b.old_pos(), 5); // didn't advance
    assert_eq!(b.new_pos(), 6);

    b.retain_rest();
    assert_eq!(b.old_pos(), 10);
    assert_eq!(b.new_pos(), 11);
}

#[test]
#[should_panic(expected = "old_pos (3) != doc_len (10)")]
fn builder_finish_panics_on_unconsumed() {
    let mut b = ChangeSetBuilder::new(10);
    b.retain(3);
    b.finish(); // should panic — 7 chars unconsumed
}

#[test]
fn is_empty_for_identity() {
    let mut b = ChangeSetBuilder::new(5);
    b.retain_rest();
    assert!(b.finish().is_empty());
}

#[test]
fn is_empty_false_for_real_changes() {
    let mut b = ChangeSetBuilder::new(5);
    b.delete(1);
    b.retain_rest();
    assert!(!b.finish().is_empty());
}

// ── apply tests ──────────────────────────────────────────────────────────

#[test]
fn apply_identity() {
    // "hello\n" = 6 chars; identity changeset retains all 6.
    let buf = Text::from("hello");
    let mut b = ChangeSetBuilder::new(6);
    b.retain_rest();
    let cs = b.finish();

    assert_eq!(cs.apply(&buf).unwrap().to_string(), "hello\n");
}

#[test]
fn apply_insert_at_start() {
    // "world\n" = 6 chars; insert "hello " before it.
    let buf = Text::from("world");
    let mut b = ChangeSetBuilder::new(6);
    b.insert("hello ");
    b.retain_rest();
    let cs = b.finish();

    assert_eq!(cs.apply(&buf).unwrap().to_string(), "hello world\n");
}

#[test]
fn apply_insert_at_end() {
    // "hello\n" = 6 chars; insert " world" before the trailing \n.
    let buf = Text::from("hello");
    let mut b = ChangeSetBuilder::new(6);
    b.retain(5); // retain "hello"
    b.insert(" world");
    b.retain_rest(); // retain "\n"
    let cs = b.finish();

    assert_eq!(cs.apply(&buf).unwrap().to_string(), "hello world\n");
}

#[test]
fn apply_insert_in_middle() {
    // "helo\n" = 5 chars; insert "l" at position 3.
    let buf = Text::from("helo");
    let mut b = ChangeSetBuilder::new(5);
    b.retain(3);
    b.insert("l");
    b.retain_rest();
    let cs = b.finish();

    assert_eq!(cs.apply(&buf).unwrap().to_string(), "hello\n");
}

#[test]
fn apply_delete_at_start() {
    // "hello world\n" = 12 chars; delete "hello " (6 chars).
    let buf = Text::from("hello world");
    let mut b = ChangeSetBuilder::new(12);
    b.delete(6); // delete "hello "
    b.retain_rest();
    let cs = b.finish();

    assert_eq!(cs.apply(&buf).unwrap().to_string(), "world\n");
}

#[test]
fn apply_delete_at_end() {
    // "hello world\n" = 12 chars; delete " world" (6 chars at pos 5–10).
    let buf = Text::from("hello world");
    let mut b = ChangeSetBuilder::new(12);
    b.retain(5);
    b.delete(6); // delete " world"
    b.retain_rest(); // retain "\n"
    let cs = b.finish();

    assert_eq!(cs.apply(&buf).unwrap().to_string(), "hello\n");
}

#[test]
fn apply_replace() {
    // "hello world\n" = 12 chars; replace "world" with "rust".
    let buf = Text::from("hello world");
    let mut b = ChangeSetBuilder::new(12);
    b.retain(6);
    b.delete(5); // delete "world"
    b.insert("rust");
    b.retain_rest(); // retain "\n"
    let cs = b.finish();

    assert_eq!(cs.apply(&buf).unwrap().to_string(), "hello rust\n");
}

#[test]
fn apply_multi_edit() {
    // "hello world\n" = 12 chars; two cursors insert "!" at positions 0 and 6.
    let buf = Text::from("hello world");
    let mut b = ChangeSetBuilder::new(12);
    b.insert("!");
    b.retain(6);
    b.insert("!");
    b.retain_rest();
    let cs = b.finish();

    assert_eq!(cs.apply(&buf).unwrap().to_string(), "!hello !world\n");
}

#[test]
fn apply_delete_entire_buffer() {
    // "hello\n" = 6 chars; delete the content "hello" (5 chars), leaving "\n".
    let buf = Text::from("hello");
    let mut b = ChangeSetBuilder::new(6);
    b.delete(5);
    b.retain_rest(); // retain the structural trailing \n
    let cs = b.finish();

    assert_eq!(cs.apply(&buf).unwrap().to_string(), "\n");
}

#[test]
fn apply_empty_buffer_insert() {
    // Text::empty() = "\n" (1 char); insert "x" before the trailing \n.
    let buf = Text::empty();
    let mut b = ChangeSetBuilder::new(1);
    b.insert("x");
    b.retain_rest(); // retain "\n"
    let cs = b.finish();

    assert_eq!(cs.apply(&buf).unwrap().to_string(), "x\n");
}

// ── map_pos tests ────────────────────────────────────────────────────────

#[test]
fn map_pos_inside_retain() {
    // Identity changeset: Retain(5). Every position maps to itself.
    let mut b = ChangeSetBuilder::new(5);
    b.retain_rest();
    let cs = b.finish();

    for i in 0..=5 {
        assert_eq!(cs.map_pos(i, Assoc::Before), i);
        assert_eq!(cs.map_pos(i, Assoc::After), i);
    }
}

#[test]
fn map_pos_after_insert_at_start() {
    // Insert("xx") then Retain(5). "hello" → "xxhello".
    let mut b = ChangeSetBuilder::new(5);
    b.insert("xx");
    b.retain_rest();
    let cs = b.finish();

    // pos=0 is at the insertion point.
    assert_eq!(cs.map_pos(0, Assoc::Before), 0); // before "xx"
    assert_eq!(cs.map_pos(0, Assoc::After), 2); // after "xx"
    // pos=1 → shifted by 2.
    assert_eq!(cs.map_pos(1, Assoc::Before), 3);
    assert_eq!(cs.map_pos(5, Assoc::Before), 7); // EOF
}

#[test]
fn map_pos_inside_deletion() {
    // Retain(2), Delete(3), Retain(5). "hello world" → "heworld" (wait,
    // that's only 10 chars). Let's use "helloworld" (10 chars).
    // Delete chars 2,3,4 ("llo"). Result: "heworld".
    let mut b = ChangeSetBuilder::new(10);
    b.retain(2);
    b.delete(3);
    b.retain_rest();
    let cs = b.finish();

    assert_eq!(cs.map_pos(0, Assoc::Before), 0); // before deletion
    assert_eq!(cs.map_pos(2, Assoc::Before), 2); // at deletion start
    assert_eq!(cs.map_pos(3, Assoc::Before), 2); // inside deletion → collapse
    assert_eq!(cs.map_pos(4, Assoc::Before), 2); // inside deletion → collapse
    assert_eq!(cs.map_pos(5, Assoc::Before), 2); // right after deletion
    assert_eq!(cs.map_pos(6, Assoc::Before), 3); // shifted back by 3
}

#[test]
fn map_pos_at_insert_boundary() {
    // Retain(3), Insert("XX"), Retain(2). "hello" → "helXXlo".
    let mut b = ChangeSetBuilder::new(5);
    b.retain(3);
    b.insert("XX");
    b.retain_rest();
    let cs = b.finish();

    assert_eq!(cs.map_pos(3, Assoc::Before), 3); // before "XX"
    assert_eq!(cs.map_pos(3, Assoc::After), 5); // after "XX"
    assert_eq!(cs.map_pos(4, Assoc::Before), 6); // 'l' shifted by 2
}

#[test]
fn map_pos_replace_pattern() {
    // Delete(3), Insert("XY"), Retain(2). "hello" → "XYlo".
    // This is a replace of "hel" with "XY".
    let mut b = ChangeSetBuilder::new(5);
    b.delete(3);
    b.insert("XY");
    b.retain_rest();
    let cs = b.finish();

    // pos=0: inside deletion → collapses to 0 (before "XY")
    assert_eq!(cs.map_pos(0, Assoc::Before), 0);
    // pos=2: inside deletion → collapses to 0
    assert_eq!(cs.map_pos(2, Assoc::Before), 0);
    // pos=3: just after deletion, at insert point.
    // Delete consumed 3, so old=3 after Delete. Insert at old=3.
    // pos==old → Assoc applies.
    assert_eq!(cs.map_pos(3, Assoc::Before), 0); // before "XY"
    assert_eq!(cs.map_pos(3, Assoc::After), 2); // after "XY"
    // pos=4: in the final Retain. old=3, new=2 after insert.
    // pos < old + 2 → new + (4-3) = 3.
    assert_eq!(cs.map_pos(4, Assoc::Before), 3);
}

#[test]
fn map_pos_eof() {
    // Retain(3), Insert("XX"). "abc" → "abcXX".
    let mut b = ChangeSetBuilder::new(3);
    b.retain_rest();
    b.insert("XX");
    let cs = b.finish();

    // pos=3 (EOF) is at the insertion point.
    assert_eq!(cs.map_pos(3, Assoc::Before), 3);
    assert_eq!(cs.map_pos(3, Assoc::After), 5);
}

// ── invert tests ─────────────────────────────────────────────────────────

#[test]
fn invert_identity() {
    // "hello\n" = 6 chars.
    let buf = Text::from("hello");
    let mut b = ChangeSetBuilder::new(6);
    b.retain_rest();
    let cs = b.finish();
    let inv = cs.invert(&buf);

    assert!(inv.is_empty());
    assert_eq!(inv.len_before, 6);
    assert_eq!(inv.len_after, 6);
}

#[test]
fn invert_insert() {
    // Insert "XX" at start of "hello\n" → "XXhello\n" (8 chars).
    // Inverse should delete 2 chars at start.
    let buf = Text::from("hello");
    let mut b = ChangeSetBuilder::new(6);
    b.insert("XX");
    b.retain_rest();
    let cs = b.finish();
    let inv = cs.invert(&buf);

    assert_eq!(inv.len_before, 8); // "XXhello\n"
    assert_eq!(inv.len_after, 6); // back to "hello\n"
    assert_eq!(inv.ops, vec![Operation::Delete(2), Operation::Retain(6)]);
}

#[test]
fn invert_delete() {
    // Delete first 3 chars of "hello\n" → "lo\n" (3 chars).
    // Inverse should insert "hel" at start.
    let buf = Text::from("hello");
    let mut b = ChangeSetBuilder::new(6);
    b.delete(3);
    b.retain_rest();
    let cs = b.finish();
    let inv = cs.invert(&buf);

    assert_eq!(inv.len_before, 3); // "lo\n"
    assert_eq!(inv.len_after, 6); // back to "hello\n"
    assert_eq!(
        inv.ops,
        vec![Operation::Insert("hel".into()), Operation::Retain(3)]
    );
}

#[test]
fn invert_roundtrip() {
    // "hello world\n" = 12 chars.
    let buf = Text::from("hello world");
    let mut b = ChangeSetBuilder::new(12);
    b.retain(6);
    b.delete(5);
    b.insert("rust");
    b.retain_rest(); // retain "\n"
    let cs = b.finish();

    let inv = cs.invert(&buf);
    let result = cs.apply(&buf).unwrap();
    assert_eq!(result.to_string(), "hello rust\n");

    let restored = inv.apply(&result).unwrap();
    assert_eq!(restored.to_string(), "hello world\n");
}

#[test]
fn invert_replace() {
    // "abcde\n" = 6 chars.
    let buf = Text::from("abcde");
    let mut b = ChangeSetBuilder::new(6);
    b.retain(1);
    b.delete(3); // delete "bcd"
    b.insert("XY");
    b.retain_rest();
    let cs = b.finish();

    let inv = cs.invert(&buf);
    let result = cs.apply(&buf).unwrap();
    assert_eq!(result.to_string(), "aXYe\n");

    let restored = inv.apply(&result).unwrap();
    assert_eq!(restored.to_string(), "abcde\n");
}

#[test]
fn invert_multi_edit() {
    // "hello world\n" = 12 chars; two inserts at different positions.
    let buf = Text::from("hello world");
    let mut b = ChangeSetBuilder::new(12);
    b.insert("!");
    b.retain(6);
    b.insert("!");
    b.retain_rest();
    let cs = b.finish();

    let inv = cs.invert(&buf);
    let result = cs.apply(&buf).unwrap();
    assert_eq!(result.to_string(), "!hello !world\n");

    let restored = inv.apply(&result).unwrap();
    assert_eq!(restored.to_string(), "hello world\n");
}

// ── compose tests ────────────────────────────────────────────────────────

#[test]
fn compose_identity_left() {
    // identity ∘ cs = cs
    let mut id_b = ChangeSetBuilder::new(5);
    id_b.retain_rest();
    let id = id_b.finish();

    let mut cs_b = ChangeSetBuilder::new(5);
    cs_b.retain(2);
    cs_b.insert("X");
    cs_b.retain_rest();
    let cs = cs_b.finish();

    // cs is PartialEq — clone it so we can compare after compose consumes it.
    let composed = id.compose(cs.clone());
    assert_eq!(composed, cs);
    assert_eq!(composed.len_before, 5);
    assert_eq!(composed.len_after, 6);
}

#[test]
fn compose_identity_right() {
    // cs ∘ identity = cs
    let mut cs_b = ChangeSetBuilder::new(5);
    cs_b.retain(2);
    cs_b.insert("X");
    cs_b.retain_rest();
    let cs = cs_b.finish();

    let mut id_b = ChangeSetBuilder::new(6); // len_after of cs
    id_b.retain_rest();
    let id = id_b.finish();

    let composed = cs.clone().compose(id);
    assert_eq!(composed, cs);
}

#[test]
fn compose_two_inserts() {
    // "abc\n" = 4 chars.
    // A: insert "X" at 0 → "Xabc\n" (4→5)
    // B: insert "Y" at 2 in "Xabc\n" → "XaYbc\n" (5→6)
    // Composed: "abc\n" → "XaYbc\n"
    let buf = Text::from("abc");

    let mut a_b = ChangeSetBuilder::new(4);
    a_b.insert("X");
    a_b.retain_rest();
    let a = a_b.finish();

    let mut b_b = ChangeSetBuilder::new(5);
    b_b.retain(2);
    b_b.insert("Y");
    b_b.retain_rest();
    let b = b_b.finish();

    // Step-by-step oracle: apply a then b separately.
    let mid = a.clone().apply(&buf).unwrap();
    let step_by_step = b.clone().apply(&mid).unwrap();
    let composed = a.compose(b);
    let direct = composed.apply(&buf).unwrap();
    assert_eq!(direct.to_string(), step_by_step.to_string());
    assert_eq!(direct.to_string(), "XaYbc\n");
}

#[test]
fn compose_insert_then_delete() {
    // "abc\n" = 4 chars.
    // A: insert "XY" at 0 → "XYabc\n" (4→6)
    // B: delete 2 at 0 in "XYabc\n" → "abc\n" (6→4)
    // Composed: identity on "abc\n"
    let buf = Text::from("abc");

    let mut a_b = ChangeSetBuilder::new(4);
    a_b.insert("XY");
    a_b.retain_rest();
    let a = a_b.finish();

    let mut b_b = ChangeSetBuilder::new(6);
    b_b.delete(2);
    b_b.retain_rest();
    let b = b_b.finish();

    let composed = a.compose(b);
    assert!(composed.is_empty(), "insert then delete should cancel");
    assert_eq!(composed.apply(&buf).unwrap().to_string(), "abc\n");
}

#[test]
fn compose_delete_then_insert() {
    // "hello\n" = 6 chars.
    // A: delete 3 at start → "lo\n" (6→3)
    // B: insert "XY" at 0 in "lo\n" → "XYlo\n" (3→5)
    // Composed: "hello\n" → "XYlo\n"
    let buf = Text::from("hello");

    let mut a_b = ChangeSetBuilder::new(6);
    a_b.delete(3);
    a_b.retain_rest();
    let a = a_b.finish();

    let mut b_b = ChangeSetBuilder::new(3);
    b_b.insert("XY");
    b_b.retain_rest();
    let b = b_b.finish();

    let mid = a.clone().apply(&buf).unwrap();
    let step_by_step = b.clone().apply(&mid).unwrap();
    let composed = a.compose(b);
    let direct = composed.apply(&buf).unwrap();
    assert_eq!(direct.to_string(), step_by_step.to_string());
    assert_eq!(direct.to_string(), "XYlo\n");
}

#[test]
fn compose_complex() {
    // "abcde\n" = 6 chars.
    // A: retain 2, delete 1, insert "XY", retain rest → "abXYde\n" (6→7)
    // B: retain 1, delete 3, retain rest on "abXYde\n"
    //    → delete "bXY" → "ade\n" (7→4)
    // Composed: "abcde\n" → "ade\n"
    let buf = Text::from("abcde");

    let mut a_b = ChangeSetBuilder::new(6);
    a_b.retain(2);
    a_b.delete(1);
    a_b.insert("XY");
    a_b.retain_rest();
    let a = a_b.finish();

    let mut b_b = ChangeSetBuilder::new(7);
    b_b.retain(1);
    b_b.delete(3);
    b_b.retain_rest();
    let b = b_b.finish();

    let mid = a.clone().apply(&buf).unwrap();
    let step_by_step = b.clone().apply(&mid).unwrap();
    let composed = a.compose(b);
    let direct = composed.apply(&buf).unwrap();
    assert_eq!(direct.to_string(), step_by_step.to_string());
    assert_eq!(direct.to_string(), "ade\n");
}

#[test]
fn compose_partial_insert_retain() {
    // "xyz\n" = 4 chars.
    // A: insert "ABCD" at start, retain rest → "ABCDxyz\n" (4→8)
    // B: retain 2, delete 2, retain rest on "ABCDxyz\n"
    //    → "AB" + "xyz\n" = "ABxyz\n" (8→6)
    // Composed: "xyz\n" → "ABxyz\n"
    let buf = Text::from("xyz");

    let mut a_b = ChangeSetBuilder::new(4);
    a_b.insert("ABCD");
    a_b.retain_rest();
    let a = a_b.finish();

    let mut b_b = ChangeSetBuilder::new(8);
    b_b.retain(2);
    b_b.delete(2);
    b_b.retain_rest();
    let b = b_b.finish();

    let mid = a.clone().apply(&buf).unwrap();
    let step_by_step = b.clone().apply(&mid).unwrap();
    let composed = a.compose(b);
    let direct = composed.apply(&buf).unwrap();
    assert_eq!(direct.to_string(), step_by_step.to_string());
    assert_eq!(direct.to_string(), "ABxyz\n");
}

// ── Property-based tests (proptest) ─────────────────────────────────────

use proptest::prelude::*;

/// Generate a random ASCII string of length 0..=max_len.
fn arb_text(max_len: usize) -> impl Strategy<Value = String> {
    proptest::collection::vec(b'a'..=b'z', 0..=max_len)
        .prop_map(|bytes| String::from_utf8(bytes).unwrap())
}

/// Generate a random valid `ChangeSet` for a document of `doc_len` chars.
///
/// Strategy: partition the document's *content* (`doc_len - 1` chars) into
/// segments, each assigned a random operation (retain or delete). Insert
/// random text between segments with some probability. The structural
/// trailing `\n` (last char) is always retained — user-facing changesets
/// must never delete it.
fn arb_changeset(doc_len: usize) -> impl Strategy<Value = ChangeSet> {
    // Only operate on the content chars; the trailing \n is handled
    // separately below. saturating_sub guards the impossible doc_len == 0.
    let content_len = doc_len.saturating_sub(1);
    let max_ops = (content_len + 1).min(8); // keep it bounded
    proptest::collection::vec(
        (
            prop_oneof![Just(0u8), Just(1u8), Just(2u8)], // 0=retain, 1=delete, 2=insert
            1..=5usize,                                   // segment length
            arb_text(4),                                  // text for inserts
        ),
        0..=max_ops,
    )
    .prop_map(move |raw_ops| {
        let mut builder = ChangeSetBuilder::new(doc_len);
        let mut remaining = content_len;

        for (action, len, text) in raw_ops {
            if remaining == 0 {
                // Only inserts are possible once we've consumed all content chars.
                if action == 2 && !text.is_empty() {
                    builder.insert(&text);
                }
                continue;
            }

            let n = len.min(remaining);

            match action {
                0 => {
                    builder.retain(n);
                    remaining -= n;
                }
                1 => {
                    builder.delete(n);
                    remaining -= n;
                }
                2 => {
                    if !text.is_empty() {
                        builder.insert(&text);
                    }
                    // Don't consume old chars for insert.
                }
                _ => unreachable!(),
            }
        }

        // Retain any unconsumed content chars, then always retain the
        // structural trailing \n — user edits must never delete it.
        builder.retain(remaining); // no-op if remaining == 0
        builder.retain(1); // structural \n
        builder.finish()
    })
}

proptest! {
    /// Applying a changeset then its inverse restores the original buffer.
    #[test]
    fn prop_invert_roundtrip(text in arb_text(20)) {
        let buf = Text::from(text.as_str());
        let doc_len = buf.len_chars(); // includes trailing \n
        let original_content = buf.to_string();

        let half = doc_len / 2;
        let mut b = ChangeSetBuilder::new(doc_len);
        b.delete(half);
        b.insert("X");
        b.retain_rest();
        let cs = b.finish();

        // Invert before apply — buf remains valid on error since apply takes &Text.
        let inv = cs.invert(&buf);
        let result = cs.apply(&buf).unwrap();
        let restored = inv.apply(&result).unwrap();
        prop_assert_eq!(restored.to_string(), original_content);
    }

    /// Composing two changesets produces the same result as applying them
    /// sequentially.
    #[test]
    fn prop_compose_equivalence(text in arb_text(20)) {
        let buf = Text::from(text.as_str());
        let doc_len = buf.len_chars(); // includes trailing \n

        // First changeset: delete first quarter, insert "AB".
        let q1 = doc_len / 4;
        let mut b1 = ChangeSetBuilder::new(doc_len);
        b1.delete(q1);
        b1.insert("AB");
        b1.retain_rest();
        let cs1 = b1.finish();

        let mid = cs1.apply(&buf).unwrap();
        let mid_len = mid.len_chars();

        // Second changeset: retain half, insert "CD", retain rest.
        let half = mid_len / 2;
        let mut b2 = ChangeSetBuilder::new(mid_len);
        b2.retain(half);
        b2.insert("CD");
        b2.retain_rest();
        let cs2 = b2.finish();

        let step_by_step = cs2.clone().apply(&mid).unwrap();
        let composed = cs1.compose(cs2);
        let direct = composed.apply(&buf).unwrap();

        prop_assert_eq!(direct.to_string(), step_by_step.to_string());
    }

    /// Applying a random changeset then its inverse always restores the
    /// original buffer.
    #[test]
    fn prop_random_changeset_invert(
        _text in arb_text(30),
        cs in arb_text(30).prop_flat_map(|t| {
            // Use Text::from to get the actual length (includes \n).
            let buf = Text::from(t.as_str());
            let len = buf.len_chars();
            arb_changeset(len).prop_map(move |cs| (t.clone(), cs))
        })
    ) {
        let (text, cs) = cs;
        let buf = Text::from(text.as_str());
        let original_content = buf.to_string();

        // Invert before apply — buf remains valid on error since apply takes &Text.
        let inv = cs.invert(&buf);
        let result = cs.apply(&buf).unwrap();
        let restored = inv.apply(&result).unwrap();
        prop_assert_eq!(restored.to_string(), original_content);
    }

    /// Compose is associative: (a∘b)∘c produces the same result as a∘(b∘c).
    ///
    /// This is a fundamental OT invariant — if it breaks, grouping
    /// keystrokes into undo steps via repeated compose would be order-
    /// dependent.
    #[test]
    fn prop_compose_associativity(
        text in arb_text(20),
    ) {
        let buf = Text::from(text.as_str());
        let doc_len = buf.len_chars(); // includes trailing \n

        // Build three sequential changesets A→B, B→C, C→D.
        let q = doc_len / 4;
        let mut b1 = ChangeSetBuilder::new(doc_len);
        b1.delete(q);
        b1.insert("X");
        b1.retain_rest();
        let a = b1.finish();

        let mid1 = a.clone().apply(&buf).unwrap();
        let mid1_len = mid1.len_chars();

        let h = mid1_len / 2;
        let mut b2 = ChangeSetBuilder::new(mid1_len);
        b2.retain(h);
        b2.insert("YY");
        b2.retain_rest();
        let b = b2.finish();

        let mid2 = b.clone().apply(&mid1).unwrap();
        let mid2_len = mid2.len_chars();

        let t = mid2_len / 3;
        let mut b3 = ChangeSetBuilder::new(mid2_len);
        b3.retain(t);
        b3.delete(1.min(mid2_len - t));
        b3.retain_rest();
        let c = b3.finish();

        // (a∘b)∘c
        let ab = a.clone().compose(b.clone());
        let ab_c = ab.compose(c.clone());

        // a∘(b∘c)
        let bc = b.compose(c);
        let a_bc = a.compose(bc);

        let result_left = ab_c.apply(&buf).unwrap();
        let result_right = a_bc.apply(&buf).unwrap();
        prop_assert_eq!(result_left.to_string(), result_right.to_string());
    }
}

// ── Invariant enforcement tests ───────────────────────────────────────────

#[test]
fn apply_returns_err_if_trailing_newline_deleted() {
    // "hi\n" = 3 chars. Delete all 3 chars including the structural '\n'.
    // This is what a buggy plugin might produce via the raw builder.
    // apply() must return Err and leave the original buffer untouched.
    let buf = Text::from("hi");
    // Construct the changeset directly to bypass the builder's finish()
    // assert (which catches old_pos != doc_len) and reach apply's
    // trailing-newline check.
    let cs = ChangeSet {
        ops: vec![Operation::Delete(3)],
        len_before: 3,
        len_after: 0,
    };
    let err = cs.apply(&buf).unwrap_err();
    assert_eq!(err, ApplyError::TrailingNewlineMissing);
    // Original buffer is untouched — we can still use it.
    assert_eq!(buf.to_string(), "hi\n");
}

#[test]
fn apply_returns_err_on_length_mismatch() {
    // Changeset built for 10 chars, buffer has 3.
    let buf = Text::from("hi");
    let mut b = ChangeSetBuilder::new(10);
    b.retain_rest();
    let cs = b.finish();

    let err = cs.apply(&buf).unwrap_err();
    assert_eq!(
        err,
        ApplyError::LengthMismatch {
            buf_len: 3,
            expected: 10
        }
    );
    // Original buffer is untouched.
    assert_eq!(buf.to_string(), "hi\n");
}
