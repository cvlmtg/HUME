//! `:split`/`:vsplit`, pane sizing, scrolling, and the split-too-small guard.

use super::*;
use pretty_assertions::assert_eq;

// ── T2: `:split` / `:vsplit` typed commands ────────────────────────────────────

/// `:split` stacks a new pane below the focused one, viewing the same buffer,
/// and moves focus there. Vim naming is inverted from the engine's `Direction`:
/// stacked panes are `Direction::Vertical` (it divides height).
#[test]
fn split_stacks_pane_on_same_buffer() {
    use hume_engine::pipeline::{Direction, LayoutTree};

    let mut ed = editor_from("-[h]>ello\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.view.focused_pane();

    ed.execute_typed("split", None).unwrap();

    assert_eq!(ed.view.panes.len(), 2, "split creates exactly one new pane");
    let pid_b = ed.view.focused_pane();
    assert_ne!(pid_b, pid_a, "focus moves to the new pane");
    assert_eq!(
        ed.view.panes[pid_b].buffer_id(),
        bid,
        "new pane views the same buffer"
    );

    match ed.view.layout() {
        LayoutTree::Split {
            direction,
            children,
            ..
        } => {
            assert_eq!(*direction, Direction::Vertical, ":split stacks panes");
            let leaves: Vec<_> = [&children.0, &children.1]
                .into_iter()
                .map(|c| match c {
                    LayoutTree::Leaf(id) => *id,
                    other => panic!("expected two leaves, got {other:?}"),
                })
                .collect();
            assert!(
                leaves.contains(&pid_a) && leaves.contains(&pid_b),
                "layout's two leaves are the original and new pane"
            );
        }
        other => panic!("expected Split layout, got {other:?}"),
    }
}

/// `:vsplit` places the new pane side by side with the focused one:
/// `Direction::Horizontal` in the engine (it divides width).
#[test]
fn vsplit_places_pane_side_by_side() {
    use hume_engine::pipeline::{Direction, LayoutTree};

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("vsplit", None).unwrap();

    match ed.view.layout() {
        LayoutTree::Split { direction, .. } => {
            assert_eq!(*direction, Direction::Horizontal, ":vsplit is side-by-side")
        }
        other => panic!("expected Split layout, got {other:?}"),
    }
}

/// End-to-end regression guard: `:split` typed through the real command-mode
/// dispatch path (`type_cmd`, not `execute_typed`'s shortcut) must still
/// move focus to the new pane. `execute_typed`-based tests above skip the
/// minibuffer entirely, so they wouldn't catch a regression specific to
/// dispatch through an open `Command` layer.
#[test]
fn split_via_command_mode_moves_focus() {
    use hume_engine::pipeline::LayoutTree;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();

    type_cmd(&mut ed, ":split");

    let pid_b = ed.view.focused_pane();
    assert_ne!(
        pid_b, pid_a,
        "focus moves to the new pane through the real command-mode path"
    );
    assert!(
        matches!(*ed.view.layout(), LayoutTree::Split { .. }),
        "layout is a Split"
    );
}

// ── T3: Multi-pane render / prepare_frame ──────────────────────────────────────

/// After `:vsplit`, `prepare_frame` must size both panes from the layout tree,
/// not just the focused one. The sibling pane must tile its half of the
/// terminal rather than keep its `Pane::new` default viewport.
#[test]
fn vsplit_sizes_both_panes_from_layout() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();
    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.view.focused_pane();
    assert_ne!(pid_a, pid_b);

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.resize_terminal(100, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx); // 25 rows → 24 usable after statusline

    let wa = ed.pane_size(pid_a).width();
    let wb = ed.pane_size(pid_b).width();
    assert_eq!(
        wa + wb,
        99,
        "vsplit halves plus the 1-column seam must tile the full terminal width"
    );
    assert!(
        wa < 100 && wb < 100,
        "neither pane keeps the full terminal width"
    );
    assert_eq!(ed.pane_size(pid_a).height(), 24);
    assert_eq!(ed.pane_size(pid_b).height(), 24);
}

