// Lazy drawer rows: `show-drawer-list!`/`update-drawer-list!` with
// `#:render`. The drawer holds opaque keys and asks the render proc for the
// rows of the visible window it has not shown at the band's current width,
// once per row, before the frame is drawn.

use std::path::Path;

use super::*;

const WIDTH: u16 = 40;
const HEIGHT: u16 = 12;

/// The cells a drawer row gets on a [`WIDTH`]-wide terminal: the band spans
/// the terminal, less one frame cell on each side.
const ROW_WIDTH: usize = WIDTH as usize - 2;

/// Every render call is appended to `requests` as `count:budget;`, and every
/// key it renders to `rendered` as `key,`. Rows read `row <key>`.
/// `on-select` appends its argument to `selections`, marked `?` when its
/// token is not the drawer's. `:requests`, `:rendered`, `:selections` and
/// `:indent-width` show a value in the status line.
const PRELUDE: &str = r##"
(define requests "")
(define rendered "")
(define selections "")
(define drawer #f)
(define (render rows budget)
  (set! requests (string-append requests (number->string (length rows)) ":"
                                (number->string budget) ";"))
  (map (lambda (k)
         (set! rendered (string-append rendered (number->string k) ","))
         (string-append "row " (number->string k)))
       rows))
(define (on-select key token)
  (set! selections (string-append selections (if key (number->string key) "#f")
                                  (if (equal? token drawer) "" "?") ";")))
(define-typed-command! "requests" "" (lambda (pane) (log! 'info requests)))
(define-typed-command! "rendered" "" (lambda (pane) (log! 'info rendered)))
(define-typed-command! "selections" "" (lambda (pane) (log! 'info selections)))
(define-typed-command! "indent-width" "" (lambda (pane)
  (log! 'info (to-string (get-buffer-option pane "indent-width")))))
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
    hume_ui::drawer::band_visible_rows(len, HEIGHT)
}

/// The drawer view's visible window: `None` for a row not rendered yet.
fn view_rows(ed: &Editor) -> Vec<Option<String>> {
    let guard = ed.state.views.drawer.read();
    guard
        .as_ref()
        .expect("drawer must be open")
        .rows
        .iter()
        .map(|row| row.as_ref().map(|row| row.text().to_owned()))
        .collect()
}

/// The status line after running the typed command `name`.
fn shown(ed: &mut Editor, name: &str) -> String {
    type_cmd(ed, &format!(":{name}"));
    ed.state.status_msg.clone().unwrap()
}

/// `(count, budget)` per render call, in call order.
fn requests(ed: &mut Editor) -> Vec<(usize, usize)> {
    shown(ed, "requests")
        .split(';')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let (count, budget) = s.split_once(':').unwrap();
            (count.parse().unwrap(), budget.parse().unwrap())
        })
        .collect()
}

/// Every key the render proc was given, in call order.
fn rendered(ed: &mut Editor) -> Vec<usize> {
    shown(ed, "rendered")
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap())
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
    assert_eq!(requests(&mut ed), vec![(v, ROW_WIDTH)]);
    assert_eq!(view_rows(&ed), (0..v).map(row).collect::<Vec<_>>());
    let listed = ed.state.input.drawer().unwrap().rows.listed();
    assert_eq!(listed.len(), 30);
    assert!(listed[v..].iter().all(Option::is_none));
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

    let next = scroll + visible(30);
    assert_eq!(rendered(&mut ed), (0..next).collect::<Vec<_>>());
    assert_eq!(view_rows(&ed), (scroll..next).map(row).collect::<Vec<_>>());
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
    assert_eq!(requests(&mut ed), vec![(v, ROW_WIDTH), (v, ROW_WIDTH)]);
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

    let taller = hume_ui::drawer::band_visible_rows(30, HEIGHT * 3);
    assert!(taller > visible(30), "setup: the band grew");
    assert_eq!(
        view_rows(&ed)[..taller],
        (0..taller).map(row).collect::<Vec<_>>()[..]
    );
    assert_eq!(rendered(&mut ed), (0..taller).collect::<Vec<_>>());
}

#[test]
fn a_width_change_renders_the_visible_rows_again_at_the_new_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(&mut ed, tmp.path(), "(range 0 30)", "render");
    frame(&mut ed, WIDTH, HEIGHT);

    frame(&mut ed, WIDTH + 10, HEIGHT);
    frame(&mut ed, WIDTH + 10, HEIGHT);

    let v = visible(30);
    assert_eq!(requests(&mut ed), vec![(v, ROW_WIDTH), (v, ROW_WIDTH + 10)]);
    assert_eq!(view_rows(&ed)[..v], (0..v).map(row).collect::<Vec<_>>()[..]);
}

/// Opens a drawer over one row `text`, given up front, with `extra`
/// keywords, and shows it on a terminal 12 cells wide: 10 for the row.
fn shown_long_row(text: &str, render: Option<&str>, extra: &str) -> String {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    let items = match render {
        Some(_) => "(list 0)".to_string(),
        None => format!("(list (cons {text:?} 0))"),
    };
    let render = render.map_or(String::new(), |proc| format!("#:render {proc}"));
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-typed-command! "go" "" (lambda (pane)
                 (show-drawer-list! pane {items} (lambda (row tok) (begin)) {render} {extra})))"#
        ),
    );
    type_cmd(&mut ed, ":go");
    frame(&mut ed, 12, HEIGHT);
    let rows = view_rows(&ed);
    rows[0].clone().expect("the row is shown")
}

#[test]
fn a_row_wider_than_the_budget_is_cut_at_the_truncate_end() {
    let long = "0123456789abcdef";
    insta::assert_snapshot!(shown_long_row(long, None, ""), @"012345678…");
    insta::assert_snapshot!(shown_long_row(long, None, "#:truncate 'head"), @"…789abcdef");
}

#[test]
fn a_rendered_row_wider_than_the_budget_is_cut_at_the_truncate_end() {
    let render = r#"(lambda (rows budget) (map (lambda (k) "0123456789abcdef") rows))"#;
    insta::assert_snapshot!(shown_long_row("", Some(render), ""), @"012345678…");
    insta::assert_snapshot!(
        shown_long_row("", Some(render), "#:truncate 'head"),
        @"…789abcdef"
    );
}

/// Enter hands `on-select` the selected row's key, not its index.
#[test]
fn enter_passes_the_selected_rows_key() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(&mut ed, tmp.path(), "(range 100 130)", "render");
    frame(&mut ed, WIDTH, HEIGHT);
    ed.feed_key(key_ctrl('d'));
    ed.feed_key(key_ctrl('d'));
    ed.feed_key(key_enter());
    ed.settle();

    assert_eq!(shown(&mut ed, "selections"), "102;");
}

#[test]
fn eager_string_rows_need_no_render_proc() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda (pane)
             (show-drawer-list! pane (list (cons "a" 'a) (cons "b" 'b)) (lambda (row tok) (begin)))))"#,
    );
    type_cmd(&mut ed, ":go");
    frame(&mut ed, WIDTH, HEIGHT);

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
        r#"(lambda (rows budget) (error "boom"))"#,
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
        r#"(lambda (rows budget) (list "only one"))"#,
    );
    frame(&mut ed, WIDTH, HEIGHT);
    frame(&mut ed, WIDTH, HEIGHT);

    assert_render_failure_closes(&mut ed);
}

