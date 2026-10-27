use super::*;
use hume_editing::changeset::ChangeSetBuilder;
use hume_editing::text::BufferText;
use hume_engine::pipeline::EngineView;
use hume_engine::theme::Theme;
use hume_rope::offset::CharOffset;

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

fn cl(n: usize) -> ContentLine {
    ContentLine::new(n)
}

fn lines(start: usize, end: usize) -> ExclusiveRange<ContentLine> {
    ExclusiveRange::new(cl(start), cl(end))
}

/// Remaps `bid`'s decorations in `store` through the change `build` records
/// against `before`.
fn remap(
    store: &mut DecorationStores,
    bid: BufferId,
    before: &str,
    build: impl FnOnce(&mut ChangeSetBuilder),
) {
    let before = BufferText::from(before);
    let mut b = ChangeSetBuilder::new(co(before.len_chars()));
    build(&mut b);
    let cs = b.finish();
    let after = cs.apply(&before).expect("the change starts from `before`");
    store.remap_through(bid, &TextChange::new(&before, &after, &cs));
}

/// The lines `source`'s signs sit on in `bid`, with their text.
fn sign_lines<'a>(
    store: &'a DecorationStores,
    source: &str,
    bid: BufferId,
) -> Vec<(usize, &'a str)> {
    store
        .signs_for(source, bid)
        .iter()
        .map(|s| (s.line.index(), &*s.text))
        .collect()
}

/// [`sign_lines`], owning each text.
fn owned_sign_lines(store: &DecorationStores, source: &str, bid: BufferId) -> Vec<(usize, String)> {
    sign_lines(store, source, bid)
        .into_iter()
        .map(|(line, text)| (line, text.to_string()))
        .collect()
}

/// Two guaranteed-distinct `BufferId`s. See the identical helper in
/// `lsp/diagnostics.rs` for why a single `EngineView` is required.
fn make_two_bids() -> (BufferId, BufferId) {
    let (mut ev, _) = EngineView::new(Theme::default(), Default::default());
    let a = ev.buffers.insert(());
    let b = ev.buffers.insert(());
    (a, b)
}

/// None of these tests assert on an entry's resolved scope, only on its
/// text/line/source, so a bare `ScopeId` (no registry needed) stands
/// in for whatever `host_impl.rs` would have interned.
fn sign(line: usize, text: &str) -> SignEntry {
    SignEntry {
        line: cl(line),
        text: text.into(),
        scope: ScopeId(0),
    }
}

/// Same as `sign`, for `EolTextEntry`. `DecorationStores` has no `signs_for_buffer` accessor
/// (`SourceStore::for_buffer`'s ordering guarantee only needs one production
/// `*_for_buffer` reader to exercise it), so those two tests go through
/// `eol_text_for_buffer` instead.
fn eol(line: usize, text: &str) -> EolTextEntry {
    EolTextEntry {
        line: cl(line),
        text: text.into(),
        scope: ScopeId(0),
        hide_on_insert_line: false,
    }
}

#[test]
fn eol_text_for_buffer_does_not_leak_another_buffers_entries() {
    // A reader keyed by (source, BufferId) that scanned every buffer would
    // still pass a single-buffer test, so this exercises two buffers
    // under the *same* source name and asserts isolation.
    let mut store = DecorationStores::default();
    let (a, b) = make_two_bids();
    store.set_eol_text("linter".to_string(), a, vec![eol(0, "a-text")]);
    store.set_eol_text(
        "linter".to_string(),
        b,
        vec![eol(0, "b-text-1"), eol(1, "b-text-2")],
    );

    let a_entries: Vec<&str> = store
        .eol_text_for_buffer(a)
        .map(|(_, e)| e.text.as_str())
        .collect();
    assert_eq!(
        a_entries,
        vec!["a-text"],
        "only buffer a's entries must be returned for a"
    );

    let b_entries: Vec<&str> = store
        .eol_text_for_buffer(b)
        .map(|(_, e)| e.text.as_str())
        .collect();
    assert_eq!(
        b_entries,
        vec!["b-text-1", "b-text-2"],
        "only buffer b's entries must be returned for b"
    );
}