/// `:split` stacks panes: height is partitioned, width stays full for both.
#[test]
fn split_sizes_both_panes_stacked() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();
    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.view.focused_pane();

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.resize_terminal(80, 41);
    ed.settle();
    ed.prepare_frame(&mut ctx); // 41 rows → 40 usable after statusline

    let ha = ed.pane_size(pid_a).height();
    let hb = ed.pane_size(pid_b).height();
    assert_eq!(
        ha + hb,
        39,
        "split halves plus the 1-row seam must tile the full usable height"
    );
    assert_eq!(ed.pane_size(pid_a).width(), 80);
    assert_eq!(ed.pane_size(pid_b).width(), 80);
}

/// A third `:vsplit` must resize all three panes to equal widths, not just
/// halve the space of whichever pane was split.
#[test]
fn vsplit_twice_sizes_three_panes_equally() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("vsplit", None).unwrap();
    ed.execute_typed("vsplit", None).unwrap();

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.resize_terminal(100, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let pids: Vec<_> = ed
        .view
        .panes
        .every_pane_across_all_tabs()
        .map(|(pid, _)| pid)
        .collect();
    let widths: Vec<u16> = pids.iter().map(|&pid| ed.pane_size(pid).width()).collect();
    assert_eq!(widths.len(), 3);
    let min = *widths.iter().min().unwrap();
    let max = *widths.iter().max().unwrap();
    assert!(
        max - min <= 1,
        "widths {widths:?} not within 1 of each other"
    );
    assert_eq!(
        widths.iter().sum::<u16>() + 2,
        100,
        "three panes plus two 1-column seams must tile the full terminal width"
    );
}

/// The stacked counterpart of [`vsplit_twice_sizes_three_panes_equally`]:
/// a third `:split` must resize all three panes to equal heights.
#[test]
fn split_twice_sizes_three_panes_equally() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("split", None).unwrap();
    ed.execute_typed("split", None).unwrap();

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.resize_terminal(80, 41);
    ed.settle();
    ed.prepare_frame(&mut ctx); // 41 rows → 40 usable after statusline

    let pids: Vec<_> = ed
        .view
        .panes
        .every_pane_across_all_tabs()
        .map(|(pid, _)| pid)
        .collect();
    let heights: Vec<u16> = pids.iter().map(|&pid| ed.pane_size(pid).height()).collect();
    assert_eq!(heights.len(), 3);
    let min = *heights.iter().min().unwrap();
    let max = *heights.iter().max().unwrap();
    assert!(
        max - min <= 1,
        "heights {heights:?} not within 1 of each other"
    );
    assert_eq!(
        heights.iter().sum::<u16>() + 2,
        40,
        "three panes plus two 1-row seams must tile the full usable height"
    );
}

/// An editor whose panes were sized by one frame at 80×25.
fn sized_editor(text: &str) -> Editor {
    let mut ed = editor_from(text);
    ed.resize_terminal(80, 25);
    ed
}

/// A hundred numbered lines with the cursor on the first.
fn hundred_lines() -> String {
    let text: String = (0..100).map(|i| format!("line{i}\n")).collect();
    format!("-[l]>{}", &text[1..])
}

#[test]
fn half_page_after_close_scrolls_by_the_whole_pane() {
    let mut ed = sized_editor(&hundred_lines());
    ed.execute_typed("split", None).unwrap();
    ed.execute_keymap_command("pane-close".into(), None, false);
    assert_eq!(ed.view.panes.len(), 1);
    let pid = ed.view.focused_pane();

    ed.execute_keymap_command("half-page-down".into(), None, false);

    // The lone pane has the 80x25 terminal's 24 rows again.
    assert_eq!(ed.view.panes[pid].viewport.top().line.index(), 12);
}

#[test]
fn half_page_after_rotate_scrolls_by_the_pane_s_new_share() {
    // A column of two panes splits 24 rows into 11 and 12 around the seam,
    // so rotating moves each pane onto the other share.
    let mut ed = sized_editor(&hundred_lines());
    ed.execute_typed("split", None).unwrap();
    let pid = ed.view.focused_pane();
    let before = ed.pane_size(pid).height();
    ed.execute_keymap_command("pane-rotate-forward".into(), None, false);
    let after = ed.pane_size(pid).height();
    assert_ne!(
        before, after,
        "setup: the rotation changes the pane's height"
    );

    ed.execute_keymap_command("half-page-down".into(), None, false);

    assert_eq!(
        ed.view.panes[pid].viewport.top().line.index(),
        usize::from(after / 2)
    );
}

