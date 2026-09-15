// Tab pages: `:tabnew`/`:tabclose`/`:tabnext`/`:tabprev`, their mappable
// `goto-next-tab`/`goto-prev-tab` siblings (`Ctrl-p t`/`Ctrl-p T`), and the
// tabline's own visibility setting. See `editor::tab`'s module doc for the
// model — a tab is a saved window layout, not a per-buffer strip.

use super::*;
use hume_engine::pipeline::LayoutTree;
use hume_grid::Rect;
use hume_scripting::ScriptingHost;

#[test]
fn tabnew_opens_a_fresh_pane_in_a_new_tab_and_focuses_it() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focus.id();
    let tab_a = ed.state.tabs.current();

    ed.execute_typed("tabnew", None).unwrap();

    assert_eq!(ed.state.tabs.len(), 2, ":tabnew must add a tab");
    assert_ne!(
        ed.state.tabs.current(),
        tab_a,
        "the new tab becomes current"
    );
    assert_ne!(
        ed.state.focus.id(),
        pid_a,
        "focus moves to the new tab's own pane"
    );
    assert_eq!(
        ed.view.panes.len(),
        2,
        "the new tab's pane is a real, separate pane — not a rename of A's"
    );
    assert!(
        matches!(*ed.view.layout(), LayoutTree::Leaf(id) if id == ed.state.focus.id()),
        "the new tab's layout is a fresh single-leaf tree, not inherited from A's"
    );
}

#[test]
fn tabnew_with_no_arg_views_the_same_buffer_as_the_source_pane() {
    let mut ed = editor_from("-[h]>ello\n");
    let bid_a = ed.focused_buffer_id();

    ed.execute_typed("tabnew", None).unwrap();

    assert_eq!(
        ed.focused_buffer_id(),
        bid_a,
        "a bare :tabnew views the same buffer as the tab it was opened from"
    );
}

/// `tab-new` — the mappable sibling of `:tabnew` with no path argument, for
/// `bind-key!`/`call!` callers that have no typed-command dispatch path.
/// Same assertions as `tabnew_opens_a_fresh_pane_in_a_new_tab_and_focuses_it`
/// and `tabnew_with_no_arg_views_the_same_buffer_as_the_source_pane`, since
/// both share the same `open_tab` core.
#[test]
fn tab_new_mappable_command_opens_a_fresh_tab_viewing_the_focused_buffer() {
    use hume_scripting::host::CommandHost;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focus.id();
    let tab_a = ed.state.tabs.current();
    let bid_a = ed.focused_buffer_id();

    live_host!(ed)
        .run_command_sync("tab-new", None, false, None)
        .expect("tab-new must not error");

    assert_eq!(ed.state.tabs.len(), 2, "tab-new must add a tab");
    assert_ne!(
        ed.state.tabs.current(),
        tab_a,
        "the new tab becomes current"
    );
    assert_ne!(
        ed.state.focus.id(),
        pid_a,
        "focus moves to the new tab's own pane"
    );
    assert_eq!(
        ed.focused_buffer_id(),
        bid_a,
        "the new tab views the same buffer as the source pane"
    );
}

#[test]
fn tabclose_is_refused_on_the_last_tab() {
    let mut ed = editor_from("-[h]>ello\n");
    let tab_a = ed.state.tabs.current();

    ed.execute_typed("tabclose", None).unwrap();

    assert_eq!(ed.state.tabs.len(), 1, "the only tab must survive");
    assert_eq!(ed.state.tabs.current(), tab_a);
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("last tab")),
        "refusal must report a status message, got {:?}",
        ed.state.status_msg
    );
}

#[test]
fn tabclose_frees_every_pane_the_closed_tab_owns_and_restores_the_previous_tab() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focus.id();
    let tab_a = ed.state.tabs.current();

    ed.execute_typed("tabnew", None).unwrap();
    // A split inside the new tab — tabclose must free both of its panes,
    // not just the one that was focused.
    ed.execute_typed("split", None).unwrap();
    assert_eq!(ed.view.panes.len(), 3, "A, plus B's two split panes");

    ed.execute_typed("tabclose", None).unwrap();

    assert_eq!(ed.state.tabs.len(), 1);
    assert_eq!(ed.state.tabs.current(), tab_a);
    assert_eq!(ed.state.focus.id(), pid_a, "focus returns to A's own pane");
    assert_eq!(
        ed.view.panes.len(),
        1,
        "both of the closed tab's panes are gone, not just the focused one"
    );
    assert!(
        matches!(*ed.view.layout(), LayoutTree::Leaf(id) if id == pid_a),
        "A's own single-leaf layout is restored, not left as a dangling split"
    );
}