#[test]
fn a_render_with_no_scripting_host_closes_the_drawer() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(&mut ed, tmp.path(), "(range 0 30)", "render");
    ed.scripting = None;
    frame(&mut ed, WIDTH, HEIGHT);

    assert!(ed.state.input.drawer().is_none(), "the drawer closed");
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.text == "drawer render error: scripting is not available"),
        "the failure is logged"
    );
}

#[test]
fn a_render_proc_returning_numbers_closes_the_drawer() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(
        &mut ed,
        tmp.path(),
        "(range 0 30)",
        "(lambda (rows budget) rows)",
    );
    frame(&mut ed, WIDTH, HEIGHT);
    frame(&mut ed, WIDTH, HEIGHT);

    assert_render_failure_closes(&mut ed);
}

#[test]
fn a_render_proc_may_return_styled_rows() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(
        &mut ed,
        tmp.path(),
        "(range 0 3)",
        r#"(lambda (rows budget)
             (map (lambda (k)
                    (let ([text (string-append "row " (number->string k))])
                      (if (even? k)
                          (hash 'text text
                                'spans (list (hash 'start-char 0 'end-char 3 'scope "keyword"
                                                   'selected-scope "markup.bold"))
                                'selected-scope "diagnostic.warning.message")
                          text)))
                  rows))"#,
    );
    frame(&mut ed, WIDTH, HEIGHT);
    assert_eq!(visible(3), 3, "setup: the band shows every row");

    let styled: ExpectedRow = (
        Some("diagnostic.warning.message"),
        &[(0, 3, "keyword", Some("markup.bold"))],
    );
    assert_drawer_styles(&mut ed, &[styled, (None, &[]), styled]);
}

/// A render result that fails to become a row closes the drawer and logs
/// why.
fn assert_render_failure_names(ed: &mut Editor, needle: &str) {
    assert!(has_error(ed, needle), "errors: {:?}", error_log(ed));
    assert_render_failure_closes(ed);
}