/// `SourceStore::set` keeps `by_buffer`'s per-buffer source list sorted
/// ascending by name, so `for_buffer` (here via `eol_text_for_buffer`, one of
/// the readers that keeps the source name) yields a deterministic
/// cross-source order, independent of which source called `set-*!` first.
/// Without the sort in `set` (e.g. reverting to plain find-or-push), setting
/// `"zzz"` before `"aaa"` would leave `"zzz"` first in iteration order and
/// this assertion would fail.
#[test]
fn sources_iterate_in_ascending_name_order_regardless_of_set_order() {
    let mut store = DecorationStores::default();
    let (a, _b) = make_two_bids();
    store.set_eol_text("zzz".to_string(), a, vec![eol(0, "z")]);
    store.set_eol_text("mmm".to_string(), a, vec![eol(0, "m")]);
    store.set_eol_text("aaa".to_string(), a, vec![eol(0, "a")]);

    let sources: Vec<&str> = store.eol_text_for_buffer(a).map(|(s, _)| s).collect();
    assert_eq!(
        sources,
        vec!["aaa", "mmm", "zzz"],
        "sources must iterate ascending by name regardless of set() call order"
    );

    // Re-setting an existing source (wholesale replace) must not move it out
    // of sorted position.
    store.set_eol_text("mmm".to_string(), a, vec![eol(1, "m2")]);
    let sources: Vec<&str> = store.eol_text_for_buffer(a).map(|(s, _)| s).collect();
    assert_eq!(sources, vec!["aaa", "mmm", "zzz"]);
}

#[test]
fn sign_sources_register_by_priority_desc_then_name_asc() {
    let mut store = DecorationStores::default();
    let (a, _b) = make_two_bids();
    store.register_sign_source("b".to_string(), a, 5);
    store.register_sign_source("a".to_string(), a, 5);
    store.register_sign_source("c".to_string(), a, 9);

    assert_eq!(
        store.sign_slot(a, "c"),
        Some(0),
        "highest priority ranks first"
    );
    assert_eq!(
        store.sign_slot(a, "a"),
        Some(1),
        "equal priority: alphabetically first name ranks first"
    );
    assert_eq!(store.sign_slot(a, "b"), Some(2));
    assert_eq!(store.sign_source_count(a), 3);
}

#[test]
fn re_registering_a_sign_source_replaces_its_priority_and_reorders_it() {
    let mut store = DecorationStores::default();
    let (a, _b) = make_two_bids();
    store.register_sign_source("a".to_string(), a, 1);
    store.register_sign_source("b".to_string(), a, 2);
    assert_eq!(
        store.sign_slot(a, "a"),
        Some(1),
        "lower priority ranks second"
    );

    store.register_sign_source("a".to_string(), a, 10);
    assert_eq!(
        store.sign_slot(a, "a"),
        Some(0),
        "re-registering replaces the priority: \"a\" now outranks \"b\""
    );
    assert_eq!(
        store.sign_source_count(a),
        2,
        "re-registering must not create a second entry for the same name"
    );
}

#[test]
fn unregistered_sign_source_has_no_slot() {
    let store = DecorationStores::default();
    let (a, _b) = make_two_bids();
    assert_eq!(store.sign_slot(a, "nope"), None);
    assert_eq!(store.sign_source_count(a), 0);
}

