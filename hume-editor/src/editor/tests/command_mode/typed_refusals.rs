use super::*;
use crate::editor::registry::TypedBody;

/// Every native typed command that reads its argument.
const TAKES_ARG: &[&str] = &[
    "set",
    "split",
    "vsplit",
    "tabnew",
    "theme",
    "change-directory",
    "buffer",
    "goto",
    "sort",
    "earlier",
    "later",
    "write",
    "write-quit",
    "edit",
];

/// Every native typed command that cannot run without its argument, and the
/// usage its refusal names.
const REQUIRES_ARG: &[(&str, &str)] = &[
    ("set", "global|buffer|pane key=value"),
    ("buffer", "<name|#|index>"),
    ("goto", "<line>"),
];

/// Every native typed command that reads `!`.
const TAKES_FORCE: &[&str] = &[
    "quit",
    "quit-all",
    "write-all",
    "buffer-delete",
    "tabclose",
    "write",
    "write-quit",
    "edit",
];

fn native_typed_names(ed: &Editor) -> Vec<String> {
    let registry = &ed.state.config.registry;
    registry
        .typed_names()
        .filter(|name| {
            matches!(
                registry.get_typed(name).map(|tc| &tc.body),
                Some(TypedBody::Native(_))
            )
        })
        .map(str::to_owned)
        .collect()
}

fn assert_refused(ed: &mut Editor, line: &str, arg: Option<&str>, expected: &str) {
    let err = ed
        .execute_typed(line, arg)
        .expect_err(&format!("`:{line}` with {arg:?} must be refused"));
    assert_eq!(err.message(), expected);
    assert_eq!(err.severity(), Severity::Error);
    assert!(
        error_log(ed).iter().any(|e| e == expected),
        "`{expected}` must reach the message log"
    );
}

#[test]
fn every_native_typed_command_refuses_what_it_does_not_take() {
    let names = native_typed_names(&editor_from("-[h]>ello\n"));
    for listed in TAKES_ARG.iter().chain(TAKES_FORCE) {
        assert!(
            names.iter().any(|n| n == listed),
            "`:{listed}` is not a registered native typed command"
        );
    }
    for name in &names {
        if !TAKES_ARG.contains(&name.as_str()) {
            let mut ed = editor_from("-[h]>ello\n");
            assert_refused(
                &mut ed,
                name,
                Some("x"),
                &format!("`:{name}` takes no argument"),
            );
        }
        if !TAKES_FORCE.contains(&name.as_str()) {
            let mut ed = editor_from("-[h]>ello\n");
            assert_refused(
                &mut ed,
                &format!("{name}!"),
                None,
                &format!("`:{name}` takes no `!`"),
            );
        }
    }
}

#[test]
fn edit_bang_with_a_path_is_refused_and_opens_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("other.txt");
    std::fs::write(&path, "other\n").unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    assert_refused(
        &mut ed,
        "e!",
        Some(path.to_str().unwrap()),
        "`:edit` takes no `!` with an argument",
    );
    let canonical = AbsolutePath::canonicalize(&path).unwrap();
    assert!(ed.state.buffers.find_by_path(&canonical).is_none());
}

/// `:e!` refuses an argument before `#` expansion, which would fail first
/// with no alternate buffer.
#[test]
fn edit_bang_refuses_an_argument_before_it_is_expanded() {
    let mut ed = editor_from("-[h]>ello\n");
    assert_refused(
        &mut ed,
        "e!",
        Some("#"),
        "`:edit` takes no `!` with an argument",
    );
}

/// The refusal comes before `#` expansion, which would fail first with no
/// alternate buffer.
#[test]
fn an_untaken_argument_is_refused_before_it_is_expanded() {
    let mut ed = editor_from("-[h]>ello\n");
    assert_refused(
        &mut ed,
        "pwd",
        Some("#"),
        "`:print-working-directory` takes no argument",
    );
}

/// `:q !` parses the detached `!` as an argument.
#[test]
fn a_detached_bang_is_refused_as_an_argument() {
    let mut ed = editor_from("-[h]>ello\n");
    assert_refused(&mut ed, "q", Some("!"), "`:quit` takes no argument");
    assert!(!ed.state.should_quit);
}

#[test]
fn a_command_that_needs_its_argument_refuses_a_line_without_one() {
    for (name, usage) in REQUIRES_ARG {
        assert!(TAKES_ARG.contains(name), "`:{name}` must take an argument");
        let mut ed = editor_from("-[h]>ello\n");
        assert_refused(
            &mut ed,
            name,
            None,
            &format!("`:{name}` requires an argument: {usage}"),
        );
    }
}
