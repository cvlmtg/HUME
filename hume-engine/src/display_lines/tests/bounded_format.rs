//! Bounded-formatting tests (`FormatBound` scan-stop behavior).

use super::*;

/// How many graphemes the formatter actually emitted for `line` — an oracle
/// over `format.rs`'s output, read from the store after the map that filled
/// it is gone.
fn stored_graphemes(store: &PaneLineStore, line: ContentLine) -> usize {
    store
        .find(line)
        .map_or(0, |i| store.entry(i).format.graphemes.len())
}

/// Whether `line` has an entry that was never formatted — the state a no-wrap
/// line sits in when only its block shape was ever asked for.
fn is_unformatted(store: &PaneLineStore, line: ContentLine) -> bool {
    store
        .find(line)
        .is_some_and(|i| store.entry(i).format.extent.is_none())
}

// ---------------------------------------------------------------------------
// Bounded formatting
// ---------------------------------------------------------------------------

/// One unwrapped line far longer than any query needs to scan. Pure ASCII, so
/// column == char offset and the expected answers are plain arithmetic.
fn long_unwrapped_line() -> Rope {
    Rope::from_str(&("a".repeat(70_000) + "\n"))
}

#[test]
fn locate_formats_only_as_far_as_the_target_offset() {
    let rope = long_unwrapped_line();
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.locate(co(5)).1,
        dc(5),
        "pure ASCII: column equals char offset"
    );

    // Dropping the map releases its borrow of the store, letting the test
    // read what the formatter actually emitted — an oracle over `format.rs`'s
    // output rather than anything `DisplayLineMap` reports about itself.
    drop(dlm);
    assert_eq!(
        stored_graphemes(&s, ContentLine::new(0)),
        6,
        "the target grapheme and the five before it, not all 70k"
    );
}

#[test]
fn char_at_formats_only_as_far_as_the_target_column() {
    let rope = long_unwrapped_line();
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.char_at(
            DisplayLinePos::new(ContentLine::new(0), 0),
            dc(5),
            DisplayColTarget::Cell
        ),
        co(5)
    );

    drop(dlm);
    assert_eq!(
        stored_graphemes(&s, ContentLine::new(0)),
        7,
        "through the first cell past column 5, not all 70k"
    );
}

#[test]
fn a_wider_offset_on_a_cached_line_reformats() {
    // The narrower scan left everything past its target unformatted, so the
    // cached extent must not be treated as answering the wider one.
    let rope = long_unwrapped_line();
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(dlm.locate(co(3)).1, dc(3));
    assert_eq!(dlm.locate(co(50)).1, dc(50), "the second query must rescan");
}

#[test]
fn a_column_query_after_an_offset_query_reformats() {
    // A byte-bounded scan says nothing about how far the columns reached, so
    // the two bound kinds can never satisfy each other.
    let rope = long_unwrapped_line();
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    let (pos, _) = dlm.locate(co(3));
    assert_eq!(dlm.char_at(pos, dc(40), DisplayColTarget::Cell), co(40));
}

#[test]
fn locate_display_line_answers_without_formatting_in_no_wrap() {
    let rope = long_unwrapped_line();
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(VirtualLineBlock::uniform(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        2,
        "V",
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.locate_display_line(co(5)),
        DisplayLinePos::new(ContentLine::new(0), 2),
        "the line's own display line sits after the two Before display lines above it"
    );

    // The entry exists — `block` built its shape — but carries no format at
    // all, which is exactly the claim: the display line came from the breakdown.
    drop(dlm);
    assert!(
        is_unformatted(&s, ContentLine::new(0)),
        "a no-wrap display line comes from the block breakdown, with no formatting"
    );
}

#[test]
fn locate_display_line_agrees_with_locate_in_both_wrap_modes() {
    // `locate` is the oracle here, which is not circular: the claim *is* that
    // the two agree, and `locate`'s own answers are pinned independently by
    // the `locate_*` tests above.
    //
    // "a\n\nébc\n" covers an empty line, a line with Before display lines above it, a
    // multi-byte grapheme, and the phantom line past the last `\n`.
    let rope = Rope::from_str("a\n\nébc\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(VirtualLineBlock::uniform(
        VirtualLineAnchor::Before(ContentLine::new(1)),
        2,
        "V",
    )));

    for wrap in [WrapMode::None, WrapMode::Soft { width: 2 }] {
        let mut s = PaneLineStore::new();
        let mut dlm = map(&rope, wrap, &providers, &mut s);
        // Inclusive upper bound is defensive — the buffer invariant keeps a
        // cursor at `head < len_chars()`.
        for offset in 0..=rope.len_chars() {
            // `locate_display_line` first, so it has to be right without a previous
            // `locate` having warmed the scratch.
            let pos = dlm.locate_display_line(co(offset));
            let via_locate = dlm.locate(co(offset)).0;
            assert_eq!(pos, via_locate, "{wrap:?}, offset {offset}");
        }
    }
}