/// A source registered for one buffer never resolves a slot in another.
/// That is the whole point of scoping registration per buffer rather than session-
/// wide: two buffers registering the same name at different priorities must
/// not interfere with each other's ranking.
#[test]
fn sign_source_registration_does_not_cross_buffers() {
    let mut store = DecorationStores::default();
    let (a, b) = make_two_bids();
    store.register_sign_source("linter".to_string(), a, 5);
    store.register_sign_source("vcs".to_string(), a, 9);
    store.register_sign_source("linter".to_string(), b, 1);

    assert_eq!(
        store.sign_slot(a, "linter"),
        Some(1),
        "in a, vcs (priority 9) outranks linter (priority 5)"
    );
    assert_eq!(
        store.sign_slot(b, "linter"),
        Some(0),
        "b's own registration of linter is unaffected by a's"
    );
    assert_eq!(
        store.sign_slot(b, "vcs"),
        None,
        "vcs was never registered for b"
    );
    assert_eq!(store.sign_source_count(a), 2);
    assert_eq!(store.sign_source_count(b), 1);
}

/// `remove_buffer`, for a closed buffer, drops its sign-source registry
/// along with its decoration entries.
#[test]
fn remove_buffer_drops_its_sign_source_registrations() {
    let mut store = DecorationStores::default();
    let (a, b) = make_two_bids();
    store.register_sign_source("linter".to_string(), a, 5);
    store.register_sign_source("vcs".to_string(), b, 1);

    store.remove_buffer(a);

    assert_eq!(
        store.sign_slot(a, "linter"),
        None,
        "a's registration is gone after remove_buffer"
    );
    assert_eq!(store.sign_source_count(a), 0);
    assert_eq!(
        store.sign_slot(b, "vcs"),
        Some(0),
        "an unrelated buffer's registration must survive"
    );

    store.register_sign_source("vcs".to_string(), a, 9);
    assert_eq!(
        store.sign_slot(a, "vcs"),
        Some(0),
        "re-registering after remove_buffer starts a fresh ranking"
    );
}

/// `signs_in_range` resolves each entry's source to its registered slot
/// itself, once per source group, and prunes to the given lines the same
/// way `in_range` does.
#[test]
fn signs_in_range_yields_each_entrys_resolved_slot_filtered_to_the_range() {
    let mut store = DecorationStores::default();
    let (a, _b) = make_two_bids();
    store.register_sign_source("vcs".to_string(), a, 9);
    store.register_sign_source("linter".to_string(), a, 3);
    store.set_signs("vcs".to_string(), a, vec![sign(0, "+"), sign(20, "+2")]);
    store.set_signs("linter".to_string(), a, vec![sign(0, "!")]);

    let mut got: Vec<_> = store
        .signs_in_range(a, lines(0, 10))
        .map(|(slot, e)| (slot, &*e.text))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![(0, "+"), (1, "!")],
        "vcs (priority 9) resolves to slot 0, linter (priority 3) to slot 1, \
         and vcs's out-of-range entry on line 20 must not appear"
    );
}

fn virtual_line(line: usize) -> VirtualLineEntry {
    VirtualLineEntry {
        line: cl(line),
        text: "x".to_string(),
        side: Side::After,
        scope: ScopeId(0),
        segments: Vec::new(),
    }
}

/// `clear_entries`, for a buffer whose text was replaced, drops its entries
/// and keeps its sign-source registry, which holds no position.
#[test]
fn clear_entries_keeps_sign_source_registrations() {
    let mut store = DecorationStores::default();
    let (a, b) = make_two_bids();
    store.register_sign_source("linter".to_string(), a, 5);
    store.register_sign_source("vcs".to_string(), a, 1);
    store.set_signs("linter".to_string(), a, vec![sign(0, "!")]);

    store.clear_entries(a);

    assert_eq!(store.signs_in_range(a, lines(0, 10)).count(), 0);
    assert_eq!(store.sign_slot(a, "linter"), Some(0));
    assert_eq!(store.sign_slot(a, "vcs"), Some(1));
    assert_eq!(store.sign_slot(b, "linter"), None);
}