// ── Per-frame work follows the active tab ──────────────────────────────────────

/// A hidden tab's pane must not get decorated by `prepare_frame` — only
/// the active tab's own pane, even when both view the same buffer and the
/// same decoration data.
#[test]
fn a_hidden_tab_s_pane_keeps_its_decoration_state_until_its_tab_is_focused() {
    // `editor_from`'s bootstrap pane is built via `Pane::new` directly, with
    // no `panes.render` entry (see `Editor::for_testing`'s comment) — only
    // `open_pane` seeds one, so this test inspects B's decoration state
    // only (B's pane comes from `open_tab` → `open_pane`), not A's.
    let mut ed = editor_from("-[h]>ello\n");
    let bid = ed.focused_buffer_id();

    ed.execute_typed("tabnew", None).unwrap();
    let pid_b = ed.state.focus.id();
    ed.execute_typed("tabprev", None).unwrap();

    let scope = ed.view.registry.intern("ui.virtual");
    ed.state.config.decorations.set_virtual_lines(
        "test".to_string(),
        bid,
        vec![hume_decorations::VirtualLineEntry {
            pos: co(0),
            text: "x".to_string(),
            before: false,
            scope,
            segments: Vec::new(),
        }],
    );

    frame(&mut ed, 80, 25);

    let virtual_lines_b = |ed: &Editor| ed.state.panes.render.get(pid_b).unwrap().virtual_lines();
    assert!(
        virtual_lines_b(&ed).is_empty(),
        "the hidden tab's pane, viewing the same buffer, must be left untouched"
    );

    ed.execute_typed("tabnext", None).unwrap();
    ed.settle();
    ed.prepare_frame(&mut hume_engine::pipeline::RenderContext::new());

    assert!(
        virtual_lines_b(&ed).contains_key(&hume_rope::line::ContentLine::new(0)),
        "B's pane must pick up the decoration once its tab is focused"
    );
}

/// A resize while a tab is hidden must not touch its panes' viewports —
/// they stay exactly as sized when that tab was last active, and only
/// catch up once it's focused again.
#[test]
fn resizing_while_a_tab_is_hidden_leaves_it_stale_until_refocused() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("tabnew", None).unwrap();
    let pid_b = ed.state.focus.id();
    frame(&mut ed, 80, 25);
    let width_before = ed.view.panes[pid_b].viewport.width;
    assert!(
        width_before > 40,
        "setup: B sized to the wide terminal while active"
    );

    ed.execute_typed("tabprev", None).unwrap();

    // Resize while B is hidden.
    frame(&mut ed, 40, 10);
    assert_eq!(
        ed.view.panes[pid_b].viewport.width, width_before,
        "B's viewport must be untouched by a resize while its tab is hidden"
    );

    // Switch back: the terminal area is already 40×10 from the resize above
    // (`prepare_frame`'s own step 0 re-partitions from `last_terminal_area`
    // every frame), so B must catch up the moment its tab becomes active.
    ed.execute_typed("tabnext", None).unwrap();
    ed.settle();
    ed.prepare_frame(&mut hume_engine::pipeline::RenderContext::new());
    assert_ne!(
        ed.view.panes[pid_b].viewport.width, width_before,
        "B must be re-sized to the current terminal once its tab is focused again"
    );
}

/// The switch itself must resync the incoming tab's viewport dims — not
/// just the next frame's `prepare_frame`. A command dispatch that switches
/// tabs and then reads pane geometry in the same call (a scroll bound to a
/// tab-switch key, a Steel body chaining a motion onto `goto-next-tab`)
/// would otherwise see the outgoing tab's stale width/height.
#[test]
fn switching_to_a_tab_resyncs_its_viewport_before_the_next_frame() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("tabnew", None).unwrap();
    let pid_b = ed.state.focus.id();
    frame(&mut ed, 80, 25);
    let width_before = ed.view.panes[pid_b].viewport.width;
    assert!(
        width_before > 40,
        "setup: B sized to the wide terminal while active"
    );

    ed.execute_typed("tabprev", None).unwrap();
    frame(&mut ed, 40, 10);
    assert_eq!(
        ed.view.panes[pid_b].viewport.width, width_before,
        "setup: B stale while hidden, per the test above"
    );

    // No `frame`/`prepare_frame` call after this — the switch alone must resync.
    ed.execute_typed("tabnext", None).unwrap();
    assert_ne!(
        ed.view.panes[pid_b].viewport.width, width_before,
        "B's viewport must resync the moment its tab goes live, before any frame runs"
    );
}