#[test]
fn half_page_after_split_scrolls_by_the_new_pane_height() {
    let mut ed = sized_editor(&hundred_lines());
    ed.execute_typed("split", None).unwrap();
    let pid = ed.view.focused_pane();
    let height = ed
        .screen_layout()
        .focused_rect(ed.view.focused_layout())
        .height;

    ed.execute_keymap_command("half-page-down".into(), None, false);

    assert_eq!(
        ed.view.panes[pid].viewport.top().line.index(),
        usize::from(height / 2)
    );
}

/// Closing the middle pane of three equal-width panes must redistribute its
/// space equally between the survivors, not hand it all to one neighbour.
#[test]
fn close_pane_redistributes_space_equally() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();
    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.view.focused_pane();
    ed.execute_typed("vsplit", None).unwrap();
    let pid_c = ed.view.focused_pane();

    ed.state.focus.set_for_test(&mut ed.view, pid_b);
    ed.execute_typed("quit", None).unwrap();
    assert_eq!(ed.view.panes.len(), 2);

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.resize_terminal(100, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let wa = ed.pane_size(pid_a).width();
    let wc = ed.pane_size(pid_c).width();
    assert!(
        wa.abs_diff(wc) <= 1,
        "widths {wa} vs {wc} not within 1 of each other after close"
    );
}

/// `Ctrl-p r` moves every pane of the row one slot right, the last wrapping
/// to the first, and focus stays on the pane it was on; `Ctrl-p R` undoes it.
#[test]
fn ctrl_p_r_and_shift_r_rotate_the_focused_row() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();
    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.view.focused_pane();
    ed.execute_typed("vsplit", None).unwrap();
    let pid_c = ed.view.focused_pane();
    assert_eq!(ed.view.layout().leaves(), vec![pid_a, pid_b, pid_c]);

    ed.handle_key(key_ctrl('p'));
    ed.handle_key(key('r'));
    assert_eq!(ed.view.layout().leaves(), vec![pid_c, pid_a, pid_b]);
    assert_eq!(ed.view.focused_pane(), pid_c, "focus moves with the pane");

    ed.handle_key(key_ctrl('p'));
    ed.handle_key(key('R'));
    assert_eq!(ed.view.layout().leaves(), vec![pid_a, pid_b, pid_c]);
    assert_eq!(ed.view.focused_pane(), pid_c);
}

#[test]
fn a_count_rotates_the_row_that_many_places() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();
    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.view.focused_pane();
    ed.execute_typed("vsplit", None).unwrap();
    let pid_c = ed.view.focused_pane();
    ed.execute_typed("vsplit", None).unwrap();
    let pid_d = ed.view.focused_pane();
    assert_eq!(ed.view.layout().leaves(), vec![pid_a, pid_b, pid_c, pid_d]);

    ed.handle_key(key('2'));
    ed.handle_key(key_ctrl('p'));
    ed.handle_key(key('r'));

    assert_eq!(ed.view.layout().leaves(), vec![pid_c, pid_d, pid_a, pid_b]);
    assert_eq!(ed.view.focused_pane(), pid_d);
}

#[test]
fn rotate_with_a_single_pane_is_refused() {
    use hume_engine::pipeline::LayoutTree;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();

    ed.handle_key(key_ctrl('p'));
    ed.handle_key(key('r'));

    assert_eq!(*ed.view.layout(), LayoutTree::Leaf(pid_a));
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("no other pane to rotate")
    );
}

// ── T4: Split-too-small guard ────────────────────────────────────────────────

/// `:vsplit` on a pane too narrow to fit two minimum-width panes plus the
/// seam divider refuses with an `Err`, not a degraded split.
#[test]
fn vsplit_too_narrow_is_noop_with_warning() {
    use hume_engine::pipeline::LayoutTree;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.resize_terminal(20, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx); // width 20 < 2*MIN_PANE_WIDTH(10)+1 = 21

    let err = ed
        .execute_typed("vsplit", None)
        .expect_err("a too-narrow pane must refuse the split");
    assert_eq!(err.severity(), Severity::Info);
    assert_eq!(err.message(), "pane too small to split");

    assert_eq!(
        ed.view.focused_pane(),
        pid_a,
        "focus does not move: split was rejected"
    );
    assert!(
        matches!(*ed.view.layout(), LayoutTree::Leaf(_)),
        "layout is unchanged"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("pane too small to split")
    );
}