#[test]
fn clear_entries_bumps_generation() {
    // A pane's `virtual_lines_synced` cache keys on
    // `(BufferId, generation(BufferId))` (`decoration_providers.rs`).
    // `clear_entries` mutates the `virtual_lines` store without going
    // through `set_virtual_lines`, so if it didn't also touch `a`'s stamp, a
    // pane reloading the same buffer would see an unchanged cache key and
    // keep mirroring the cleared (pre-reload) virtual lines.
    let mut store = DecorationStores::default();
    let (a, b) = make_two_bids();
    store.set_virtual_lines("git-diff".to_string(), a, vec![virtual_line(0)]);
    let a_generation_after_set = store.generation(a);
    let b_generation_before = store.generation(b);

    store.clear_entries(a);

    assert_ne!(
        store.generation(a),
        a_generation_after_set,
        "clear_entries must bump bid's stamp so panes mirroring \
         the cleared buffer resync instead of keeping stale entries"
    );
    assert_eq!(
        store.generation(b),
        b_generation_before,
        "clear_entries(a) must not touch an unrelated buffer's stamp: the \
         per-buffer generation exists precisely so unrelated buffers don't \
         resync each other's panes"
    );
    assert!(
        store.virtual_lines_for("git-diff", a).is_empty(),
        "clear_entries must still clear the entries themselves"
    );
}

/// `remap_through` must only touch a buffer's stamp when a kind actually had
/// an entry to remap. `record_lsp_edits` queues edits for any
/// LSP-attached buffer, since it gates on `lsp_server.is_some() ||
/// has_any(bid)`, so a buffer with zero decorations still reaches
/// `remap_through`. Bumping its stamp would resync every pane on that buffer
/// with nothing to invalidate.
#[test]
fn remap_through_only_touches_a_buffer_that_has_decorations() {
    let mut store = DecorationStores::default();
    let (a, b) = make_two_bids(); // a: has a sign; b: has nothing at all.
    store.set_signs("linter".to_string(), a, vec![sign(0, "x")]);
    let a_generation_after_set = store.generation(a);
    let b_generation_before = store.generation(b);

    // An identity change: its content doesn't matter to this test, only
    // that `remap_through` is called with *something* to remap through.
    remap(&mut store, a, "abcd\n", |_| {});
    remap(&mut store, b, "abcd\n", |_| {});

    assert_ne!(
        store.generation(a),
        a_generation_after_set,
        "remap_through must touch a buffer that has an entry to remap"
    );
    assert_eq!(
        store.generation(b),
        b_generation_before,
        "remap_through must not touch a buffer with nothing to remap: this \
         is the fix for the keystroke-storm bug: typing in an LSP-attached \
         but undecorated buffer must not invalidate every pane's \
         virtual-lines resync cache"
    );
}

fn inlay(pos: usize, text: &str, side: Side) -> InlayHintEntry {
    InlayHintEntry {
        pos: co(pos),
        text: text.into(),
        side,
    }
}

/// Deleting a whole line drops what is anchored on it: an inlay hint inside
/// it and its sign. A sign on a later line follows that line up.
#[test]
fn deleting_a_line_drops_its_decorations_and_shifts_the_later_ones() {
    let mut store = DecorationStores::default();
    let (a, _b) = make_two_bids();
    store.set_inlay_hints(
        "lsp-inlay-hints".to_string(),
        a,
        vec![inlay(5, ": i32", Side::After)],
    );
    store.set_signs("linter".to_string(), a, vec![sign(1, "b"), sign(2, "c")]);

    // "foo\nbar\nbaz\n": delete "bar\n".
    remap(&mut store, a, "foo\nbar\nbaz\n", |b| {
        b.retain_to(co(4)).delete_to(co(8));
    });

    assert!(store.inlay_hints_for_buffer(a).next().is_none());
    assert_eq!(sign_lines(&store, "linter", a), vec![(1, "c")]);
}

