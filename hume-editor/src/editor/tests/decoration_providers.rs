// Regression tests for the "no render entry, no work" contract
// `hume-editor/src/editor/tests/mod.rs`'s `for_testing` documents in
// source: a pane built via bare `Pane::new` (not `build_pane`) has no
// `ScopedHighlighter`/`SignSource`/etc. providers to feed, so the write
// sides in `decoration_providers.rs` must skip a handle-less pane's
// per-pane computation entirely, not just the final write.

use super::*;

/// `update_highlight_providers`'s search-match refresh must not run for a
/// handle-less pane: `super::search::ops::update_buffer_matches` mutates
/// `Buffer.search_matches` regardless of whether anything can read the
/// result, so a stale cache left there deliberately (to simulate the
/// revision drifting since the last real sync) must stay stale.
#[test]
fn highlight_bridge_skips_search_cache_refresh_for_handleless_pane() {
    let mut ed = editor_from("-[h]>ello world\n");
    assert!(
        ed.state.panes.render.is_empty(),
        "sanity: for_testing builds panes with no render entry"
    );

    let bid = ed.focused_buffer_id();
    ed = ed.with_search_regex("world");
    let stale_cache = ed.state.buffers.get(bid).search_matches.cache.clone();
    assert!(
        stale_cache.is_some(),
        "sanity: with_search_regex populates the cache"
    );

    // Bump the buffer's revision without going through `sync_search_cache` —
    // `handle_key` alone, unlike `feed_key`/`step`, never refreshes the
    // search cache — so the cache above is now stale relative to the live
    // revision, the same drift `update_buffer_matches`'s doc says a
    // non-focused pane's buffer can carry.
    ed.handle_key(key('i'));
    ed.handle_key(key('!'));
    ed.handle_key(key_esc());
    assert_ne!(
        ed.state.buffers.get(bid).revision_id(),
        stale_cache.as_ref().unwrap().0,
        "sanity: the edit bumped the revision"
    );

    let active = ed.view.active_pane_ids();
    let panes = ed.decorated_panes(&active);
    ed.update_highlight_providers(&panes);

    assert_eq!(
        ed.state.buffers.get(bid).search_matches.cache,
        stale_cache,
        "no render entry means no handle to feed — the cache refresh must be \
         skipped, not just its write"
    );
}

/// `update_virtual_line_providers`'s sync stamp must not be recorded for a
/// handle-less pane: the stamp means "this pane's provider holds data for
/// this generation," and writing it without a handle to write the data
/// *into* would suppress the real sync once a handle is added later
/// without an accompanying generation bump.
#[test]
fn virtual_line_bridge_skips_sync_stamp_for_handleless_pane() {
    let mut ed = editor_from("-[l]>ine one\nline two\n");
    assert!(ed.state.panes.render.is_empty());

    let bid = ed.focused_buffer_id();
    let pid = ed.state.focused_pane_id;
    let scope = ed.view.registry.intern("ui.virtual");
    ed.state.config.decorations.set_virtual_lines(
        "test-source".to_string(),
        bid,
        vec![hume_decorations::VirtualLineEntry {
            pos: hume_rope::offset::CharOffset::new(0),
            text: "hint".to_string(),
            before: false,
            scope,
            segments: Vec::new(),
        }],
    );

    let active = ed.view.active_pane_ids();
    let panes = ed.decorated_panes(&active);
    ed.update_virtual_line_providers(&panes);

    assert!(
        !ed.virtual_lines_synced.contains_key(&pid),
        "no render entry means no handle written — the sync stamp must not \
         be recorded either"
    );
}
