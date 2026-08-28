use super::*;
use hume_engine::pipeline::EngineView;
use hume_engine::theme::Theme;

/// Two guaranteed-distinct `BufferId`s — see the identical helper in
/// `lsp/diagnostics.rs` for why a single `EngineView` is required.
fn make_two_bids() -> (BufferId, BufferId) {
    let mut ev = EngineView::new(Theme::default());
    let a = ev.buffers.insert(());
    let b = ev.buffers.insert(());
    (a, b)
}

/// None of these tests assert on a sign's resolved scope — only on its
/// text/position/source — so a bare `ScopeId` (no registry needed) stands
/// in for whatever `host_impl.rs` would have interned.
fn sign(pos: usize, text: &str) -> SignEntry {
    SignEntry {
        pos,
        text: text.to_string(),
        scope: ScopeId(0),
    }
}

#[test]
fn signs_for_buffer_does_not_leak_another_buffers_entries() {
    // Regression test for the by-BufferId restructure: a reader keyed by
    // (source, BufferId) that regressed to scanning every buffer would
    // still pass a single-buffer test, so this exercises two buffers
    // under the *same* source name and asserts isolation.
    let mut store = DecorationStores::default();
    let (a, b) = make_two_bids();
    store.set_signs("linter".to_string(), a, vec![sign(0, "a-sign")]);
    store.set_signs(
        "linter".to_string(),
        b,
        vec![sign(0, "b-sign-1"), sign(1, "b-sign-2")],
    );

    let a_signs: Vec<&str> = store
        .signs_for_buffer(a)
        .map(|(_, e)| e.text.as_str())
        .collect();
    assert_eq!(
        a_signs,
        vec!["a-sign"],
        "only buffer a's signs must be returned for a"
    );

    let b_signs: Vec<&str> = store
        .signs_for_buffer(b)
        .map(|(_, e)| e.text.as_str())
        .collect();
    assert_eq!(
        b_signs,
        vec!["b-sign-1", "b-sign-2"],
        "only buffer b's signs must be returned for b"
    );
}

/// `SourceStore::set` keeps `by_buffer`'s per-buffer source list sorted
/// ascending by name, so `for_buffer` (here via `signs_for_buffer`, the one
/// reader that keeps the source name) yields a deterministic cross-source
/// order — independent of which source called `set-*!` first. Without the
/// sort in `set` (e.g. reverting to plain find-or-push), setting `"zzz"`
/// before `"aaa"` would leave `"zzz"` first in iteration order and this
/// assertion would fail.
#[test]
fn sources_iterate_in_ascending_name_order_regardless_of_set_order() {
    let mut store = DecorationStores::default();
    let (a, _b) = make_two_bids();
    store.set_signs("zzz".to_string(), a, vec![sign(0, "z")]);
    store.set_signs("mmm".to_string(), a, vec![sign(0, "m")]);
    store.set_signs("aaa".to_string(), a, vec![sign(0, "a")]);

    let sources: Vec<&str> = store.signs_for_buffer(a).map(|(s, _)| s).collect();
    assert_eq!(
        sources,
        vec!["aaa", "mmm", "zzz"],
        "sources must iterate ascending by name regardless of set() call order"
    );

    // Re-setting an existing source (wholesale replace) must not move it out
    // of sorted position.
    store.set_signs("mmm".to_string(), a, vec![sign(1, "m2")]);
    let sources: Vec<&str> = store.signs_for_buffer(a).map(|(s, _)| s).collect();
    assert_eq!(sources, vec!["aaa", "mmm", "zzz"]);
}

#[test]
fn sign_sources_register_by_priority_desc_then_name_asc() {
    let mut store = DecorationStores::default();
    store.register_sign_source("b".to_string(), 5);
    store.register_sign_source("a".to_string(), 5);
    store.register_sign_source("c".to_string(), 9);

    assert_eq!(
        store.sign_slot("c"),
        Some(0),
        "highest priority ranks first"
    );
    assert_eq!(
        store.sign_slot("a"),
        Some(1),
        "equal priority — alphabetically first name ranks first"
    );
    assert_eq!(store.sign_slot("b"), Some(2));
    assert_eq!(store.sign_source_count(), 3);
}

