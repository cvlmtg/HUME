//! A bottom band (drawer, docked popup) that opens or closes between frames
//! changes the pane's height for every reader that runs before the next
//! frame.

use super::*;

use hume_grid::Rect;
use hume_rope::line::ContentLine;

/// A one-item drawer at 80x24 is one row plus one top frame row, so the pane
/// keeps 24 - 1 (statusline) - 2 = 21 rows.
const PANE_ROWS_UNDER_A_ONE_ITEM_DRAWER: usize = 21;

#[test]
fn centring_in_the_steel_call_that_opened_a_drawer_counts_the_drawer() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, _f) = tall_file();
    let mut ed = open_headless(Some(path));
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda (pane)
             (begin
               (show-drawer-list! pane (list "a") (lambda (idx tok) (begin)))
               (call! "center-view-on-cursor" pane))))"#,
    );
    ed.execute_typed("goto", Some("150")).unwrap();
    frame(&mut ed, 80, 24);

    type_cmd(&mut ed, ":go");

    // The centre row is 21/2 = 10 and the cursor is on 0-based line 149, so
    // the top line is 139.
    assert_eq!(ed.viewport().top().line, ContentLine::new(139));
}

#[test]
fn goto_location_in_the_steel_call_that_opened_a_drawer_centres_against_the_drawer() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, _f) = tall_file();
    let mut ed = open_headless(Some(path));
    // Lines 0-9 take 2 chars each, 10-99 take 3, and 100 on take 4, so line
    // 149 starts at 20 + 270 + 49 * 4 = 486.
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda (pane)
             (begin
               (show-drawer-list! pane (list "a") (lambda (idx tok) (begin)))
               (goto-location! pane (hash 'target pane 'char-offset 486)))))"#,
    );
    frame(&mut ed, 80, 24);

    type_cmd(&mut ed, ":go");

    // The centre row is 21/2 = 10 and the cursor is on 0-based line 149, so
    // the top line is 139.
    assert_eq!(ed.viewport().top().line, ContentLine::new(139));
}

#[test]
fn half_page_on_the_key_that_retires_a_docked_popup_scrolls_by_the_full_pane() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, _f) = tall_file();
    let mut ed = open_headless(Some(path));
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda (pane) (show-popup! pane "hello" #:kind 'scrollable #:anchor 'bottom)))"#,
    );
    type_cmd(&mut ed, ":go");
    frame(&mut ed, 80, 25);
    assert!(
        ed.state.views.popup_band.read().is_some(),
        "sanity: the popup is docked"
    );

    ed.feed_key(key_ctrl('d'));

    assert!(
        ed.state.input.popup().is_none(),
        "sanity: Ctrl-d retires a popup with nothing to scroll"
    );
    // The band is one line plus two border rows, leaving 25 - 1 - 3 = 21 rows
    // while it shows. Once it is gone the pane has 24 rows and half a page is
    // 12 lines.
    assert_eq!(ed.viewport().top().line, ContentLine::new(12));
}

/// A `go` command that docks a scrollable 30-line popup.
fn tall_docked_popup_command() -> String {
    let text = popup_lines(30);
    format!(
        r#"(define-typed-command! "go" "" (lambda (pane) (show-popup! pane "{text}" #:kind 'scrollable #:anchor 'bottom)))"#
    )
}

#[test]
fn half_page_on_a_docked_popup_before_its_first_frame_scrolls_it() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, _f) = tall_file();
    let mut ed = open_headless(Some(path));
    run(&mut ed, tmp.path(), &tall_docked_popup_command());
    frame(&mut ed, 80, 24);

    type_cmd(&mut ed, ":go");
    ed.feed_key(key_ctrl('d'));

    // The band's ceiling at 24 rows is 8, two of them frame rows, so 30 lines
    // show 6 at a time and half a page is 3.
    let popup = ed
        .state
        .input
        .popup()
        .expect("Ctrl-d scrolls a popup taller than its band");
    assert_eq!(popup.scroll, 3);
}

#[test]
fn half_page_on_a_docked_popup_in_a_headless_run_scrolls_it() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, _f) = tall_file();
    let mut ed = open_headless(Some(path));
    run(&mut ed, tmp.path(), &tall_docked_popup_command());

    // No frame, as in a headless run.
    type_cmd(&mut ed, ":go");
    ed.feed_key(key_ctrl('d'));

    // The band's ceiling on the headless 80x25 terminal is 8 rows, two of
    // them frame rows, so 30 lines show 6 at a time and half a page is 3.
    let popup = ed
        .state
        .input
        .popup()
        .expect("Ctrl-d scrolls a popup taller than its band");
    assert_eq!(popup.scroll, 3);
}