/// `:q` on a tab's own last pane closes the tab (Vim's placement), not the
/// editor — `view.panes.len()` is a global pool shared by every tab, so it
/// stays `> 1` here even though the active tab has just this one pane.
#[test]
fn quit_on_a_tab_s_last_pane_closes_the_tab() {
    let mut ed = editor_from("-[h]>ello\n");
    let tab_a = ed.state.tabs.current();
    ed.execute_typed("tabnew", None).unwrap();
    assert_eq!(ed.state.tabs.len(), 2);

    ed.execute_typed("quit", None).unwrap();

    assert!(
        !ed.state.should_quit,
        ":q with other tabs open must not quit"
    );
    assert_eq!(ed.state.tabs.len(), 1, "the tab closes");
    assert_eq!(ed.state.tabs.current(), tab_a, "focus returns to tab A");
}

/// `Ctrl-p c` stays pane-scoped: on a tab's own last pane it refuses with a
/// status message, even with other tabs open — `:q` owns closing the tab.
#[test]
fn close_pane_is_refused_on_a_tab_s_last_pane_with_other_tabs_open() {
    let mut ed = editor_from_kitty("-[h]>ello\n");
    let tab_b = {
        ed.execute_typed("tabnew", None).unwrap();
        ed.state.tabs.current()
    };

    ed.handle_key(key_ctrl('p'));
    ed.handle_key(key('c'));

    assert_eq!(ed.state.tabs.len(), 2, "no tab was closed");
    assert_eq!(ed.state.tabs.current(), tab_b);
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("cannot close last pane")
    );
}

#[test]
fn tabclose_on_the_leftmost_tab_focuses_its_right_neighbour() {
    let mut ed = editor_from("-[h]>ello\n");
    let tab_a = ed.state.tabs.current();
    ed.execute_typed("tabnew", None).unwrap();
    let tab_b = ed.state.tabs.current();
    ed.execute_typed("tabnew", None).unwrap();
    assert_eq!(
        ed.state.tabs.order(),
        &[tab_a, tab_b, ed.state.tabs.current()]
    );

    ed.execute_typed("tabprev", None).unwrap();
    ed.execute_typed("tabprev", None).unwrap();
    assert_eq!(
        ed.state.tabs.current(),
        tab_a,
        "sanity: focused on A, leftmost"
    );

    ed.execute_typed("tabclose", None).unwrap();

    assert_eq!(
        ed.state.tabs.current(),
        tab_b,
        "closing the leftmost tab focuses its right neighbour, not a wrap to the last tab"
    );
}

/// The common case: closing a middle tab focuses its *right* neighbour —
/// Vim's own `:tabclose` default. Left-preference only kicks in for the
/// rightmost tab (see the test below), which this codebase implements as
/// its own default rather than opting into Vim 9.1's `'tabclose'=left`.
#[test]
fn tabclose_in_the_middle_focuses_its_right_neighbour() {
    let mut ed = editor_from("-[h]>ello\n");
    let tab_a = ed.state.tabs.current();
    ed.execute_typed("tabnew", None).unwrap();
    let tab_b = ed.state.tabs.current();
    ed.execute_typed("tabnew", None).unwrap();
    let tab_c = ed.state.tabs.current();
    assert_eq!(ed.state.tabs.order(), &[tab_a, tab_b, tab_c]);

    ed.execute_typed("tabprev", None).unwrap();
    assert_eq!(
        ed.state.tabs.current(),
        tab_b,
        "sanity: focused on B, middle"
    );

    ed.execute_typed("tabclose", None).unwrap();

    assert_eq!(
        ed.state.tabs.current(),
        tab_c,
        "closing a middle tab focuses its right neighbour, not the left one \
         `open_after_current`'s own insert-after-current placement came from"
    );
}

