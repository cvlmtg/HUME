//! Rendering several panes, and quit and close-pane semantics.

use super::*;
use pretty_assertions::assert_eq;

/// After `:vsplit`, `render_into` must draw both panes at their own rects,
/// not just the focused one. Both panes view the same buffer here, so the
/// same content must appear in both halves of the styled-frame snapshot.
#[test]
fn vsplit_renders_content_in_both_halves() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = crate::testing::build_snapshot_theme();
    ed.execute_typed("vsplit", None).unwrap();

    let rect = Rect::new(0, 0, 20, 4);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

/// The other half of the test above: with `insert-cursor-shape=block` the
/// focused pane's primary head is *painted* from `ui.cursor.primary.insert`
/// rather than left bare for the real terminal cursor to occupy. Both panes
/// show a themed head here, where the default `bar` leaves only the unfocused
/// one painted.
#[test]
fn insert_block_shape_paints_the_head_in_both_panes() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = crate::testing::build_snapshot_theme();
    ed.state.settings.insert_cursor_shape = crate::editor::settings::CursorShape::Block;
    ed.execute_typed("vsplit", None).unwrap();

    ed.feed_key(key('i'));
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: entered Insert mode");

    let rect = Rect::new(0, 0, 20, 4);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

/// A theme that gives `ui.window` only a `bg` (the common upstream Helix
/// shape: Helix's own border code leaves an unset fg as whatever the
/// terminal already shows) must not leave the seam glyph's foreground
/// unthemed in HUME: it falls back to `ui.text`'s color, the same base every
/// other undecorated surface (an ordinary content line, a virtual line)
/// already falls back to. See `EngineView::render`'s seam block in
/// `hume-engine/src/pipeline/mod.rs`.
#[test]
fn seam_divider_falls_back_to_ui_text_when_window_has_no_fg() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = hume_engine::theme::loader::parse_theme(
        r##"
        "ui.text" = { fg = "#abcdef" }
        "ui.background" = { bg = "#000000" }
        "ui.window" = { bg = "#123456" }
        "##,
    )
    .expect("inline test theme must parse")
    .theme;
    ed.execute_typed("vsplit", None).unwrap();

    let rect = Rect::new(0, 0, 20, 4);
    let frame = render_to_styled_string(&mut ed, rect);
    assert!(
        frame.contains("fg=#abcdef,bg=#123456"),
        "seam glyph must use ui.text's color when ui.window has no fg of its own, got: {frame}"
    );
}

/// Where a horizontal seam meets a vertical seam, the crossing cell must get
/// a proper junction glyph (`┬`), not whichever straight glyph drew last.
/// `:split` stacks A/B, then `:vsplit` on B splits it into B|C; the seam
/// below A meets the seam between B and C in a T shape.
#[test]
fn split_then_vsplit_renders_t_junction_glyph() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = crate::testing::build_snapshot_theme();
    ed.execute_typed("split", None).unwrap();
    ed.execute_typed("vsplit", None).unwrap();

    let rect = Rect::new(0, 0, 20, 8);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

/// A 2×2 grid of panes (both rows split at the same ratio, so their vertical
/// seams align in the same column) must render a full cross (`┼`) where the
/// horizontal and vertical seams meet, not two overlapping straight lines.
/// Same grid shape as `quit_in_grid_promotes_correct_sibling`.
#[test]
fn grid_of_four_panes_renders_cross_junction_glyph() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = crate::testing::build_snapshot_theme();
    let pid_a = ed.view.focused_pane();

    ed.execute_typed("split", None).unwrap(); // A/B stacked.
    let pid_b = ed.view.focused_pane();

    ed.switch_focused_pane(pid_a);
    ed.execute_typed("vsplit", None).unwrap(); // A/D side by side, top row.

    ed.switch_focused_pane(pid_b);
    ed.execute_typed("vsplit", None).unwrap(); // B/C side by side, bottom row.

    let rect = Rect::new(0, 0, 20, 8);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

/// Entering Insert mode must hide the fake block cursor only in the focused
/// pane (which real terminal bar cursor overlays), not in every pane.
/// `resolve_pane_settings` (frame.rs) forces a block-cursor mode for
/// unfocused panes regardless of the editor's global mode; this locks that
/// per-pane behavior at the render level. `:vsplit` moves focus to the new
/// (right) pane, so the left pane's cursor cell must keep its block style
/// after `i`, while the right pane's cursor cell goes transparent.
#[test]
fn insert_mode_hides_cursor_only_in_focused_pane() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = crate::testing::build_snapshot_theme();
    ed.execute_typed("vsplit", None).unwrap();
    assert_eq!(ed.state.mode(), Mode::Normal, "sanity: starts in Normal");

    ed.feed_key(key('i'));
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: entered Insert mode");

    let rect = Rect::new(0, 0, 20, 4);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