#[test]
fn paging_in_a_headless_run_with_two_tabs_counts_the_tab_bar() {
    let (path, _f) = tall_file();
    let mut ed = open_headless(Some(path));
    ed.execute_typed("tabnew", None).unwrap();

    ed.feed_key(key_pagedown());

    // Two tabs show the bar under the `dynamic` default, leaving the
    // headless 80x25 terminal's pane 25 - 1 - 1 = 23 rows, so a page lands
    // the top line on 23.
    assert_eq!(ed.viewport().top().line, ContentLine::new(23));
}

#[test]
fn viewport_range_in_the_steel_call_that_opened_a_drawer_counts_the_drawer() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, _f) = tall_file();
    let mut ed = open_headless(Some(path));
    frame(&mut ed, 80, 24);

    let end = log_probe(
        &mut ed,
        tmp.path(),
        r#"(begin
             (show-drawer-list! bid (list "a") (lambda (idx tok) (begin)))
             (to-string (hash-ref (viewport-range bid) 'end-line)))"#,
    );

    // The viewport starts at line 0, so its end is the pane's row count.
    assert_eq!(end, PANE_ROWS_UNDER_A_ONE_ITEM_DRAWER.to_string());
}

#[test]
fn a_viewport_change_queued_after_a_drawer_opens_counts_the_drawer() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, _f) = tall_file();
    let mut ed = open_headless(Some(path));
    run(
        &mut ed,
        tmp.path(),
        r#"(define *end* #f)
           (register-hook! 'on-viewport-change
             (lambda (pane range) (set! *end* (hash-ref range 'end-line))))"#,
    );
    seed_frame(&mut ed, 80, 24);

    // No frame between `:ls` opening the drawer and the queueing, so the
    // payload's size comes from the layout the queueing derives.
    ed.execute_typed("ls", None).unwrap();
    ed.queue_viewport_change(ed.view.focused_pane());
    ed.settle();

    let host = ed.scripting.take().unwrap();
    let fired = run_probe(
        &mut ed,
        host,
        tmp.path(),
        &format!("(equal? *end* {PANE_ROWS_UNDER_A_ONE_ITEM_DRAWER})"),
    );
    // `:ls` with one buffer is one row, the same height as a one-item drawer.
    assert!(
        fired,
        "the payload's end line must be the pane's {PANE_ROWS_UNDER_A_ONE_ITEM_DRAWER} rows under the drawer"
    );
}

#[test]
fn paging_in_the_steel_call_that_showed_the_tab_bar_counts_the_tab_bar() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, _f) = tall_file();
    let mut ed = open_headless(Some(path));
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda (pane)
             (begin
               (set-option! "tabline" "always")
               (call! "page-down" pane))))"#,
    );
    frame(&mut ed, 80, 24);

    type_cmd(&mut ed, ":go");

    // The pane keeps 24 - 1 (tab bar) - 1 (statusline) = 22 rows, and a page
    // from the top lands the top line on 22.
    assert_eq!(ed.viewport().top().line, ContentLine::new(22));
}

#[test]
fn centring_in_the_steel_call_that_grew_a_drawer_counts_the_new_rows() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, _f) = tall_file();
    let mut ed = open_headless(Some(path));
    run(
        &mut ed,
        tmp.path(),
        r#"(define *tok* #f)
           (define-typed-command! "open" "" (lambda (pane)
             (set! *tok* (show-drawer-list! pane (list "a") (lambda (idx tok) (begin))))))
           (define-typed-command! "grow" "" (lambda (pane)
             (begin
               (update-drawer-list! *tok* (list "a" "b" "c") (lambda (idx tok) (begin)) 0)
               (call! "center-view-on-cursor" pane))))"#,
    );
    ed.execute_typed("goto", Some("150")).unwrap();
    type_cmd(&mut ed, ":open");
    frame(&mut ed, 80, 24);

    type_cmd(&mut ed, ":grow");

    // Three items are three rows plus one top frame row, leaving
    // 24 - 1 - 4 = 19 rows: the centre row is 9 and the top line
    // 149 - 9 = 140.
    assert_eq!(ed.viewport().top().line, ContentLine::new(140));
}

