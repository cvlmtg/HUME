// `EditorState::buffer_tag` and `Pane::line_store` lifetime — the scope key
// `hume_engine::rows::line_store` uses to decide whether a cached line format
// still describes the buffer it was built from, and the per-frame rewind that
// catches what the key cannot see.

use std::rc::Rc;

use super::doubles::{FormatProbe, InlineHint, VirtualRows};
use super::*;
use hume_editing::selection::Selection;
use hume_engine::pipeline::RenderContext;
use hume_engine::providers::VirtualLineAnchor;
use hume_grid::{Grid, Rect};

/// 50 single-char lines, unwrapped. Built on `Editor::open` (not
/// `Editor::for_testing`), like `messages.rs`, because only `open`'s startup
/// path runs the initial pane through `build_pane` — the same constructor
/// `:split` uses for a new pane. A `for_testing` pane skips it and carries no
/// gutter columns, so it would resolve a different `content_width` than a
/// split-created pane over the identical buffer, an unrelated mismatch that
/// would rescope the store and mask what these tests check. Also sets the
/// *global* wrap mode rather than a per-pane override, for the same reason:
/// a pane `:split` creates starts with no override of its own and would
/// otherwise fall back to the default soft wrap instead of matching.
fn many_lines_editor() -> Editor {
    let mut ed = Editor::open(None, Arc::new(|| {})).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state
        .buffers
        .get_mut(bid)
        .set_view_content(BufferText::from("a\n".repeat(50).as_str()));
    set_cursor(&mut ed, 0);
    ed.state.settings.wrap_mode = hume_engine::pane::WrapMode::None;
    ed
}

/// A grouped edit (an open Insert session) bumps `Buffer::text_gen` on every
/// keystroke but does not record a new revision — `commit_edit_group` is what
/// moves `history.current_id()`, and that only runs on session end. A tag built
/// from the revision id is therefore frozen for the whole session while the
/// rope underneath it changes.
#[test]
fn buffer_tag_changes_within_an_open_insert_session() {
    let mut ed = editor_from("-[h]>ello\n");
    let bid = ed.focused_buffer_id();
    let before = ed.state.buffer_tag(bid);

    ed.feed_key(key('i'));
    ed.feed_key(key('X'));

    let after = ed.state.buffer_tag(bid);
    assert_ne!(
        before, after,
        "buffer_tag must move with every keystroke of an open insert session, \
         not just when the session commits"
    );
}

/// `Buffer::set_view_content` (the `:messages`/`:ls` refresh path) rebuilds
/// `History` from scratch, so `revision_id()` returns to `RevisionId(0)` on
/// every call — the same value the buffer started at. A tag keyed on the
/// revision id alone therefore repeats across two refreshes with different
/// content. Calls `set_view_content` directly (bypassing `:messages`, whose
/// handler also touches the decoration store on every call, which would mask
/// this specific hazard) to isolate it.
#[test]
fn buffer_tag_changes_across_a_set_view_content_refresh() {
    let mut ed = editor_from("-[s]>cratch\n");
    let bid = ed.focused_buffer_id();
    let before = ed.state.buffer_tag(bid);

    ed.state
        .buffers
        .get_mut(bid)
        .set_view_content(BufferText::from("refreshed\n"));

    let after = ed.state.buffer_tag(bid);
    assert_ne!(
        before, after,
        "a set_view_content refresh resets history to the root revision, so a \
         tag keyed on the revision id alone would repeat across two refreshes \
         with different content"
    );
}