/// `:split` on a pane too short likewise refuses with an `Err`.
#[test]
fn split_too_short_is_noop_with_warning() {
    use hume_engine::pipeline::LayoutTree;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.resize_terminal(80, 7);
    ed.settle();
    ed.prepare_frame(&mut ctx); // 7 rows -> 6 usable after statusline < 2*MIN_PANE_HEIGHT(3)+1 = 7

    let err = ed
        .execute_typed("split", None)
        .expect_err("a too-short pane must refuse the split");
    assert_eq!(err.severity(), Severity::Info);
    assert_eq!(err.message(), "pane too small to split");

    assert_eq!(
        ed.view.focused_pane(),
        pid_a,
        "focus does not move: split was rejected"
    );
    assert!(
        matches!(*ed.view.layout(), LayoutTree::Leaf(_)),
        "layout is unchanged"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("pane too small to split")
    );
}

/// The guard is a threshold, not a blanket restriction: a pane at exactly
/// the minimum-fitting size still splits.
#[test]
fn vsplit_at_minimum_width_still_splits() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.resize_terminal(21, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx); // exactly 2*MIN_PANE_WIDTH(10)+1

    ed.execute_typed("vsplit", None).unwrap();

    assert_ne!(
        ed.view.focused_pane(),
        pid_a,
        "split succeeds at the threshold"
    );
}

/// Symmetric threshold check for `:split`'s height axis.
#[test]
fn split_at_minimum_height_still_splits() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.resize_terminal(80, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx); // 8 rows -> 7 usable = exactly 2*MIN_PANE_HEIGHT(3)+1

    ed.execute_typed("split", None).unwrap();

    assert_ne!(
        ed.view.focused_pane(),
        pid_a,
        "split succeeds at the threshold"
    );
}

/// `fits_split` must judge the *post-equalize* size a split would produce,
/// not the pre-split rect of whichever pane is being split, since
/// `equalize` resizes every pane sharing the axis, a stack of several
/// equal-height panes can still have room for one more even though halving
/// any single pane's current height would not fit.
///
/// 24 rows -> 23 usable after the statusline. 4 stacked panes (3 splits) at
/// 3 seams: (23-3)/4 ≈ 5 rows each, well above `MIN_PANE_HEIGHT`(3), so a
/// naive "halve this pane's current rect" guard (5 > 2*3 is false) would
/// wrongly reject a 5th pane; the actual post-equalize height once there are
/// 5 panes is (23-4)/5 ≈ 3.8 -> 3, which clears the minimum.
#[test]
fn fifth_pane_split_succeeds_when_post_equalize_size_still_fits() {
    let mut ed = editor_from("-[h]>ello\n");

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.resize_terminal(80, 24);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    for _ in 0..3 {
        ed.execute_typed("split", None).unwrap();
    }
    assert_eq!(ed.view.panes.len(), 4, "setup: 4 stacked panes");

    ed.execute_typed("split", None).unwrap();

    assert_eq!(
        ed.view.panes.len(),
        5,
        "the split producing a 5th pane must succeed: post-equalize height still fits"
    );
    assert_ne!(
        ed.state.status_msg.as_deref(),
        Some("pane too small to split"),
    );
}

/// A zero-height render area (e.g. a terminal reporting height 0 on early
/// startup) must not panic. `EngineView::render` must guard the statusline
/// (and tab bar) provider's synthesized `Rect` on `area.height`. An
/// unguarded `Rect` claiming a row regardless of `area.height` would send
/// the provider's `Buffer::set_string` calls out-of-bounds and panic, unlike
/// the background fill which clamps.
#[test]
fn zero_height_render_does_not_panic() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    let rect = Rect::new(0, 0, 20, 0);
    assert_eq!(render_to_styled_string(&mut ed, rect), "");
}