/// Left-preference is the fallback, reachable only from the rightmost tab
/// (there is no right neighbour to prefer).
#[test]
fn tabclose_on_the_rightmost_tab_focuses_its_left_neighbour() {
    let mut ed = editor_from("-[h]>ello\n");
    let tab_a = ed.state.tabs.current();
    ed.execute_typed("tabnew", None).unwrap();
    let tab_b = ed.state.tabs.current();
    assert_eq!(ed.state.tabs.order(), &[tab_a, tab_b]);
    assert_eq!(
        ed.state.tabs.current(),
        tab_b,
        "sanity: focused on B, rightmost"
    );

    ed.execute_typed("tabclose", None).unwrap();

    assert_eq!(
        ed.state.tabs.current(),
        tab_a,
        "closing the rightmost tab falls back to its left neighbour"
    );
}

#[test]
fn splitting_inside_one_tab_never_touches_another_tab_s_layout() {
    // The defining property of "a tab is a saved layout": a split issued
    // while tab B is focused must never appear in tab A's own tree.
    let mut ed = editor_from("-[h]>ello\n");
    let tab_a_layout_before = ed.view.layout().clone();

    ed.execute_typed("tabnew", None).unwrap();
    ed.execute_typed("split", None).unwrap();
    assert!(matches!(*ed.view.layout(), LayoutTree::Split { .. }));

    ed.execute_typed("tabprev", None).unwrap();
    assert_eq!(
        *ed.view.layout(),
        tab_a_layout_before,
        "switching back to A must restore its own untouched single-leaf layout"
    );
}

#[test]
fn tabnext_and_tabprev_wrap_around() {
    let mut ed = editor_from("-[h]>ello\n");
    let tab_a = ed.state.tabs.current();
    ed.execute_typed("tabnew", None).unwrap();
    let tab_b = ed.state.tabs.current();
    ed.execute_typed("tabnew", None).unwrap();
    let tab_c = ed.state.tabs.current();

    ed.execute_typed("tabnext", None).unwrap();
    assert_eq!(ed.state.tabs.current(), tab_a, "wraps from C to A");

    ed.execute_typed("tabprev", None).unwrap();
    assert_eq!(ed.state.tabs.current(), tab_c, "wraps back from A to C");

    ed.execute_typed("tabprev", None).unwrap();
    assert_eq!(ed.state.tabs.current(), tab_b);
}

#[test]
fn tabnext_and_tabprev_aliases_work() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("tabnew", None).unwrap();
    let tab_b = ed.state.tabs.current();

    ed.execute_typed("tabp", None).unwrap();
    ed.execute_typed("tabn", None).unwrap();
    assert_eq!(ed.state.tabs.current(), tab_b);
}

/// `Ctrl-p t` / `Ctrl-p T` — the mappable `goto-next-tab`/`goto-prev-tab`
/// siblings of `:tabnext`/`:tabprev`.
#[test]
fn ctrl_p_t_and_shift_t_cycle_tabs() {
    let mut ed = editor_from_kitty("-[h]>ello\n");
    let tab_a = ed.state.tabs.current();
    ed.execute_typed("tabnew", None).unwrap();
    let tab_b = ed.state.tabs.current();

    ed.handle_key(key_ctrl('p'));
    ed.handle_key(key('T'));
    assert_eq!(
        ed.state.tabs.current(),
        tab_a,
        "Ctrl-p T goes to the previous tab"
    );

    ed.handle_key(key_ctrl('p'));
    ed.handle_key(key('t'));
    assert_eq!(
        ed.state.tabs.current(),
        tab_b,
        "Ctrl-p t goes to the next tab"
    );
}

