// Lazy drawer rows: `show-drawer-list!`/`update-drawer-list!` with
// `#:render`. The drawer holds opaque keys and asks the render proc for the
// rows of the visible window only, once per row, before the frame is drawn.

use std::path::Path;

use super::*;

const WIDTH: u16 = 40;
const HEIGHT: u16 = 12;

/// Every render call is appended to `requests` as `start:count;`. Rows read
/// `row <key>`. `on-select` appends its argument to `selections`, marked `?` when its token is not the drawer's. `:requests`,
/// `:selections` and `:tab-width` show a value in the status line.
const PRELUDE: &str = r##"
(define requests "")
(define selections "")
(define drawer #f)
(define (render start keys)
  (set! requests (string-append requests (number->string start) ":"
                                (number->string (length keys)) ";"))
  (map (lambda (k) (string-append "row " (number->string k))) keys))
(define (on-select i token)
  (set! selections (string-append selections (if i (number->string i) "#f")
                                  (if (equal? token drawer) "" "?") ";")))
(define-typed-command! "requests" "" (lambda (pane) (log! 'info requests)))
(define-typed-command! "selections" "" (lambda (pane) (log! 'info selections)))
(define-typed-command! "tab-width" "" (lambda (pane)
  (log! 'info (to-string (get-buffer-option pane "tab-width")))))
"##;

/// Defines `:go`, which opens a drawer over `keys` with `render-proc`, and
/// runs it; `:swap-big` and `:swap-small` replace the keys.
fn open(ed: &mut Editor, tmp: &Path, keys: &str, render_proc: &str) {
    run(
        ed,
        tmp,
        &format!(
            r#"{PRELUDE}
(define-typed-command! "go" "" (lambda (pane)
  (set! drawer (show-drawer-list! pane {keys} on-select #:render {render_proc}))))
(define (swap keys) (update-drawer-list! drawer keys on-select 0 #:render render))
(define-typed-command! "swap-big" "" (lambda (pane) (swap (range 100 130))))
(define-typed-command! "swap-small" "" (lambda (pane) (swap (range 200 204))))"#
        ),
    );
    type_cmd(ed, ":go");
}

fn visible(len: usize) -> usize {
    hume_ui::drawer::visible_rows(
        len,
        hume_engine::pipeline::EngineView::bottom_band_max(HEIGHT),
    )
}

/// The drawer view's rows: `None` for a row not rendered yet.
fn view_rows(ed: &Editor) -> Vec<Option<String>> {
    let guard = ed.state.views.drawer.read();
    guard.as_ref().expect("drawer must be open").rows.to_vec()
}

/// The status line after running the typed command `name`.
fn shown(ed: &mut Editor, name: &str) -> String {
    type_cmd(ed, &format!(":{name}"));
    ed.state.status_msg.clone().unwrap()
}

/// `(start, count)` per render call, in call order.
fn requests(ed: &mut Editor) -> Vec<(usize, usize)> {
    shown(ed, "requests")
        .split(';')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let (start, count) = s.split_once(':').unwrap();
            (start.parse().unwrap(), count.parse().unwrap())
        })
        .collect()
}

fn row(key: usize) -> Option<String> {
    Some(format!("row {key}"))
}

#[test]
fn only_the_visible_window_is_rendered() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(&mut ed, tmp.path(), "(range 0 30)", "render");
    frame(&mut ed, WIDTH, HEIGHT);

    let v = visible(30);
    assert!(v < 30, "setup: the band cannot show every row");
    assert_eq!(requests(&mut ed), vec![(0, v)]);
    let rows = view_rows(&ed);
    assert_eq!(rows.len(), 30);
    assert_eq!(rows[..v], (0..v).map(row).collect::<Vec<_>>()[..]);
    assert!(rows[v..].iter().all(Option::is_none));
}

#[test]
fn scrolling_renders_only_rows_not_rendered_before() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(&mut ed, tmp.path(), "(range 0 30)", "render");
    frame(&mut ed, WIDTH, HEIGHT);

    for _ in 0..30 {
        ed.feed_key(key_ctrl('d'));
        frame(&mut ed, WIDTH, HEIGHT);
    }
    let scroll = ed.state.input.drawer().unwrap().scroll;
    assert!(scroll > 0, "setup: the window moved");

    let mut next = 0;
    for (start, count) in requests(&mut ed) {
        assert_eq!(start, next, "each request starts where the last ended");
        next = start + count;
    }
    assert_eq!(next, scroll + visible(30));
    let rows = view_rows(&ed);
    assert_eq!(
        rows[scroll..next],
        (scroll..next).map(row).collect::<Vec<_>>()[..]
    );
}

#[test]
fn new_keys_render_the_window_again() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(&mut ed, tmp.path(), "(range 0 30)", "render");
    frame(&mut ed, WIDTH, HEIGHT);

    type_cmd(&mut ed, ":swap-big");
    frame(&mut ed, WIDTH, HEIGHT);

    let v = visible(30);
    assert_eq!(requests(&mut ed), vec![(0, v), (0, v)]);
    assert_eq!(
        view_rows(&ed)[..v],
        (100..100 + v).map(row).collect::<Vec<_>>()[..]
    );
}

#[test]
fn a_shrunk_list_shows_rendered_rows_in_the_same_frame() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(&mut ed, tmp.path(), "(range 0 30)", "render");
    frame(&mut ed, WIDTH, HEIGHT);
    for _ in 0..30 {
        ed.feed_key(key_ctrl('d'));
    }
    frame(&mut ed, WIDTH, HEIGHT);

    type_cmd(&mut ed, ":swap-small");
    frame(&mut ed, WIDTH, HEIGHT);

    let v = visible(4);
    assert_eq!(ed.state.input.drawer().unwrap().scroll, 0);
    assert_eq!(
        view_rows(&ed)[..v],
        (200..200 + v).map(row).collect::<Vec<_>>()[..]
    );
}

#[test]
fn a_taller_terminal_shows_rendered_rows_in_the_same_frame() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(&mut ed, tmp.path(), "(range 0 30)", "render");
    frame(&mut ed, WIDTH, HEIGHT);

    frame(&mut ed, WIDTH, HEIGHT * 3);

    let taller = hume_ui::drawer::visible_rows(
        30,
        hume_engine::pipeline::EngineView::bottom_band_max(HEIGHT * 3),
    );
    assert!(taller > visible(30), "setup: the band grew");
    assert_eq!(
        view_rows(&ed)[..taller],
        (0..taller).map(row).collect::<Vec<_>>()[..]
    );
}

#[test]
fn eager_string_rows_need_no_render_proc() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda (pane)
             (show-drawer-list! pane (list "a" "b") (lambda (i tok) (begin)))))"#,
    );
    type_cmd(&mut ed, ":go");

    assert_eq!(
        view_rows(&ed),
        vec![Some("a".to_string()), Some("b".to_string())]
    );
}

/// A failing render closes the drawer, delivers `#f` to `on-select`, and
/// logs the failure.
fn assert_render_failure_closes(ed: &mut Editor) {
    assert!(ed.state.input.drawer().is_none(), "the drawer closed");
    assert_eq!(shown(ed, "selections"), "#f;");
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.text.contains("drawer render error")),
        "the failure is logged"
    );
}

#[test]
fn a_raising_render_proc_closes_the_drawer() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(
        &mut ed,
        tmp.path(),
        "(range 0 30)",
        r#"(lambda (start keys) (error "boom"))"#,
    );
    frame(&mut ed, WIDTH, HEIGHT);
    frame(&mut ed, WIDTH, HEIGHT);

    assert_render_failure_closes(&mut ed);
}

#[test]
fn a_render_proc_returning_too_few_rows_closes_the_drawer() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(
        &mut ed,
        tmp.path(),
        "(range 0 30)",
        r#"(lambda (start keys) (list "only one"))"#,
    );
    frame(&mut ed, WIDTH, HEIGHT);
    frame(&mut ed, WIDTH, HEIGHT);

    assert_render_failure_closes(&mut ed);
}

#[test]
fn a_render_proc_returning_a_non_string_closes_the_drawer() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(
        &mut ed,
        tmp.path(),
        "(range 0 30)",
        "(lambda (start keys) keys)",
    );
    frame(&mut ed, WIDTH, HEIGHT);
    frame(&mut ed, WIDTH, HEIGHT);

    assert_render_failure_closes(&mut ed);
}

/// The render proc reads the editor but cannot change it: a write is
/// refused even when the proc catches the error, and the drawer closes.
#[test]
fn a_render_proc_that_changes_the_editor_closes_the_drawer() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(
        &mut ed,
        tmp.path(),
        "(range 0 30)",
        r#"(lambda (start keys)
             (with-handler (lambda (e) (begin))
                           (set-buffer-option! (focused-pane) "tab-width" "8"))
             (map (lambda (k) "row") keys))"#,
    );
    frame(&mut ed, WIDTH, HEIGHT);
    frame(&mut ed, WIDTH, HEIGHT);

    assert_render_failure_closes(&mut ed);
    assert_eq!(
        shown(&mut ed, "tab-width"),
        "4",
        "the default tab width is untouched"
    );
}