/// The error names the failing row by its index in the drawer, not in the
/// rendered window.
#[test]
fn a_render_error_for_invalid_spans_names_the_row() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(
        &mut ed,
        tmp.path(),
        "(range 0 30)",
        r#"(lambda (rows budget)
             (map (lambda (k)
                    (if (= k 29)
                        (hash 'text "x" 'spans (list (hash 'start-char 0 'end-char 9 'scope "keyword")))
                        "ok"))
                  rows))"#,
    );
    assert!(
        visible(30) < 30,
        "setup: the last row renders after a scroll"
    );
    frame(&mut ed, WIDTH, HEIGHT);
    for _ in 0..30 {
        if ed.state.input.drawer().is_none() {
            break;
        }
        ed.feed_key(key_ctrl('d'));
        frame(&mut ed, WIDTH, HEIGHT);
    }
    frame(&mut ed, WIDTH, HEIGHT);

    assert_render_failure_names(
        &mut ed,
        "row 29 'spans 0: end 9 is past the text's char length 1",
    );
}

#[test]
fn a_render_proc_returning_a_row_with_an_unknown_key_closes_the_drawer() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(
        &mut ed,
        tmp.path(),
        "(range 0 30)",
        r#"(lambda (rows budget) (map (lambda (k) (hash 'text "x" 'spans '() 'bogus 1)) rows))"#,
    );
    frame(&mut ed, WIDTH, HEIGHT);
    frame(&mut ed, WIDTH, HEIGHT);

    assert_render_failure_names(&mut ed, "unknown key 'bogus");
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
        r#"(lambda (rows budget)
             (with-handler (lambda (e) (begin))
                           (set-buffer-option! (focused-pane) "indent-width" 8))
             (map (lambda (k) "row") rows))"#,
    );
    frame(&mut ed, WIDTH, HEIGHT);
    frame(&mut ed, WIDTH, HEIGHT);

    assert_render_failure_closes(&mut ed);
    assert_eq!(
        shown(&mut ed, "indent-width"),
        "4",
        "the default tab width is untouched"
    );
}

/// Opens a drawer with `#:truncate 'head`, replaces its row through
/// `update-drawer-list!` with no `#:truncate`, and shows the new row on a
/// terminal 12 cells wide: 10 for the row. With `render`, the rows are keys
/// it makes rows of; without, they are given up front.
fn updated_long_row(render: Option<&str>) -> String {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    let long = "fedcba9876543210";
    let (first, second, render) = match render {
        Some(proc) => (
            "(list 0)".to_string(),
            "(list 1)".to_string(),
            format!("#:render {proc}"),
        ),
        None => (
            "(list (cons \"start\" 0))".to_string(),
            format!("(list (cons {long:?} 1))"),
            String::new(),
        ),
    };
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define drawer #f)
(define-typed-command! "go" "" (lambda (pane)
  (set! drawer (show-drawer-list! pane {first} (lambda (row tok) (begin)) {render}
                                  #:truncate 'head))))
(define-typed-command! "swap" "" (lambda (pane)
  (update-drawer-list! drawer {second} (lambda (row tok) (begin)) 0 {render})))"#
        ),
    );
    type_cmd(&mut ed, ":go");
    frame(&mut ed, 12, HEIGHT);
    type_cmd(&mut ed, ":swap");
    frame(&mut ed, 12, HEIGHT);
    let rows = view_rows(&ed);
    rows[0].clone().expect("the row is shown")
}

#[test]
fn update_drawer_list_keeps_the_opening_truncate() {
    insta::assert_snapshot!(updated_long_row(None), @"…876543210");
    let render = r#"(lambda (rows budget) (map (lambda (k) "fedcba9876543210") rows))"#;
    insta::assert_snapshot!(updated_long_row(Some(render)), @"…876543210");
}

/// Where the open drawer stores the rows its render proc made, `None` for a
/// drawer with no render proc.
fn rendered_store(ed: &Editor) -> Option<*const ()> {
    ed.state
        .input
        .drawer()
        .expect("drawer must be open")
        .rows
        .rendered()
        .map(|rendered| rendered.store())
}

#[test]
fn a_rendered_drawer_allocates_its_rows_once() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(&mut ed, tmp.path(), "(range 0 30)", "render");
    let first = rendered_store(&ed).expect("a rendered drawer stores its rows");
    frame(&mut ed, WIDTH, HEIGHT);

    for _ in 0..30 {
        ed.feed_key(key_ctrl('d'));
        frame(&mut ed, WIDTH, HEIGHT);
    }
    let scrolled = requests(&mut ed).len();
    assert!(scrolled > 1, "setup: the scroll rendered rows");
    frame(&mut ed, WIDTH + 10, HEIGHT);

    assert_eq!(
        requests(&mut ed).len(),
        scrolled + 1,
        "setup: the width change rendered rows"
    );
    assert_eq!(rendered_store(&ed), Some(first));
}

#[test]
fn a_drawer_given_its_rows_stores_no_rendered_rows() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    open(
        &mut ed,
        tmp.path(),
        "(map (lambda (k) (cons (number->string k) k)) (range 0 30))",
        "#f",
    );
    frame(&mut ed, WIDTH, HEIGHT);
    ed.feed_key(key_ctrl('d'));
    frame(&mut ed, WIDTH, HEIGHT);

    assert_eq!(rendered_store(&ed), None);
}