/// `goto-next-tab` dispatched via `call!`/`run_command_sync` — the path a
/// Steel hook or a custom Insert-mode keybinding reaches, not a keypress or
/// mouse click — must leave the outgoing pane exactly the way
/// `clicking_another_tab_while_in_insert_...` (`tests/mouse.rs`) proves the
/// mouse path does: `focus_pane` is the one chokepoint every `focus` write
/// goes through, so this isn't a per-caller special case.
#[test]
fn goto_next_tab_via_call_while_in_insert_exits_insert_and_commits_the_outgoing_pane() {
    use hume_scripting::host::CommandHost;

    let tmp = safe_tempdir();
    let path = tmp.path().join("other.txt");
    std::fs::write(&path, "zz\n").unwrap();

    // "  x\ncd\n": cursor on line 0's own trailing '\n', same setup as
    // `clicking_another_tab_while_in_insert_...`.
    let mut ed = editor_from("  x-[\n]>cd\n");
    let bid_a = ed.focused_buffer_id();
    ed.execute_typed("tabnew", Some(path.to_str().unwrap()))
        .unwrap();
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "setup: distinct buffers");
    ed.execute_typed("tabprev", None).unwrap();

    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    // Enter copies "  " onto a new line and lands the cursor on that blank,
    // auto-indented line — `autoindent_pending` is set, so exiting Insert
    // now will trim it.
    assert_eq!(
        ed.state.buffers.get(bid_a).text().to_string(),
        "  x\n  \ncd\n",
        "setup: blank auto-indented line pending trim"
    );
    assert_eq!(ed.state.mode(), Mode::Insert, "setup: still typing in A");

    live_host!(ed)
        .run_command_sync("goto-next-tab", None, false, None)
        .expect("goto-next-tab must not error");

    assert_eq!(
        ed.state.mode(),
        Mode::Normal,
        "leaving A via call! must exit Insert, same as a pane click"
    );
    assert_eq!(
        ed.state.buffers.get(bid_a).text().to_string(),
        "  x\n\ncd\n",
        "A's blank auto-indented line's whitespace must have been trimmed on exit"
    );
    assert_eq!(
        ed.state.buffers.get(bid_b).text().to_string(),
        "zz\n",
        "B's own buffer must be untouched by A's exit"
    );
}

// ── `tabline` setting ─────────────────────────────────────────────────────────

#[test]
fn tabline_dynamic_default_hides_with_one_tab_and_shows_with_two() {
    let mut ed = editor_from("-[h]>ello\n");
    frame(&mut ed, 40, 10);
    assert!(
        !ed.state.tabline_view.read().visible,
        "one tab: tabline stays hidden under the 'dynamic' default"
    );

    ed.execute_typed("tabnew", None).unwrap();
    ed.settle();
    ed.prepare_frame(&mut hume_engine::pipeline::RenderContext::new());
    assert!(
        ed.state.tabline_view.read().visible,
        "two tabs: tabline must show under the 'dynamic' default"
    );
}

#[test]
fn tabline_never_stays_hidden_even_with_multiple_tabs() {
    // Steel round-tripping `set-option!` down to this same field is covered
    // by `settings::tests::set_global_tabline` — this test only needs to
    // confirm the tabline sync itself honors the setting once written.
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.settings.tabline = crate::editor::settings::TablineVisibility::Never;
    ed.execute_typed("tabnew", None).unwrap();
    frame(&mut ed, 40, 10);

    assert!(!ed.state.tabline_view.read().visible);
}

/// A hidden tabline must skip building any `TabEntry` at all — `visible`
/// controls whether the row paints, but `sync_tabline_view`'s fast path for
/// the hidden case returns before the per-tab loop runs.
#[test]
fn hidden_tabline_populates_no_entries() {
    let mut ed = editor_from("-[h]>ello\n");
    frame(&mut ed, 40, 10);

    let guard = ed.state.tabline_view.read();
    assert!(!guard.visible);
    assert!(guard.tabs.is_empty());
}

/// Once a narrow terminal has scrolled the tab bar forward to keep the
/// active (last-opened) tab in view, widening the terminal enough to fit
/// every tab must retreat the window back to 0 — not leave earlier tabs
/// hidden behind a scroll position the row no longer needs.
#[test]
fn widening_the_terminal_after_scrolling_brings_earlier_tabs_back() {
    let mut ed = editor_from("-[h]>ello\n");
    for _ in 0..5 {
        ed.execute_typed("tabnew", None).unwrap();
    }
    assert_eq!(ed.state.tabs.len(), 6);

    // Narrow enough that only one "*scratch*" tab at a time fits — forces
    // scroll to advance well past 0 to keep the active (last) tab visible.
    frame(&mut ed, 20, 10);
    assert!(
        ed.state.tabline_view.read().scroll > 0,
        "setup: narrow terminal must have scrolled"
    );

    // Wide enough that all six now fit together.
    frame(&mut ed, 200, 10);
    assert_eq!(
        ed.state.tabline_view.read().scroll,
        0,
        "widening must retreat the scroll window back to 0"
    );
}