#[test]
fn a_typed_split_after_a_drawer_opens_counts_the_drawer() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, _f) = tall_file();
    let mut ed = open_headless(Some(path));
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "open" "" (lambda (pane)
             (show-drawer-list! pane (list "a" "b" "c") (lambda (idx tok) (begin)))))"#,
    );
    frame(&mut ed, 80, 10);

    ed.execute_typed("open", None).unwrap();
    let err = ed
        .execute_typed("split", None)
        .expect_err("the drawer leaves too few rows to split");

    // The pane has 10 - 1 = 9 rows, enough for two 3-row panes and a seam.
    // The drawer takes the band's 35% ceiling of 3 rows, leaving 6.
    assert_eq!(err.message(), "pane too small to split");
}

#[test]
fn centring_after_buffers_open_under_ls_counts_its_new_rows() {
    let (path, _f) = tall_file();
    let extra: Vec<_> = (0..3)
        .map(|_| tempfile::NamedTempFile::new().unwrap())
        .collect();
    let mut ed = open_headless(Some(path));
    ed.execute_typed("goto", Some("150")).unwrap();
    ed.execute_typed("ls", None).unwrap();
    frame(&mut ed, 80, 24);

    for f in &extra {
        ed.open_extra_file(&hume_platform::path::AbsolutePath::rooted(
            &std::fs::canonicalize(f.path()).unwrap(),
        ))
        .expect("the extra file opens");
    }
    ed.feed_key(key('z'));
    ed.feed_key(key('z'));

    // Four buffers are four `:ls` rows plus one top frame row, leaving
    // 24 - 1 - 5 = 18 rows: the centre row is 9 and the top line
    // 149 - 9 = 140.
    assert_eq!(ed.viewport().top().line, ContentLine::new(140));
}

/// Opens a two-item drawer, then docks a three-line popup, on a 40x16
/// terminal.
fn open_both_bands(ed: &mut Editor, tmp: &std::path::Path) {
    let text = popup_lines(3);
    run(
        ed,
        tmp,
        &format!(
            r#"(define-typed-command! "go" "" (lambda (pane)
                 (begin
                   (show-drawer-list! pane (list "first" "second") (lambda (idx tok) (begin)))
                   (show-popup! pane "{text}" #:anchor 'bottom))))"#
        ),
    );
    frame(ed, 40, 16);
    type_cmd(ed, ":go");
    frame(ed, 40, 16);
}

#[test]
fn a_drawer_and_a_docked_popup_both_take_their_rows_from_the_pane() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    open_both_bands(&mut ed, tmp.path());

    assert!(
        ed.state.input.drawer().is_some(),
        "setup: the drawer is open"
    );
    assert!(
        ed.state.views.popup_band.read().is_some(),
        "setup: the popup is docked"
    );
    // The drawer is two rows plus its top frame row and the popup three
    // lines plus two frame rows, leaving 16 - 1 - 3 - 5 = 7 rows.
    assert_eq!(ed.pane_size(ed.view.focused_pane()).height(), 7);
}

#[test]
fn a_docked_popup_stacks_above_the_drawer() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    ed.view.theme = crate::testing::build_snapshot_theme();
    open_both_bands(&mut ed, tmp.path());

    insta::assert_snapshot!(render_snapshot::render_to_styled_string(
        &mut ed,
        Rect::new(0, 0, 40, 16)
    ));
}

#[test]
fn a_wheel_notch_that_retires_a_docked_popup_stops_at_the_full_pane_s_end_bound() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, _f) = tall_file();
    let mut ed = open_headless(Some(path));
    run(&mut ed, tmp.path(), &tall_docked_popup_command());
    ed.execute_typed("goto", Some("191")).unwrap();
    type_cmd(&mut ed, ":go");
    frame(&mut ed, 80, 24);
    assert_eq!(
        ed.viewport().top().line,
        ContentLine::new(179),
        "setup: the cursor on line 190 sits at the bottom margin of the \
         15-row pane above the 8-row band"
    );

    ed.handle_input(mouse_wheel_at(5, 5, true));

    // The notch retires the popup, so it scrolls the 23-row pane the next
    // frame draws: with a 3-row margin its top stops at 199 - (23 - 3 - 1) =
    // 180, not at the 182 three lines further would reach.
    assert!(
        ed.state.input.popup().is_none(),
        "the notch retires the popup"
    );
    assert_eq!(ed.viewport().top().line, ContentLine::new(180));
}