/// Two panes viewing the same buffer at the same width resolve a
/// bit-identical `FormatKey` — `:split` stacks panes, so both keep the full
/// terminal width (only height changes, and height isn't part of the key).
/// Nothing in that key names the pane, so a single shared store would serve
/// one pane's entry for a line to the other and skip querying that pane's
/// own providers entirely. The store living *on* the pane is what makes that
/// unrepresentable; this is the end-to-end proof of it.
#[test]
fn line_store_does_not_leak_between_panes() {
    let mut ed = many_lines_editor();
    let pid_a = ed.state.focused_pane_id;
    const TARGET: usize = 25;

    let calls_a = Rc::new(Cell::new(0));
    ed.view.panes[pid_a]
        .providers
        .add_decoration_source(Box::new(
            VirtualRows::uniform(VirtualLineAnchor::Before(TARGET), 1, "V")
                .counting(Rc::clone(&calls_a)),
        ));
    seek_to_line(&mut ed, TARGET);
    ed.feed_key(key('z'));
    ed.feed_key(key('z'));
    assert_eq!(calls_a.get(), 1, "pane A's own provider must be queried");

    ed.execute_typed("split", None).unwrap();
    // `:split` alone leaves the new pane's viewport at its zero-value
    // default — a real frame is what sizes it against the layout tree, and
    // an unsized pane B walks a different row list from pane A's, which
    // would mask the isolation this test exists to check.
    ed.sync_viewport_dims(80, 24);
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_b, pid_a, "split must move focus to the new pane");

    let calls_b = Rc::new(Cell::new(0));
    ed.view.panes[pid_b]
        .providers
        .add_decoration_source(Box::new(
            VirtualRows::uniform(VirtualLineAnchor::Before(TARGET), 1, "V")
                .counting(Rc::clone(&calls_b)),
        ));
    seek_to_line(&mut ed, TARGET);
    ed.feed_key(key('z'));
    ed.feed_key(key('z'));

    assert_eq!(
        calls_b.get(),
        1,
        "pane B must query its own provider for the line pane A already \
         cached, not silently reuse pane A's entry for it"
    );
}

/// A `z`-scroll and the frame pipeline share one store, so this pins the
/// rewind from the *between-frame* entry point — the sibling test below
/// drives the render path into the same store. Entries must not survive a
/// frame: the per-pane inlay-hint/virtual-line mirrors are rebuilt every
/// frame filtered to that frame's viewport without bumping the decoration
/// generation (`line_store`'s module doc), so a stale entry from before the
/// rebuild can describe a block shape that no longer holds.
#[test]
fn a_between_frame_walk_does_not_survive_a_frame() {
    let mut ed = many_lines_editor();
    let pid = ed.state.focused_pane_id;
    const TARGET: usize = 25;

    let calls = Rc::new(Cell::new(0));
    ed.view.panes[pid].providers.add_decoration_source(Box::new(
        VirtualRows::uniform(VirtualLineAnchor::Before(TARGET), 1, "V").counting(Rc::clone(&calls)),
    ));
    seek_to_line(&mut ed, TARGET);
    ed.feed_key(key('z'));
    ed.feed_key(key('z'));
    assert_eq!(
        calls.get(),
        1,
        "first z z must populate the entry it walks over"
    );

    seek_to_line(&mut ed, TARGET);
    ed.feed_key(key('z'));
    ed.feed_key(key('z'));
    assert_eq!(
        calls.get(),
        1,
        "a second z z within the same frame interval must reuse the entry, \
         not re-query"
    );

    // Away from TARGET before the frame: `prepare_frame`'s own per-pane
    // scroll step (frame.rs's `scroll_into_view`) walks this same store,
    // following the cursor's *current* line — leaving the cursor on TARGET
    // would have that pass re-populate the very entry this test wants to
    // find dropped, hiding the rewind it is checking for.
    seek_to_line(&mut ed, 0);
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 24);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    seek_to_line(&mut ed, TARGET);
    ed.feed_key(key('z'));
    ed.feed_key(key('z'));
    assert_eq!(
        calls.get(),
        2,
        "a frame boundary must drop the entry, not let it survive to the \
         next interval"
    );
}