/// The same retreat must happen when tabs close down to a count that fits
/// the row already open, not only when the terminal itself widens.
#[test]
fn tabclose_down_to_a_fitting_count_resets_the_scroll_window() {
    let mut ed = editor_from("-[h]>ello\n");
    for _ in 0..5 {
        ed.execute_typed("tabnew", None).unwrap();
    }

    // Wide enough to fit two "*scratch*" tabs at once, not six.
    frame(&mut ed, 30, 10);
    assert!(
        ed.state.tabline_view.read().scroll > 0,
        "setup: six tabs at this width must have scrolled"
    );

    for _ in 0..4 {
        ed.execute_typed("tabclose", None).unwrap();
    }
    assert_eq!(ed.state.tabs.len(), 2, "setup: down to two tabs");
    ed.settle();
    ed.prepare_frame(&mut hume_engine::pipeline::RenderContext::new());
    assert_eq!(
        ed.state.tabline_view.read().scroll,
        0,
        "two tabs fit together at this width — the window must reset"
    );
}

/// Full-frame render with two tabs open — the active one styled distinctly,
/// a `│` separator between them, and the pane content below unaffected.
/// `lib.rs`'s tab shows no dirty marker; `*scratch*`'s label comes from the
/// buffer that both tabs shared before `:tabnew <path>` opened a second one.
#[test]
fn two_tabs_render_with_the_active_one_styled_distinctly() {
    use super::render_snapshot::render_to_styled_string;

    let tmp = safe_tempdir();
    let path = tmp.path().join("lib.rs");
    std::fs::write(&path, "fn lib() {}\n").unwrap();

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = crate::testing::build_snapshot_theme();
    ed.execute_typed("tabnew", Some(path.to_str().unwrap()))
        .unwrap();

    let rect = Rect::new(0, 0, 40, 4);
    let full = render_to_styled_string(&mut ed, rect);
    // The statusline (last row) shows the tempdir's own absolute path,
    // randomized per test run — trimmed so the pinned snapshot covers only
    // the deterministic tabline + pane content this test actually exercises.
    let without_statusline: String = full.lines().take(3).collect::<Vec<_>>().join("\n");
    insta::assert_snapshot!(without_statusline);
}

#[test]
fn tabline_always_shows_even_with_one_tab() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.settings.tabline = crate::editor::settings::TablineVisibility::Always;
    frame(&mut ed, 40, 10);

    assert!(ed.state.tabline_view.read().visible);
}

/// Characterization (behavior unchanged, no red run needed — code review
/// fix #5, commit range 48c11211..ebc3b2e0): `close_tab`'s Insert-session
/// teardown reads and writes exclusively through pool-based lookups
/// (`focused_buffer_id` indexes `view.panes` directly by id, never through
/// `view.layout`), so it already resolved correctly regardless of whether
/// `view.layout` had been swapped to the survivor's tree yet — this passes
/// identically before and after moving the teardown to run before that
/// swap. The fix closes a *structural* inconsistency window (layout and
/// focus transiently naming different tabs) that no current consumer
/// observes, not an active bug; this test pins that the closing tab's own
/// edit lands on its own buffer, not the survivor's, so a future consumer
/// that *does* resolve through the layout during teardown stays correct too.
#[test]
fn tabclose_while_in_insert_exits_insert_and_commits_the_outgoing_pane() {
    let tmp = safe_tempdir();
    let path = tmp.path().join("other.txt");
    std::fs::write(&path, "zz\n").unwrap();

    // "  x\ncd\n": cursor on line 0's own trailing '\n', same setup as
    // `click_after_blank_line_trim_lands_on_correct_char` (mouse.rs).
    let mut ed = editor_from("  x-[\n]>cd\n");
    let tab_closing = ed.state.tabs.current();
    let bid_closing = ed.focused_buffer_id();

    ed.execute_typed("tabnew", Some(path.to_str().unwrap()))
        .unwrap();
    let tab_survivor = ed.state.tabs.current();
    assert_ne!(tab_closing, tab_survivor, "setup: distinct tabs");

    ed.execute_typed("tabprev", None).unwrap();
    assert_eq!(ed.state.tabs.current(), tab_closing, "setup: back on A");

    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    // Enter copies "  " onto a new line and lands the cursor on that blank,
    // auto-indented line — `autoindent_pending` is set, so leaving Insert
    // now will trim it.
    assert_eq!(
        ed.state.buffers.get(bid_closing).text().to_string(),
        "  x\n  \ncd\n",
        "setup: blank auto-indented line pending trim"
    );
    assert_eq!(ed.state.mode(), Mode::Insert, "setup: still typing in A");

    ed.execute_typed("tabclose", None).unwrap();

    assert_eq!(
        ed.state.tabs.current(),
        tab_survivor,
        ":tabclose must land on the survivor tab"
    );
    assert_eq!(
        ed.state.mode(),
        Mode::Normal,
        "closing a tab from Insert must exit Insert, same as any other focus switch"
    );
    assert_eq!(
        ed.state.buffers.get(bid_closing).text().to_string(),
        "  x\n\ncd\n",
        "the closing tab's own blank auto-indented line must have been trimmed on exit, \
         not left uncommitted or applied to the survivor"
    );
}

