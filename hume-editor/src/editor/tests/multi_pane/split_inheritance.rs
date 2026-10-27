//! Per-pane wrap mode, geometry guards and per-pane highlight isolation.

use super::*;
use pretty_assertions::assert_eq;

// ── wrap_mode: pane override → buffer override → global ───────────────────────

/// A same-buffer split (`:split` with no path) inherits the source pane's
/// live override, not the global default. This lets a `:wrap`-toggled
/// pane pass its mode on to a split of itself.
#[test]
fn same_buffer_split_inherits_source_panes_wrap_override() {
    let mut ed = editor_from("-[h]>ello\n");
    pin_wrap(&mut ed, hume_engine::pane::WrapMode::Soft { width: 40 });
    // Global default differs, to prove it is NOT the source.
    ed.state.settings.wrap_mode = hume_engine::pane::WrapMode::None;

    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.view.focused_pane();

    assert_eq!(
        ed.view.panes[pid_b].wrap().mode,
        Some(hume_engine::pane::WrapMode::Soft { width: 40 }),
        "same-buffer split inherits the source pane's live override"
    );
}

/// The other half of the split-inheritance contract: a source pane with *no*
/// pane-level override (still inheriting from the buffer/global setting)
/// splits into a pane that is likewise unpinned, not one frozen at
/// whichever mode the source happened to resolve to. The new pane keeps
/// following later `:set buffer`/`:set global wrap-mode=…` changes, same as
/// the pane it split from.
#[test]
fn same_buffer_split_of_an_unpinned_pane_stays_unpinned() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();
    assert_eq!(ed.view.panes[pid_a].wrap().mode, None, "sanity: unpinned");

    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.view.focused_pane();

    assert_eq!(
        ed.view.panes[pid_b].wrap().mode,
        None,
        "split of an unpinned pane is itself unpinned, not frozen at the \
         resolved mode"
    );
}

/// `:wrap` toggles only the focused pane's override; a sibling pane on the
/// same buffer is untouched. The override lives on `Pane`, not on the
/// buffer, so two panes viewing the same buffer can wrap independently once
/// one is pinned.
#[test]
fn wrap_toggle_affects_only_focused_pane() {
    let mut ed = editor_from("-[h]>ello\n");
    // Global is resolved lazily on every read, so this reaches pid_a's
    // effective mode retroactively; no pane pin needed for A to start off.
    ed.state.settings.wrap_mode = hume_engine::pane::WrapMode::None;
    let pid_a = ed.view.focused_pane();

    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.view.focused_pane();
    assert_ne!(pid_a, pid_b);
    assert_eq!(
        ed.view.panes[pid_a].buffer_id(),
        ed.view.panes[pid_b].buffer_id(),
        "sanity: both panes view the same buffer"
    );

    // Focus is on B (the new pane) after :split; toggle wrap there.
    ed.execute_typed("wrap", None).unwrap();

    let doc = ed.state.buffers.get(ed.view.panes[pid_a].buffer_id());
    assert!(
        crate::editor::commands::effective_wrap_mode(
            doc,
            &ed.state.settings,
            &ed.view.panes[pid_b]
        )
        .is_wrapping(),
        "B: wrap toggled on"
    );
    assert!(
        !crate::editor::commands::effective_wrap_mode(
            doc,
            &ed.state.settings,
            &ed.view.panes[pid_a]
        )
        .is_wrapping(),
        "A: unaffected by B's toggle, despite sharing a buffer"
    );
}

// ── Geometry regression guards ─────────────────────────────────────────────────
//
// `EngineView` must not cache a `pane_rects` snapshot across frames. Every
// consumer (pane-focus commands, `fits_split`, the bar-cursor lookup) must
// recompute from the live layout tree plus the terminal area cached by the
// last `prepare_frame`. A cache would let a close/split earlier in the same
// macro-replay batch invalidate geometry that a later command in the same
// batch would otherwise still trust.