#[test]
fn deleting_a_lines_first_char_keeps_its_sign() {
    let mut store = DecorationStores::default();
    let (a, _b) = make_two_bids();
    store.set_signs("linter".to_string(), a, vec![sign(1, "!")]);

    // "foo\nbar\n": delete the "b".
    remap(&mut store, a, "foo\nbar\n", |b| {
        b.retain_to(co(4)).delete_to(co(5));
    });

    assert_eq!(sign_lines(&store, "linter", a), vec![(1, "!")]);
}

#[test]
fn removing_a_lines_whole_indent_keeps_its_sign() {
    let mut store = DecorationStores::default();
    let (a, _b) = make_two_bids();
    store.set_signs("linter".to_string(), a, vec![sign(1, "!")]);

    // "foo\n  bar\n": delete the two-space indent.
    remap(&mut store, a, "foo\n  bar\n", |b| {
        b.retain_to(co(4)).delete_to(co(6));
    });

    assert_eq!(sign_lines(&store, "linter", a), vec![(1, "!")]);
}

/// Joining two lines consumes the first one's `\n`: its sign goes, and the
/// second one's sign stays on the joined line.
#[test]
fn joining_two_lines_keeps_only_the_second_lines_sign() {
    let mut store = DecorationStores::default();
    let (a, _b) = make_two_bids();
    store.set_signs("linter".to_string(), a, vec![sign(0, "a"), sign(1, "b")]);

    // "foo\nbar\n": replace the first "\n" with a space.
    remap(&mut store, a, "foo\nbar\n", |b| {
        b.retain_to(co(3)).delete_to(co(4)).insert(" ");
    });

    assert_eq!(sign_lines(&store, "linter", a), vec![(0, "b")]);
}

/// Text inserted at a line's `\n`, here a new line opened below it, stays
/// after the line: its sign keeps its line, and the next line's sign moves
/// down with it.
#[test]
fn opening_a_line_below_keeps_the_sign_on_its_line() {
    let mut store = DecorationStores::default();
    let (a, _b) = make_two_bids();
    store.set_signs("linter".to_string(), a, vec![sign(0, "a"), sign(1, "b")]);

    // "foo\nbar\n": insert "\nnew" at the first "\n".
    remap(&mut store, a, "foo\nbar\n", |b| {
        b.retain_to(co(3)).insert("\nnew");
    });

    assert_eq!(sign_lines(&store, "linter", a), vec![(0, "a"), (2, "b")]);
}

/// A line opened above moves the decorated line's `\n`, so its sign follows
/// the content down rather than staying on the new line.
#[test]
fn opening_a_line_above_moves_the_sign_with_its_line() {
    let mut store = DecorationStores::default();
    let (a, _b) = make_two_bids();
    store.set_signs("linter".to_string(), a, vec![sign(1, "b")]);

    // "foo\nbar\n": insert "\n" at the start of "bar".
    remap(&mut store, a, "foo\nbar\n", |b| {
        b.retain_to(co(4)).insert("\n");
    });

    assert_eq!(sign_lines(&store, "linter", a), vec![(2, "b")]);
}

/// `"l0\nl1\n…"` up to `l{n-1}`: line `k` starts at `3k`, its `\n` is at
/// `3k + 2`.
fn numbered_lines(n: usize) -> String {
    (0..n).map(|k| format!("l{k}\n")).collect()
}

/// A sign on each of `lines`, labelled with its line number.
fn numbered_signs(lines: &[usize]) -> Vec<SignEntry> {
    lines.iter().map(|&k| sign(k, &k.to_string())).collect()
}

/// Remaps signs on `lines` of a `line_count`-line `numbered_lines` text
/// through the change `build` records, returning where each surviving label
/// lands.
fn remap_numbered(
    line_count: usize,
    lines: &[usize],
    build: impl FnOnce(&mut ChangeSetBuilder),
) -> Vec<(usize, String)> {
    let mut store = DecorationStores::default();
    let (a, _b) = make_two_bids();
    store.set_signs("linter".to_string(), a, numbered_signs(lines));
    remap(&mut store, a, &numbered_lines(line_count), build);
    owned_sign_lines(&store, "linter", a)
}