/// Regression: `last_viewport_key` used to be keyed only by pane id, and
/// `prepare_frame`'s step 4 only ever visits *active-tab* panes — so a
/// background tab's pane kept its last-observed `(top_line, height)`
/// untouched the whole time it was hidden. Returning to it at unchanged
/// terminal geometry then matched that stale entry and never re-armed
/// `on-viewport-change`, so a viewport-driven consumer (LSP inlay hints)
/// stayed pinned to whatever it last saw before the tab went to the
/// background (code review fix #1, commit range 48c11211..ebc3b2e0).
#[test]
fn returning_to_a_background_tab_at_unchanged_geometry_still_refires_viewport_change() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    let tab_a = ed.state.tabs.current();

    ed.execute_typed("tabnew", None).unwrap();
    let tab_b = ed.state.tabs.current();
    let bid_b = ed.focused_buffer_id();
    seed_frame(&mut ed, 40, 10); // B observed once while its own tab is active

    ed.execute_typed("tabprev", None).unwrap();
    assert_eq!(ed.state.tabs.current(), tab_a, "setup: back on A");
    seed_frame(&mut ed, 40, 10); // same 40x10 geometry throughout — only which
    // tab is active changes from here on

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-viewport-change (lambda (bid first end)
             (set-buffer-option! bid "tab-width" (+ 1 (get-option bid "tab-width")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    ed.execute_typed("tabnext", None).unwrap();
    assert_eq!(ed.state.tabs.current(), tab_b, "setup: back on B");
    frame(&mut ed, 40, 10);

    ed.drain_async_sources();
    ed.settle();

    assert_eq!(
        ed.state.buffers.get(bid_b).overrides.tab_width,
        Some(EditorSettings::default().tab_width + 1),
        "returning to B's tab at unchanged geometry must still re-fire on-viewport-change"
    );
}

/// Regression: `last_viewport_key`'s key used to be `(top_line, height)`
/// alone, with no `buffer_id` — so a pane switching buffers (`:e`, `:b#`)
/// at unchanged geometry matched its own stale entry and never re-armed
/// `on-viewport-change` for the newly-shown buffer (code review fix #1,
/// commit range 48c11211..ebc3b2e0). Single pane, single tab — this is the
/// same-tab twin of `returning_to_a_background_tab_...` above, which covers
/// the pane-dropped-from-the-active-set cause instead.
#[test]
fn switching_buffer_in_place_at_unchanged_geometry_still_refires_viewport_change() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    seed_frame(&mut ed, 40, 10); // seeds last_viewport_key[pid] keyed on A's buffer

    let file_tmp = safe_tempdir();
    let file = file_tmp.path().join("second.txt");
    std::fs::write(&file, "hi\n").unwrap();

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-viewport-change (lambda (bid first end)
             (set-buffer-option! bid "tab-width" (+ 1 (get-option bid "tab-width")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid_b = ed.focused_buffer_id();

    frame(&mut ed, 40, 10); // identical (top_line, height) — only buffer_id differs

    ed.drain_async_sources();
    ed.settle();

    assert_eq!(
        ed.state.buffers.get(bid_b).overrides.tab_width,
        Some(EditorSettings::default().tab_width + 1),
        "switching buffer in place at unchanged (top_line, height) must still \
         re-fire on-viewport-change"
    );
}

/// A pane's debounced `on-viewport-change` timer must not fire with its
/// frozen bounds if the pane's tab went to the background before the timer
/// came due. `prepare_frame`'s own housekeeping (dropping
/// `last_viewport_key`, retiring `viewport_debounce` on close) only runs
/// for a pane no longer in the pool at all — a background-tab pane still is
/// — and the timer can come due from `settle()`'s drain, which runs
/// *before* that housekeeping even sees the pane leave the active set. The
/// guard belongs in `queue_viewport_change` itself, the one chokepoint
/// every fire (this debounce timer, a config reload's resync) goes through.
#[test]
fn a_pending_debounced_viewport_change_does_not_fire_for_a_pane_that_went_background_first() {
    let tmp = safe_tempdir();
    // A second, distinct file for the new tab — `tabnew` with no argument
    // would instead duplicate A's own pane onto its same buffer, which
    // would still legitimately re-arm and fire for bid_a from the new
    // pane, defeating the point of backgrounding it.
    let other = tmp.path().join("other.txt");
    std::fs::write(&other, "hi\n").unwrap();

    let mut ed = editor_from("-[a]>bc\ndef\nghi\njkl\nmno\n");
    ed.state.settings.lsp_viewport_debounce_ms = 0;
    let bid_a = ed.focused_buffer_id();
    seed_frame(&mut ed, 40, 3); // baseline last_viewport_key[pid_a] at top_line 0

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-viewport-change (lambda (bid first end)
             (set-buffer-option! bid "tab-width" (+ 1 (get-option bid "tab-width")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    // Scroll A, arming its debounce timer (ms=0, so due on the very next drain).
    seek_to_line(&mut ed, 4);
    frame(&mut ed, 40, 3);

    // Switch away before that timer is drained — the dispatch alone, no
    // frame in between, mirrors a tab-switch keypress landing right after
    // the scroll that armed the timer.
    ed.execute_typed("tabnew", Some(other.to_str().unwrap()))
        .unwrap();

    // The new tab's own next frame drains the still-pending timer, inside
    // its `settle()` — which runs before this same frame's own housekeeping
    // drops pid_a's `last_viewport_key`. Must not fire on-viewport-change for A.
    // (It legitimately arms and fires for the new tab's own buffer — that's
    // not under test here.)
    frame(&mut ed, 40, 3);
    ed.drain_async_sources();
    ed.settle();

    assert_eq!(
        ed.state.buffers.get(bid_a).overrides.tab_width,
        None,
        "a pane's tab going to the background before its debounce timer fires \
         must suppress that fire, not deliver it with frozen bounds"
    );

    // Returning to A must still re-fire, at whatever its (unchanged) geometry
    // now is — the suppression above must not have also skipped this.
    ed.execute_typed("tabprev", None).unwrap();
    frame(&mut ed, 40, 3);
    ed.drain_async_sources();
    ed.settle();

    assert_eq!(
        ed.state.buffers.get(bid_a).overrides.tab_width,
        Some(EditorSettings::default().tab_width + 1),
        "returning to A's tab must still re-fire on-viewport-change exactly once"
    );
}

/// Regression: `sync_tabline_view` used to rebuild `TablineViewState` from
/// scratch every frame — one allocating `display_name()` call per tab plus
/// an O(n²) scroll probe — even on a frame where nothing about the tab bar
/// changed. `tabline_signature` gates the rebuild now; this pins that a
/// second `prepare_frame` with nothing tab-affecting in between really does
/// skip it, not just happen to recompute an identical result.
#[test]
fn sync_tabline_view_skips_the_rebuild_when_nothing_changed() {
    let mut ed = editor_from("-[a]>bc\n");
    ed.execute_typed("tabnew", None).unwrap();
    frame(&mut ed, 40, 10);

    // A value a real rebuild always overwrites — the scroll probe runs
    // unconditionally whenever the rebuild itself runs at all.
    let mut forced = ed.state.tabline_view.read().clone();
    forced.scroll = 9999;
    ed.state.tabline_view.set(forced);

    frame(&mut ed, 40, 10);

    assert_eq!(
        ed.state.tabline_view.read().scroll,
        9999,
        "an unchanged signature must skip the rebuild entirely, not just \
         recompute the same scroll value"
    );
}