/// Closing a pane and then focusing the next one, with no `prepare_frame` in
/// between (exactly what a macro-replay batch does: several commands run
/// per frame), must land on the surviving pane. A cached rect list would
/// still list the just-closed pane, handing focus to a dead `PaneId`.
#[test]
fn close_then_focus_next_without_reframe_lands_on_live_pane() {
    use crate::editor::commands::{FocusedPane, cmd_pane_focus_next};
    use hume_ops::MotionMode;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();
    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.view.focused_pane();
    assert_ne!(pid_a, pid_b);

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.resize_terminal(100, 51);
    ed.settle();
    ed.prepare_frame(&mut ctx); // establish terminal geometry once

    // Close B, then immediately focus-next, no `prepare_frame` in between.
    ed.execute_typed("quit", None).unwrap();
    assert_eq!(ed.view.panes.len(), 1, "sanity: B is closed");
    let fp = FocusedPane::current(&ed.view);
    cmd_pane_focus_next(
        &mut ed.state,
        &mut ed.view,
        fp,
        Count::typed(1),
        MotionMode::Move,
    )
    .unwrap();

    assert_eq!(
        ed.view.focused_pane(),
        pid_a,
        "focus-next after a same-batch close must land on the live survivor"
    );
    assert!(ed.view.panes.contains_key(ed.view.focused_pane()));
}

/// Splitting and then focusing directionally, with no `prepare_frame` in
/// between, must reach the freshly created pane. A cached rect list from
/// before the split would still show only the original pane, so the focus
/// command would silently no-op instead of moving to the new pane.
#[test]
fn split_then_focus_left_without_reframe_reaches_new_pane() {
    use crate::editor::commands::{FocusedPane, cmd_pane_focus_left};
    use hume_ops::MotionMode;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.resize_terminal(100, 51);
    ed.settle();
    ed.prepare_frame(&mut ctx); // geometry established with one pane

    // :vsplit puts the new pane on the right and moves focus to it, with no
    // `prepare_frame` in between.
    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.view.focused_pane();
    assert_ne!(pid_a, pid_b);

    let fp = FocusedPane::current(&ed.view);
    cmd_pane_focus_left(
        &mut ed.state,
        &mut ed.view,
        fp,
        Count::typed(1),
        MotionMode::Move,
    )
    .unwrap();
    assert_eq!(
        ed.view.focused_pane(),
        pid_a,
        "focus-left from the freshly split pane must reach the original pane"
    );
}

/// A bare split (same buffer as the source pane) inherits the source pane's
/// cursor and scroll position, rather than jumping to the top of the file.
#[test]
fn split_inherits_focused_panes_selection_and_scroll() {
    let content: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let text = BufferText::from(content.as_str());
    let sels = sels_at(&text, &[(0, 0)], 0);
    let mut ed = Editor::for_testing_with(test_fixtures::testing::state(text, sels));
    let bid = ed.focused_buffer_id();
    let pid_a = ed.view.focused_pane();

    // Move A's cursor and scroll well away from the top of the file.
    let cursor_pos = ed
        .doc()
        .text()
        .line_to_char(hume_rope::line::RopeyLine::new(150));
    set_cursor(&mut ed, cursor_pos.index());
    ed.view.panes[pid_a].viewport.seed_top_for_test(
        hume_engine::display_lines::DisplayLinePos::new(hume_rope::line::ContentLine::new(140), 0),
    );

    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.view.focused_pane();
    assert_ne!(pid_a, pid_b);

    assert_eq!(
        ed.state.panes.entries.seeded(pid_b, bid).selections(),
        ed.state.panes.entries.seeded(pid_a, bid).selections(),
        "new pane inherits the source pane's selection"
    );
    assert_eq!(
        ed.view.panes[pid_b].viewport.top().line,
        hume_rope::line::ContentLine::new(140),
        "new pane inherits the source pane's scroll position"
    );
}

/// A same-buffer split inherits the source pane's `saved_scrolls`: its
/// memory of where it was in buffers visited *before* the split. Without
/// this, the new pane would reset such a buffer to the top on first visit
/// instead of recalling where the source pane last left it.
#[test]
fn same_buffer_split_inherits_saved_scrolls() {
    use crate::editor::buffer::lifecycle::open_buffer;
    use hume_engine::pane::ScrollPosition;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();

    // A second buffer the source pane visited (and scrolled) before the
    // split, then switched away from. This is what populates
    // `saved_scrolls` in real usage (see `Pane::show_buffer`).
    let bid2 = open_buffer(&mut ed.view, &mut ed.state.buffers, Buffer::scratch(), 0);
    ed.view.panes[pid_a].saved_scrolls.insert(
        bid2,
        ScrollPosition {
            top: hume_engine::display_lines::DisplayLinePos::new(
                hume_rope::line::ContentLine::new(42),
                0,
            ),
            horizontal_offset: hume_rope::column::DisplayLineCol::new(0),
        },
    );

    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.view.focused_pane();
    assert_ne!(pid_a, pid_b);

    assert_eq!(
        ed.view.panes[pid_b].saved_scrolls.get(bid2),
        ed.view.panes[pid_a].saved_scrolls.get(bid2),
        "new pane inherits the source pane's saved_scrolls history"
    );
}