#[test]
fn re_registering_a_sign_source_replaces_its_priority_and_reorders_it() {
    let mut store = DecorationStores::default();
    store.register_sign_source("a".to_string(), 1);
    store.register_sign_source("b".to_string(), 2);
    assert_eq!(store.sign_slot("a"), Some(1), "lower priority ranks second");

    store.register_sign_source("a".to_string(), 10);
    assert_eq!(
        store.sign_slot("a"),
        Some(0),
        "re-registering replaces the priority — \"a\" now outranks \"b\""
    );
    assert_eq!(
        store.sign_source_count(),
        2,
        "re-registering must not create a second entry for the same name"
    );
}

#[test]
fn unregistered_sign_source_has_no_slot() {
    let store = DecorationStores::default();
    assert_eq!(store.sign_slot("nope"), None);
    assert_eq!(store.sign_source_count(), 0);
}

fn virtual_line(pos: usize) -> VirtualLineEntry {
    VirtualLineEntry {
        pos,
        text: "x".to_string(),
        before: false,
        scope: ScopeId(0),
        segments: Vec::new(),
    }
}

#[test]
fn remove_buffer_bumps_generation() {
    // A pane's `virtual_lines_synced` cache keys on
    // `(BufferId, generation(BufferId))` (`decoration_providers.rs`).
    // `remove_buffer` mutates the `virtual_lines` store without going
    // through `set_virtual_lines`, so if it didn't also touch `a`'s stamp, a
    // pane reloading the same buffer would see an unchanged cache key and
    // keep mirroring the just-removed (pre-reload) virtual lines forever.
    let mut store = DecorationStores::default();
    let (a, b) = make_two_bids();
    store.set_virtual_lines("git-diff".to_string(), a, vec![virtual_line(0)]);
    let a_generation_after_set = store.generation(a);
    let b_generation_before = store.generation(b);

    store.remove_buffer(a);

    assert_ne!(
        store.generation(a),
        a_generation_after_set,
        "remove_buffer must bump bid's stamp so panes mirroring \
         the cleared buffer resync instead of keeping stale entries"
    );
    assert_eq!(
        store.generation(b),
        b_generation_before,
        "remove_buffer(a) must not touch an unrelated buffer's stamp — the \
         per-buffer generation exists precisely so unrelated buffers don't \
         resync each other's panes"
    );
    assert!(
        store.virtual_lines_for("git-diff", a).is_empty(),
        "remove_buffer must still clear the entries themselves"
    );
}

/// Post-ship correction to the original dirty-tracking design:
/// `remap_through` used to bump the (then
/// store-wide) generation unconditionally, on every queued edit in *any*
/// LSP-attached buffer — including one with zero decorations, which
/// `record_lsp_edits` (`doc_ops.rs`) still queues, since it gates on
/// `lsp_server.is_some() || has_any(bid)`. With a per-buffer stamp, the same
/// unconditional bump would just narrow the blast radius from "every pane on
/// every buffer" to "every pane on this one buffer" — still wrong for a
/// buffer with nothing to invalidate. `remap_through` must only touch a
/// buffer's stamp when a kind actually had an entry to remap.
#[test]
fn remap_through_only_touches_a_buffer_that_has_decorations() {
    use hume_editing::changeset::ChangeSetBuilder;

    let mut store = DecorationStores::default();
    let (a, b) = make_two_bids(); // a: has a sign; b: has nothing at all.
    store.set_signs("linter".to_string(), a, vec![sign(0, "x")]);
    let a_generation_after_set = store.generation(a);
    let b_generation_before = store.generation(b);

    // An identity changeset — its content doesn't matter to this test, only
    // that `remap_through` is called with *something* to remap through.
    let cs = {
        let mut csb = ChangeSetBuilder::new(5);
        csb.retain_rest();
        csb.finish()
    };

    store.remap_through(a, &cs);
    store.remap_through(b, &cs);

    assert_ne!(
        store.generation(a),
        a_generation_after_set,
        "remap_through must touch a buffer that has an entry to remap"
    );
    assert_eq!(
        store.generation(b),
        b_generation_before,
        "remap_through must not touch a buffer with nothing to remap — this \
         is the fix for the keystroke-storm bug: typing in an LSP-attached \
         but undecorated buffer must not invalidate every pane's \
         virtual-lines resync cache"
    );
}