/// A pane created via `open_pane` (the shared core of `:split`/`:vsplit` and the
/// keymap-bound `pane-split`/`pane-vsplit`) must get the same gutter column as
/// the initial pane, not the empty `ProviderSet` `Pane::new` alone would give
/// it. Uses the real `Editor::open` constructor (not the bare-pane `for_testing`
/// harness used elsewhere in this file) so the initial pane reflects actual
/// production setup. It compares the split pane's
/// `gutter_columns().count()` against the pre-existing initial pane's, rather
/// than asserting a hardcoded count that could pass even if both were wrongly
/// empty.
#[test]
fn split_pane_gets_gutter_column() {
    let mut ed = open_headless(None);
    let pid_a = ed.view.focused_pane();
    let initial_gutter_column_count = ed.view.panes[pid_a].providers.gutter_columns().count();
    assert!(
        initial_gutter_column_count > 0,
        "sanity: the initial pane must itself have a gutter column"
    );

    let bid = ed.focused_buffer_id();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    );

    assert_eq!(
        ed.view.panes[pid_b].providers.gutter_columns().count(),
        initial_gutter_column_count,
        "split pane must have the same gutter columns as the initial pane"
    );
}

// ── T5: `:q` pane-awareness + close-pane semantics ──────────────────────────
//
// `close_focused_pane` backs both `:q` (multi-pane branch, exercised here)
// and the keymap-bound `pane-close` (`Ctrl-p c`, exercised in kitty.rs).

/// With multiple panes open, `:q` closes the focused pane instead of the
/// editor, and moves focus to the promoted sibling.
#[test]
fn quit_with_multiple_panes_closes_focused_pane_not_editor() {
    use hume_engine::pipeline::LayoutTree;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();
    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.view.focused_pane();
    assert_ne!(pid_a, pid_b);

    ed.execute_typed("quit", None).unwrap();

    assert!(
        !ed.state.should_quit,
        ":q with panes open must not quit the editor"
    );
    assert_eq!(ed.view.panes.len(), 1, "the focused pane is closed");
    assert_eq!(
        ed.view.focused_pane(),
        pid_a,
        "focus returns to the surviving pane"
    );
    assert!(
        matches!(*ed.view.layout(), LayoutTree::Leaf(id) if id == pid_a),
        "layout collapses back to a single leaf"
    );
}

/// The multi-pane `:q` branch skips the dirty check entirely: closing a pane
/// never loses edits because the buffer stays open in the buffer list. This
/// is a deliberate difference from the single-pane path, which still refuses
/// on unsaved changes (covered by `colon_q_on_dirty_buffer_refuses`).
#[test]
fn quit_with_multiple_panes_ignores_dirty_buffer() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("split", None).unwrap();

    // Dirty the buffer both panes are viewing.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(focused_unsaved(&ed), "sanity: buffer is dirty");

    let result = ed.execute_typed("quit", None);
    assert!(
        result.is_ok(),
        "multi-pane :q must not be blocked by unsaved changes: {result:?}"
    );
    assert_eq!(ed.view.panes.len(), 1);
}

/// `:wq` delegates to `:q` after a successful write, so it must mirror the
/// same pane-aware close: with panes open it writes then closes the focused
/// pane, rather than tearing down the whole editor.
#[test]
fn wq_with_multiple_panes_closes_focused_pane_not_editor() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let pid_a = ed.view.focused_pane();
    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.view.focused_pane();
    assert_ne!(pid_a, pid_b);

    // Dirty the buffer both panes are viewing.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(focused_unsaved(&ed), "sanity: buffer is dirty");

    let expected_content = ed.doc().text().to_string();
    let result = ed.execute_typed("wq", None);

    assert!(
        result.is_ok(),
        ":wq with panes open must write and close the pane: {result:?}"
    );
    assert!(
        !ed.state.should_quit,
        ":wq with panes open must not quit the editor"
    );
    assert_eq!(ed.view.panes.len(), 1, "the focused pane is closed");
    assert_eq!(
        ed.view.focused_pane(),
        pid_a,
        "focus returns to the surviving pane"
    );
    assert!(!focused_unsaved(&ed), "the write must have happened");
    assert_eq!(
        std::fs::read_to_string(&tmp).unwrap(),
        expected_content,
        "the edit must have been written to disk before the pane closed"
    );
}