/// Before any frame, a split measures the headless terminal.
#[test]
fn split_before_first_frame_is_refused_once_the_headless_pane_is_too_short() {
    let mut ed = editor_from("-[h]>ello\n");
    let mut splits = 0;
    let err = loop {
        match ed.execute_typed("split", None) {
            Ok(()) => splits += 1,
            Err(err) => break err,
        }
        assert!(splits < 24, "a 24-row pane area cannot hold 24 panes");
    };
    // Panes on one axis share its rows equally: the headless 80x25
    // terminal's 24-row pane area holds six panes and their five seams at 3
    // or 4 rows each, and a seventh pane would leave some with 2, under the
    // 3-row minimum.
    assert_eq!(splits, 5);
    assert_eq!(err.message(), "pane too small to split");
}

/// `pane-dividers=false` reclaims the seam column: sibling panes tile
/// edge-to-edge instead of leaving an unpainted 1-cell gap between them.
#[test]
fn dividers_off_pane_rects_tile_with_no_gap() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.settings.pane_dividers = false;
    ed.execute_typed("vsplit", None).unwrap();

    ed.render_to_buf(Rect::new(0, 0, 100, 51));

    let hits: Vec<_> = (0..100)
        .map(|x| {
            ed.view
                .pane_hit(hume_grid::Position::new(x, 0))
                .expect("no column is reserved for a seam when pane-dividers is off")
                .pane()
        })
        .collect();
    let right_start = hits
        .iter()
        .position(|&pid| pid != hits[0])
        .expect("both panes are drawn on the row");
    assert!(
        hits[right_start..]
            .iter()
            .all(|&pid| pid == hits[right_start]),
        "the right pane starts where the left one ends"
    );
}

/// Visual lock-in companion to `dividers_off_pane_rects_tile_with_no_gap`:
/// with `pane-dividers` off the two halves render edge-to-edge (no gap
/// column between them), and the non-focused pane is still visibly dimmed:
/// dimming is the focus cue, independent of the divider glyph.
#[test]
fn vsplit_dividers_off_tiles_edge_to_edge_and_still_dims() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = crate::testing::build_snapshot_theme();
    ed.state.settings.pane_dividers = false;
    ed.execute_typed("vsplit", None).unwrap();

    let rect = Rect::new(0, 0, 20, 4);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

/// A same-buffer split inherits the source pane's jump history so the new
/// pane can Ctrl-o back to positions visited before the split. The two lists
/// then diverge: a new jump in either pane does not affect the other.
#[test]
fn split_same_buffer_clones_jump_list_then_diverges() {
    let mut ed = jump_editor(10);
    let pid_a = ed.view.focused_pane();

    // `gg` (goto-first-line) is a jump command: records the pre-jump position.
    ed.handle_key(key('g'));
    ed.handle_key(key('g'));
    assert_eq!(
        ed.state.panes.entries[pid_a].jumps.len(),
        1,
        "source pane has one jump entry after gg"
    );

    // Same-buffer split: new pane inherits the source pane's jump history.
    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.view.focused_pane();
    assert_ne!(pid_a, pid_b, "focus moved to the new pane");
    assert_eq!(
        ed.state.panes.entries[pid_b].jumps.len(),
        1,
        "new pane inherited the source pane's jump entry"
    );

    // A new jump in the new pane must not leak back into the source pane.
    // `ge` (goto-last-line) is a jump command; cursor is at line 0 (inherited
    // from the source pane's `gg`), so it records {line 0} and moves to line 19.
    ed.handle_key(key('g'));
    ed.handle_key(key('e'));
    assert_eq!(
        ed.state.panes.entries[pid_b].jumps.len(),
        2,
        "new pane recorded its own jump after the split"
    );
    assert_eq!(
        ed.state.panes.entries[pid_a].jumps.len(),
        1,
        "source pane's jump list is unchanged after the new pane's jump"
    );
}

