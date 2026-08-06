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

fn sign(line: usize, text: &str) -> SignEntry {
    SignEntry {
        line,
        text: text.to_string(),
        scope: "error".to_string(),
        priority: 10,
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

fn virtual_line(line: usize) -> VirtualLineEntry {
    VirtualLineEntry {
        line,
        text: "x".to_string(),
        before: false,
        scope: None,
        segments: Vec::new(),
    }
}

#[test]
fn remove_buffer_bumps_virtual_lines_generation() {
    // A pane's `virtual_lines_synced` cache keys on
    // `(BufferId, virtual_lines_generation())` (`decoration_providers.rs`).
    // `remove_buffer` mutates the `virtual_lines` map without going through
    // `set_virtual_lines`, so if it didn't also bump the generation, a pane
    // reloading the same buffer would see an unchanged cache key and keep
    // mirroring the just-removed (pre-reload) virtual lines forever.
    let mut store = DecorationStores::default();
    let (a, _b) = make_two_bids();
    store.set_virtual_lines("git-diff".to_string(), a, vec![virtual_line(0)]);
    let generation_after_set = store.virtual_lines_generation();

    store.remove_buffer(a);

    assert_ne!(
        store.virtual_lines_generation(),
        generation_after_set,
        "remove_buffer must bump virtual_lines_generation so panes mirroring \
         the cleared buffer resync instead of keeping stale entries"
    );
    assert!(
        store.virtual_lines_for("git-diff", a).is_empty(),
        "remove_buffer must still clear the entries themselves"
    );
}