/// `viewport_debounce`/`last_visible_range`/`virtual_lines_synced` live on
/// `Editor` rather than `EditorState.panes`, so `drop_pane_state` can't clear
/// them directly. `prepare_frame`'s `prune_closed_pane_caches` sweep is the
/// only place that reclaims a closed pane's entries. Without it these three
/// maps grow without bound over an editor session's lifetime.
#[test]
fn closing_a_pane_reclaims_its_entries_from_the_frame_caches() {
    let mut ed = editor_from("-[h]>ello\n");
    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.resize_terminal(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.view.focused_pane();
    ed.resize_terminal(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    assert!(
        ed.last_visible_range.contains_key(&pid_b),
        "sanity: prepare_frame populated pane B's scroll-key cache entry"
    );
    assert!(
        ed.virtual_lines_synced.contains_key(&pid_b),
        "sanity: prepare_frame populated pane B's virtual-line-sync cache entry"
    );
    assert!(
        ed.viewport_debounce.contains_key(&pid_b),
        "sanity: prepare_frame armed pane B's viewport-debounce timer"
    );

    // Closes the focused pane (B) and promotes A back to focus.
    ed.execute_typed("quit", None).unwrap();
    ed.resize_terminal(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    assert!(
        !ed.last_visible_range.contains_key(&pid_b),
        "closed pane's scroll-key entry must be reclaimed"
    );
    assert!(
        !ed.virtual_lines_synced.contains_key(&pid_b),
        "closed pane's virtual-line-sync entry must be reclaimed"
    );
    assert!(
        !ed.viewport_debounce.contains_key(&pid_b),
        "closed pane's viewport-debounce timer must be cancelled and reclaimed"
    );
}

/// `virtual_lines_synced`'s cache key must include the pane's buffer, not
/// just `decorations.generation()`. A generation-only key would keep a pane
/// mirroring its *previous* buffer's virtual lines after a switch, since
/// switching a buffer doesn't bump the generation.
#[test]
fn switching_a_panes_buffer_rebuilds_its_virtual_lines() {
    use hume_decorations::VirtualLineEntry;

    let mut ed = editor_from("-[h]>ello\n");
    let bid_a = ed.focused_buffer_id();
    let pid = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        bid_a,
        hume_engine::pipeline::Direction::Horizontal,
    );
    ed.switch_focused_pane(pid);

    let scope = ed.view.registry.intern("ui.virtual");
    ed.state.config.decorations.set_virtual_lines(
        "test".to_string(),
        bid_a,
        vec![VirtualLineEntry {
            pos: co(0),
            text: "deleted".to_string(),
            before: false,
            scope,
            segments: Vec::new(),
        }],
    );

    ed.resize_terminal(80, 25);
    ed.settle();
    ed.prepare_frame(&mut hume_engine::pipeline::RenderContext::new());

    assert!(
        ed.state.panes.entries[pid]
            .render
            .virtual_lines()
            .contains_key(&hume_rope::line::ContentLine::new(0)),
        "sanity: pane A mirrors buffer A's virtual line at line 0"
    );

    // Switch the same pane to a fresh buffer with no virtual lines set.
    let bid_b = ed.open_buffer(Buffer::scratch());
    ed.switch_to_buffer_with_jump(FocusedPane::current(&ed.view), bid_b);
    ed.resize_terminal(80, 25);
    ed.settle();
    ed.prepare_frame(&mut hume_engine::pipeline::RenderContext::new());

    assert!(
        ed.state.panes.entries[pid]
            .render
            .virtual_lines()
            .is_empty(),
        "after switching to buffer B, the pane must no longer mirror buffer \
         A's virtual lines; a generation-only sync gate would leave line 0 \
         populated with A's stale entry"
    );
}

/// Closing one leaf of a 2×2 grid promotes the correct sibling and leaves the
/// other three panes untouched. The expected post-close
/// shape (`Split{Leaf(A), Split{B, C}}`) is derived from how the grid was
/// built, not by re-running the close logic.
#[test]
fn quit_in_grid_promotes_correct_sibling() {
    use hume_engine::pipeline::LayoutTree;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();

    // A/B stacked.
    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.view.focused_pane();

    // A/D side by side, top row.
    ed.switch_focused_pane(pid_a);
    ed.execute_typed("vsplit", None).unwrap();
    let pid_d = ed.view.focused_pane();

    // B/C side by side, bottom row. Grid is now: top (A, D), bottom (B, C).
    ed.switch_focused_pane(pid_b);
    ed.execute_typed("vsplit", None).unwrap();
    let pid_c = ed.view.focused_pane();

    assert_eq!(ed.view.panes.len(), 4, "sanity: four panes in the grid");

    // Close D (top-right): its sibling A is promoted, collapsing the top row
    // to a single leaf; the bottom row (B, C) is untouched.
    ed.switch_focused_pane(pid_d);
    ed.execute_typed("quit", None).unwrap();

    assert_eq!(ed.view.panes.len(), 3, "one pane closed");
    assert_eq!(
        ed.view.focused_pane(),
        pid_a,
        "A is promoted as D's surviving sibling"
    );
    assert!(!ed.view.panes.contains_key(pid_d), "D was closed");

    match ed.view.layout() {
        LayoutTree::Split { children, .. } => {
            assert!(
                matches!(&children.0, LayoutTree::Leaf(id) if *id == pid_a),
                "top row collapses to A"
            );
            match &children.1 {
                LayoutTree::Split {
                    children: bottom, ..
                } => {
                    let leaves: Vec<_> = [&bottom.0, &bottom.1]
                        .into_iter()
                        .map(|c| match c {
                            LayoutTree::Leaf(id) => *id,
                            other => panic!("expected leaf, got {other:?}"),
                        })
                        .collect();
                    assert!(
                        leaves.contains(&pid_b) && leaves.contains(&pid_c),
                        "bottom row keeps B and C untouched"
                    );
                }
                other => panic!("expected bottom row Split, got {other:?}"),
            }
        }
        other => panic!("expected Split layout, got {other:?}"),
    }
}