fn labelled(pairs: &[(usize, usize)]) -> Vec<(usize, String)> {
    pairs
        .iter()
        .map(|&(line, label)| (line, label.to_string()))
        .collect()
}

/// Three cursors in one change: a char typed on line 1, a line opened
/// above line 3, and line 5 joined with line 6.
#[test]
fn several_edited_regions_in_one_change_each_move_their_own_lines() {
    let got = remap_numbered(7, &[0, 1, 2, 3, 4, 5, 6], |b| {
        b.retain_to(co(3)).insert("x");
        b.retain_to(co(9)).insert("\n");
        b.retain_to(co(17)).delete_to(co(18));
    });
    assert_eq!(
        got,
        labelled(&[(0, 0), (1, 1), (2, 2), (4, 3), (5, 4), (6, 6)])
    );
}

#[test]
fn newlines_inserted_before_between_and_after_entries_shift_the_later_ones() {
    let got = remap_numbered(7, &[2, 4], |b| {
        b.insert("\n");
        b.retain_to(co(9)).insert("\n");
        b.retain_to(co(18)).insert("\n");
    });
    assert_eq!(got, labelled(&[(3, 2), (6, 4)]));
}

#[test]
fn newlines_deleted_before_between_and_after_entries_shift_the_later_ones() {
    let got = remap_numbered(7, &[2, 4], |b| {
        b.retain_to(co(2)).delete_to(co(3));
        b.retain_to(co(11)).delete_to(co(12));
        b.retain_to(co(17)).delete_to(co(18));
    });
    assert_eq!(got, labelled(&[(1, 2), (2, 4)]));
}

/// Deleting from inside line 1 to inside line 4 consumes the breaks of
/// lines 1 to 3: their signs go, and line 4's lands on the joined line.
#[test]
fn a_deletion_across_several_decorated_lines_keeps_only_the_last_lines_sign() {
    let got = remap_numbered(7, &[0, 1, 2, 3, 4, 5, 6], |b| {
        b.retain_to(co(4)).delete_to(co(13));
    });
    assert_eq!(got, labelled(&[(0, 0), (1, 4), (2, 5), (3, 6)]));
}

/// A named edit and the `(line, label)` pairs of the signs it leaves.
type EditCase = (
    &'static str,
    fn(&mut ChangeSetBuilder),
    &'static [(usize, usize)],
);

#[test]
fn edits_at_the_buffer_end_keep_the_last_lines_sign_unless_its_break_goes() {
    let cases: [EditCase; 5] = [
        (
            "typing at the end of the last line",
            |b| {
                b.retain_to(co(5)).insert("x");
            },
            &[(0, 0), (1, 1)],
        ),
        (
            "opening a line below the last one",
            |b| {
                b.retain_to(co(5)).insert("\nnew");
            },
            &[(0, 0), (1, 1)],
        ),
        (
            "appending after the structural newline",
            |b| {
                b.retain_to(co(6)).insert("l2\n");
            },
            &[(0, 0), (1, 1)],
        ),
        (
            "deleting the last line's content",
            |b| {
                b.retain_to(co(3)).delete_to(co(5));
            },
            &[(0, 0), (1, 1)],
        ),
        (
            "joining the last line up",
            |b| {
                b.retain_to(co(2)).delete_to(co(5));
            },
            &[(0, 1)],
        ),
    ];
    for (case, build, expected) in cases {
        assert_eq!(
            remap_numbered(2, &[0, 1], build),
            labelled(expected),
            "{case}"
        );
    }
}

