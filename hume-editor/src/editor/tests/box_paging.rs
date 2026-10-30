//! Ctrl-d/Ctrl-u on a cursor-anchored popup opened since the last frame, as
//! in a macro, which runs no frame between its keys.

use super::*;

/// `show-popup!` commands for a 1-line and a 30-line cursor-anchored popup.
fn popup_commands() -> String {
    let long = popup_lines(30);
    format!(
        r#"(define-typed-command! "short" "" (lambda (pane) (show-popup! pane "one" #:kind 'scrollable)))
           (define-typed-command! "long" "" (lambda (pane) (show-popup! pane "{long}" #:kind 'scrollable)))"#
    )
}

/// The open popup's scroll, then the inner height the next frame paints it
/// with.
fn scroll_and_painted_height(ed: &mut Editor) -> (usize, usize) {
    let scroll = ed
        .state
        .input
        .popup()
        .expect("Ctrl-d scrolls a popup taller than its box")
        .scroll;
    frame(ed, 80, 24);
    let inner = ed
        .state
        .views
        .popup
        .read()
        .as_ref()
        .expect("the popup paints")
        .inner_height();
    (scroll, inner)
}

#[test]
fn half_page_on_a_cursor_popup_before_its_first_frame_scrolls_it() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, _f) = tall_file();
    let mut ed = open_headless(Some(path));
    run(&mut ed, tmp.path(), &popup_commands());
    frame(&mut ed, 80, 24);

    type_cmd(&mut ed, ":long");
    ed.feed_key(key_ctrl('d'));

    let (scroll, inner) = scroll_and_painted_height(&mut ed);
    assert!(inner >= 2, "setup: the box shows at least two lines");
    assert_eq!(scroll, inner / 2);
}

#[test]
fn half_page_on_a_cursor_popup_measures_it_not_the_last_painted_one() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, _f) = tall_file();
    let mut ed = open_headless(Some(path));
    run(&mut ed, tmp.path(), &popup_commands());
    type_cmd(&mut ed, ":short");
    frame(&mut ed, 80, 24);

    type_cmd(&mut ed, ":long");
    ed.feed_key(key_ctrl('d'));

    let (scroll, inner) = scroll_and_painted_height(&mut ed);
    assert!(inner >= 2, "setup: the box shows at least two lines");
    assert_eq!(scroll, inner / 2);
}

#[test]
fn half_page_on_a_cursor_popup_opened_with_a_drawer_measures_the_shorter_pane() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, _f) = tall_file();
    let mut ed = open_headless(Some(path));
    let long = popup_lines(30);
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-typed-command! "go" "" (lambda (pane)
                 (begin
                   (show-drawer-list! pane (map (lambda (row) (cons row row)) (list "a" "b" "c" "d" "e")) (lambda (row tok) (begin)))
                   (show-popup! pane "{long}" #:kind 'scrollable))))"#
        ),
    );
    frame(&mut ed, 80, 24);

    type_cmd(&mut ed, ":go");
    ed.feed_key(key_ctrl('d'));

    // The five-item drawer takes 6 rows, leaving the pane 17: the box is a
    // third of that, 5 rows, 3 of them inside the frame.
    let (scroll, inner) = scroll_and_painted_height(&mut ed);
    assert_eq!(inner, 3, "setup: the drawer shortens the box");
    assert_eq!(scroll, inner / 2);
}

#[test]
fn page_down_in_a_picker_that_evicted_a_docked_popup_pages_by_the_full_pane() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, _f) = tall_file();
    let mut ed = open_headless(Some(path));
    let long = popup_lines(30);
    let items = (0..50)
        .map(|i| format!(r#"(cons "item{i}" {i})"#))
        .collect::<Vec<_>>()
        .join(" ");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-typed-command! "dock" "" (lambda (pane) (show-popup! pane "{long}" #:anchor 'bottom)))
               (define-command! "pick" "" (lambda (pane) (picker! pane (list {items}) (hash "pagedown" (lambda (token item) (picker-move! token 1 #:by 'page))))))
               (bind-key! 'normal "Z" "pick")"#
        ),
    );
    type_cmd(&mut ed, ":dock");
    frame(&mut ed, 80, 24);
    assert!(
        ed.state.views.popup_band.read().is_some(),
        "setup: the popup is docked"
    );

    ed.feed_key(key('Z'));
    ed.feed_key(key_pagedown());

    frame(&mut ed, 80, 24);
    let geo = hume_ui::picker_panel::panel_geometry(ed.screen_layout().pane_area())
        .expect("viable geometry at 80x24");
    assert_eq!(ed.state.input.picker().unwrap().selected(), geo.list_rows);
}