// ── Per-pane highlight isolation ────────────────────────────────────────────
//
// `update_highlight_providers` must not write into globally-shared highlight
// state read by every pane's `ScopedHighlighter`; each pane owns its own
// highlight buffers (`PaneHighlights`), computed from that pane's own buffer
// and viewport. Global, focused-buffer-only state would render the focused
// pane's highlight bytes (bracket/search matches) onto every other pane's
// unrelated text, including one viewing a different buffer or the same
// buffer scrolled elsewhere.

/// A search match that spans a `\n` must produce one highlight span per line
/// it touches, each clipped to that line's own content, not a single span
/// computed by converting the match's absolute end offset through whichever
/// line the *start* happened to be on (which gives a corrupt or inverted
/// span whenever a match crosses a line boundary).
///
/// Byte offsets are hand-computed from the known ASCII
/// content ("abc\ndef\n"), not derived by calling the code under test.
#[test]
fn multiline_search_match_splits_into_per_line_highlight_spans() {
    let mut ed = open_headless(None);
    let pid = ed.view.focused_pane();

    ed.feed_key(key('i'));
    for ch in "abc".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_enter());
    for ch in "def".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    assert_eq!(
        ed.doc().text().to_string(),
        "abc\ndef\n",
        "sanity: buffer content"
    );

    // Matches "c\ndef": char 2 ('c') through char 6 ('f'), crossing the
    // line-0/line-1 boundary at the '\n' (char 3).
    ed = ed.with_search_regex("c\ndef");

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.resize_terminal(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    // Every span shares the one search-match scope (`ScopedHighlighter`
    // carries it per-span now, not fixed on the provider), dropped here
    // since this test is about span geometry, not scope resolution.
    let matches: Vec<(usize, usize, usize)> = pane_highlights(&ed, pid, HighlightTier::SearchMatch)
        .into_iter()
        .map(|(line, start, end, _)| (line.index(), start.index(), end.index()))
        .collect();
    assert_eq!(
        matches,
        vec![(0, 2, 3), (1, 0, 3)],
        "line 0 gets 'c' clipped before its own '\\n' (byte 2..3); \
         line 1 gets 'def' from its own start (byte 0..3)"
    );
}

/// Opens a split on the same buffer, focuses the new pane, feeds `keys`
/// there to open a minibuffer session, then closes that pane through the
/// command path a hook or timer would use. Returns the surviving pane.
fn close_pane_under_open_session(ed: &mut Editor, keys: &str) -> PaneId {
    use hume_core::PaneHandle;
    use hume_scripting::host::CommandHost;

    let bid = ed.focused_buffer_id();
    let pid_a = ed.view.focused_pane();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    );
    ed.switch_focused_pane(pid_b);
    for ch in keys.chars() {
        ed.feed_key(key(ch));
    }
    live_host!(ed)
        .run_command_sync(
            "pane-close",
            PaneHandle::with_pane(bid, pid_b),
            NativeCall::default(),
        )
        .expect("pane-close must succeed");
    ed.settle();
    pid_a
}

#[test]
fn closing_the_pane_of_an_open_search_ends_the_search() {
    let mut ed = editor_from("-[a]>bc abc\n");
    let pid_a = close_pane_under_open_session(&mut ed, "/b");
    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Normal);
    ed.feed_key(key_esc());
    ed.feed_key(key('l'));
    assert_eq!(ed.view.focused_pane(), pid_a);
    assert_eq!(state(&ed), "a-[b]>c abc\n");
    assert!(
        ed.state
            .buffers
            .get(ed.focused_buffer_id())
            .search_pattern
            .is_none(),
        "the closed pane's live search is cleared from the buffer"
    );
}

#[test]
fn closing_the_pane_of_an_open_sift_ends_the_sift() {
    let mut ed = editor_from("-[a]>bc abc\n");
    let pid_a = close_pane_under_open_session(&mut ed, "%sb");
    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Normal);
    ed.feed_key(key_esc());
    ed.feed_key(key('l'));
    assert_eq!(ed.view.focused_pane(), pid_a);
    assert_eq!(state(&ed), "a-[b]>c abc\n");
}