#[test]
fn remove_source_for_buffer_drops_only_that_pair() {
    let (a, b) = make_two_bids();
    let mut store: SourceStore<&str, SignEntry> = SourceStore::default();
    store.set("x", a, vec![sign(0, "xa")]);
    store.set("y", a, vec![sign(1, "ya")]);
    store.set("x", b, vec![sign(2, "xb")]);

    assert!(store.remove_source_for_buffer(&"x", a));
    assert!(!store.remove_source_for_buffer(&"x", a));

    let texts = |bid| -> Vec<String> {
        store
            .for_buffer(bid)
            .map(|(_, s)| s.text.to_string())
            .collect()
    };
    assert_eq!(texts(a), vec!["ya"]);
    assert_eq!(texts(b), vec!["xb"]);
}

#[test]
fn removing_a_buffers_last_source_forgets_the_buffer() {
    let (a, _) = make_two_bids();
    let mut store: SourceStore<&str, SignEntry> = SourceStore::default();
    store.set("x", a, vec![sign(0, "xa")]);
    assert!(store.remove_source_for_buffer(&"x", a));
    assert_eq!(store.buffers().count(), 0);
}

fn line_bg(line: usize) -> LineBgEntry {
    LineBgEntry {
        line: cl(line),
        scope: ScopeId(0),
    }
}

fn extra(start: usize, end: usize) -> ExtraHighlightEntry {
    ExtraHighlightEntry {
        start: co(start),
        end: co(end),
        scope: ScopeId(0),
    }
}

#[test]
fn eol_text_in_range_keeps_in_range_entries_in_source_order() {
    let mut store = DecorationStores::default();
    let (a, b) = make_two_bids();
    store.set_eol_text(
        "zeta".to_string(),
        a,
        vec![eol(0, "z0"), eol(12, "z12"), eol(4, "z4")],
    );
    store.set_eol_text("alpha".to_string(), a, vec![eol(8, "a8"), eol(10, "a10")]);
    store.set_eol_text("alpha".to_string(), b, vec![eol(4, "b4")]);

    let got: Vec<(&str, &str)> = store
        .eol_text_in_range(a, lines(4, 10))
        .map(|(s, e)| (s, e.text.as_str()))
        .collect();
    assert_eq!(got, vec![("alpha", "a8"), ("zeta", "z4")]);
}

#[test]
fn line_backgrounds_in_range_keeps_in_range_entries_in_source_order() {
    let mut store = DecorationStores::default();
    let (a, b) = make_two_bids();
    store.set_line_backgrounds("zeta".to_string(), a, vec![line_bg(0), line_bg(4)]);
    store.set_line_backgrounds("alpha".to_string(), a, vec![line_bg(8), line_bg(10)]);
    store.set_line_backgrounds("alpha".to_string(), b, vec![line_bg(4)]);

    let got: Vec<(&str, usize)> = store
        .line_backgrounds_in_range(a, lines(4, 10))
        .map(|(s, e)| (s, e.line.index()))
        .collect();
    assert_eq!(got, vec![("alpha", 8), ("zeta", 4)]);
}

#[test]
fn extra_highlights_starting_before_keeps_spans_that_start_before_the_end() {
    let mut store = DecorationStores::default();
    let (a, b) = make_two_bids();
    store.set_extra_highlights("zeta".to_string(), a, vec![extra(25, 27), extra(0, 3)]);
    store.set_extra_highlights("alpha".to_string(), a, vec![extra(2, 30), extra(10, 12)]);
    store.set_extra_highlights("alpha".to_string(), b, vec![extra(0, 1)]);

    let got: Vec<(usize, usize)> = store
        .extra_highlights_starting_before(a, co(10))
        .map(|e| (e.start.index(), e.end.index()))
        .collect();
    assert_eq!(got, vec![(2, 30), (0, 3)]);
}