/// An INLINE-kind source whose emission is toggled by a shared flag rather
/// than driven by a decoration store — standing in for the real inlay-hint
/// mirror, which is rebuilt every frame filtered to the viewport it shows
/// *without* bumping the decoration generation (see `line_store`'s module
/// doc). So a hint appearing or disappearing between two frames is a change
/// `FormatKey` cannot see; only the per-frame rewind catches it — what
/// `line_store`'s module doc calls "a correctness requirement rather than
/// hygiene". `render_to_buf` allocates a fresh `RenderContext` per call, so
/// every other render test in this crate starts cold and would stay green
/// even if `EngineView::begin_frame` were deleted. This one drives
/// `prepare_frame`/`render_into` across two frames on one editor instead,
/// mirroring `run_loop`'s real reuse (`lifecycle.rs`), and asserts on the
/// glyph that moves.
#[test]
fn a_rendered_frames_entries_do_not_survive_it() {
    // Line 0 is "abcdef" (6 columns) in a 10-column content width; the
    // 6-column hint makes 12, wrapping it onto a second row — exactly
    // `content_pos_counts_an_inline_hints_extra_wrap_row`'s fixture.
    let text = BufferText::from("abcdef\ny\n");
    let sels = SelectionSet::single(Selection::collapsed(7));
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    ed.state.settings.scrolloff = 0;
    let pid = ed.state.focused_pane_id;
    ed.view.panes[pid].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Soft { width: 0 }),
        saved: None,
    });
    let hint_on = Rc::new(Cell::new(true));
    ed.view.panes[pid].providers.add_decoration_source(Box::new(
        InlineHint::new(0, 0, "HHHHHH").gated(Rc::clone(&hint_on)),
    ));

    let rect = Rect::new(0, 0, 10, 4);
    let mut grid = Grid::new(rect.width, rect.height);
    let mut ctx = RenderContext::new();

    ed.sync_viewport_dims(rect.width, rect.height);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    ed.render_into(rect, &mut grid, &mut ctx);
    assert_eq!(
        cell(&grid, 0, 2),
        "y",
        "sanity: hint present, its wrap row pushes y down to row 2"
    );

    hint_on.set(false);
    ed.sync_viewport_dims(rect.width, rect.height);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    ed.render_into(rect, &mut grid, &mut ctx);
    assert_eq!(
        cell(&grid, 0, 1),
        "y",
        "the second frame must re-format line 0 without the hint, not read \
         the first frame's entry — y must move back up to row 1"
    );
}

/// The point of the whole store: within one frame, the scroll step and the
/// render pass walk the same visible lines, and the second must find what the
/// first formatted.
///
/// Both passes reach the engine through their own `RowMap`, each built from
/// `EditorState::format_key` on the same pane — the scroll step through
/// `commands::pane_row_map`, the render pass through `frame.rs`'s
/// `resolve_pane_settings` — so the two share an entry by construction: one
/// composition, called twice, cannot itself disagree with itself. This test
/// still pins the outcome rather than the mechanism, so a future call site
/// that builds a `FormatKey` some other way (bypassing `format_key`) still
/// gets caught: any divergence rescopes the store between the passes and
/// quietly restores the double formatting this store exists to remove, with
/// every other test still green.
///
/// Wrapping, because that is where the cost is: under `WrapMode::None` the
/// scroll pass counts rows without formatting at all.
#[test]
fn the_two_frame_passes_format_each_visible_line_once() {
    const PROBED: usize = 0;
    let text: String = (0..6)
        .map(|i| format!("line{i} with enough text to wrap\n"))
        .collect();
    let sels = SelectionSet::single(Selection::collapsed(0));
    let mut ed = Editor::for_testing(Buffer::new(BufferText::from(text.as_str()), sels));
    let pid = ed.state.focused_pane_id;
    ed.state.settings.wrap_mode = hume_engine::pane::WrapMode::Soft { width: 0 };

    let formats = Rc::new(Cell::new(0));
    ed.view.panes[pid]
        .providers
        .add_decoration_source(Box::new(FormatProbe::new(PROBED, Rc::clone(&formats))));

    // Cursor two lines below the probed one, so the scroll step's own walk
    // crosses it on the way — otherwise the render pass would be the only
    // pass to reach it and one format would prove nothing.
    seek_to_line(&mut ed, PROBED + 2);

    // Tall enough for all six lines at two wrap rows each, plus the
    // statusline: the probed line has to still be on screen when the render
    // pass runs, or it never reaches it and the count below proves nothing.
    let rect = Rect::new(0, 0, 20, 14);
    let mut grid = Grid::new(rect.width, rect.height);
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(rect.width, rect.height);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let after_scroll = formats.get();
    ed.render_into(rect, &mut grid, &mut ctx);

    assert_eq!(
        after_scroll, 1,
        "sanity: the scroll step must have formatted the probed line, or the \
         render pass below has nothing to reuse and this proves nothing"
    );
    let top_row: String = (0..rect.width).map(|x| cell(&grid, x, 0)).collect();
    assert!(
        top_row.contains(&format!("line{PROBED}")),
        "sanity: the frame must actually have drawn the probed line, got {top_row:?}"
    );
    assert_eq!(
        formats.get(),
        after_scroll,
        "the render pass must reuse the scroll step's format, not resolve \
         settings that rescope the store and format the line a second time"
    );
}