/// The line-anchored remap rule stated per entry: find the line's `\n`,
/// map it with `Assoc::Before`, drop the entry if a deletion consumed it,
/// else read the line of the mapped offset. `None` for a dropped entry.
fn reference_remap_lines(lines: &[usize], change: &TextChange) -> Vec<Option<usize>> {
    let mut cursor = hume_editing::changeset::PosMapCursor::new(change.changes().ops());
    lines
        .iter()
        .map(|&line| {
            let newline = change.before().lines().newline(cl(line)).offset();
            let mapped = cursor.map_anchor(newline, hume_editing::changeset::Assoc::Before);
            (!mapped.anchor_deleted).then(|| change.after().char_to_line(mapped.pos).index())
        })
        .collect()
}

mod remap_lines_matches_the_per_entry_rule {
    use super::*;
    use proptest::prelude::*;

    /// Texts made of line breaks, ASCII, a precomposed char and a
    /// combining sequence.
    fn arb_text() -> impl Strategy<Value = String> {
        proptest::collection::vec(
            proptest::sample::select(vec!["a", "\n", "é", "e\u{301}"]),
            0..24,
        )
        .prop_map(|pieces| pieces.concat())
    }

    /// One op of a change: retain or delete up to `len` chars, or insert
    /// `text`.
    #[derive(Debug, Clone)]
    enum Step {
        Retain(usize),
        Delete(usize),
        Insert(String),
    }

    fn arb_steps() -> impl Strategy<Value = Vec<Step>> {
        let step = prop_oneof![
            (1..6usize).prop_map(Step::Retain),
            (1..6usize).prop_map(Step::Delete),
            arb_text().prop_map(Step::Insert),
        ];
        proptest::collection::vec(step, 0..8)
    }

    /// Lines in `0..64`, folded into the text's content lines by the test.
    fn arb_lines() -> impl Strategy<Value = Vec<usize>> {
        proptest::collection::vec(0..64usize, 0..12)
    }

    proptest! {
        #[test]
        fn on_random_texts_and_changes(
            text in arb_text(),
            steps in arb_steps(),
            first in arb_lines(),
            second in arb_lines(),
        ) {
            let before = BufferText::from(text.as_str());
            let len = before.len_chars();
            // `terminated` appends a `\n`, as an edit that deletes the
            // structural one has its result terminated.
            let build = |terminated: bool| {
                let mut b = ChangeSetBuilder::new(co(len));
                let mut old = 0;
                for step in &steps {
                    match step {
                        Step::Retain(n) => {
                            old = (old + n).min(len);
                            b.retain_to(co(old));
                        }
                        Step::Delete(n) => {
                            old = (old + n).min(len);
                            b.delete_to(co(old));
                        }
                        Step::Insert(s) => {
                            b.insert(s);
                        }
                    }
                }
                if terminated {
                    b.retain_to(co(len)).insert("\n");
                }
                b.finish()
            };
            let mut cs = build(false);
            let after = match cs.apply(&before) {
                Ok(after) => after,
                Err(_) => {
                    cs = build(true);
                    cs.apply(&before).expect("the result ends with a line break")
                }
            };
            let change = TextChange::new(&before, &after, &cs);

            let count = before.content_line_count().get();
            let fold = |lines: Vec<usize>| -> Vec<usize> {
                let mut lines: Vec<usize> = lines.into_iter().map(|l| l % count).collect();
                lines.sort_unstable();
                lines
            };
            let sources = [("first", fold(first)), ("second", fold(second))];

            let mut store = DecorationStores::default();
            let (a, _b) = make_two_bids();
            for (source, lines) in &sources {
                store.set_signs(source.to_string(), a, numbered_signs(lines));
            }
            store.remap_through(a, &change);

            for (source, lines) in &sources {
                let expected: Vec<(usize, String)> = lines
                    .iter()
                    .zip(reference_remap_lines(lines, &change))
                    .filter_map(|(&label, line)| line.map(|line| (line, label.to_string())))
                    .collect();
                prop_assert_eq!(owned_sign_lines(&store, source, a), expected, "source {}", source);
            }
        }
    }
}
