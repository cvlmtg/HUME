use super::*;
use crate::editor::settings::{Scope, THEME_KEY, all_setting_keys, setting_scopes};
use hume_core::BufferIntrinsicOption;

/// Run `:get <arg>` and return the status it reports.
fn get(ed: &mut Editor, arg: &str) -> String {
    ed.execute_typed(&format!("get {arg}"), None)
        .unwrap_or_else(|e| panic!(":get {arg}: {}", e.message()));
    status(ed)
}

/// The error `:get <arg>` fails with.
fn get_error(ed: &mut Editor, arg: &str) -> String {
    ed.execute_typed(&format!("get {arg}"), None)
        .expect_err(":get must fail")
        .message()
        .to_owned()
}

#[test]
fn get_global_reports_the_global_value() {
    let mut ed = editor_from("-[a]>b\n");
    insta::assert_snapshot!(get(&mut ed, "global undo-persist"), @"undo-persist=false");
    run_set(&mut ed, "global undo-persist=true").unwrap();
    insta::assert_snapshot!(get(&mut ed, "global undo-persist"), @"undo-persist=true");
}

#[test]
fn get_buffer_reports_the_override_or_else_the_global_value() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "global indent-width=2").unwrap();
    insta::assert_snapshot!(get(&mut ed, "buffer indent-width"), @"indent-width=2");
    run_set(&mut ed, "buffer indent-width=8").unwrap();
    insta::assert_snapshot!(get(&mut ed, "buffer indent-width"), @"indent-width=8");
    insta::assert_snapshot!(get(&mut ed, "global indent-width"), @"indent-width=2");
}

#[test]
fn get_pane_reports_the_pane_override_or_else_the_buffer_value() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "buffer wrap-mode=soft").unwrap();
    insta::assert_snapshot!(get(&mut ed, "pane wrap-mode"), @"wrap-mode=soft");
    run_set(&mut ed, "pane wrap-mode=word").unwrap();
    insta::assert_snapshot!(get(&mut ed, "pane wrap-mode"), @"wrap-mode=word");
    insta::assert_snapshot!(get(&mut ed, "buffer wrap-mode"), @"wrap-mode=soft");
}

#[test]
fn get_buffer_reports_the_buffer_language_and_line_ending() {
    let mut ed = editor_from("-[a]>b\n");
    insta::assert_snapshot!(get(&mut ed, "buffer language"), @"language=");
    insta::assert_snapshot!(get(&mut ed, "buffer line-ending"), @"line-ending=lf");
    run_set(&mut ed, "buffer line-ending=crlf").unwrap();
    insta::assert_snapshot!(get(&mut ed, "buffer line-ending"), @"line-ending=crlf");
}

#[test]
fn get_statusline_reports_the_sections_as_set_takes_them() {
    let mut ed = editor_from("-[a]>b\n");
    run_set(&mut ed, "global statusline=Mode,FilePath||Position").unwrap();
    insta::assert_snapshot!(get(&mut ed, "global statusline"), @"statusline=Mode,FilePath||Position");
}

#[test]
fn get_an_unknown_setting_errors() {
    let mut ed = editor_from("-[a]>b\n");
    insta::assert_snapshot!(get_error(&mut ed, "global foo"), @"unknown setting 'foo'");
}

#[test]
fn get_a_setting_in_a_scope_it_lacks_names_its_scopes() {
    let mut ed = editor_from("-[a]>b\n");
    insta::assert_snapshot!(
        get_error(&mut ed, "pane indent-width"),
        @"'indent-width' cannot be read with :get pane (valid scopes: global, buffer)"
    );
    insta::assert_snapshot!(
        get_error(&mut ed, "global language"),
        @"'language' is per-buffer: use ':get buffer language'"
    );
}

#[test]
fn get_without_a_key_shows_its_usage() {
    let mut ed = editor_from("-[a]>b\n");
    insta::assert_snapshot!(get_error(&mut ed, "global"), @"Usage: :get global|buffer|pane key");
}

/// `:get <arg>`'s value, after checking it reads back through `:set` as
/// the same value.
fn assert_round_trips(ed: &mut Editor, scope: impl std::fmt::Display, key: &str) -> String {
    let arg = format!("{scope} {key}");
    let reported = get(ed, &arg);
    let value = reported
        .strip_prefix(&format!("{key}="))
        .unwrap_or_else(|| panic!("{arg}: {reported:?}"))
        .to_owned();
    run_set(ed, &format!("{scope} {key}={value}"))
        .unwrap_or_else(|e| panic!(":set {arg}={value}: {}", e.message()));
    assert_eq!(get(ed, &arg), reported, "{arg}");
    value
}

/// Every value `:get` reports reads back through `:set` as the same value.
/// `theme` is left out: setting it loads the theme from a directory this
/// editor has none of. The buffer has a language, so `language` round-trips
/// a name and not the empty text.
#[test]
fn every_value_get_reports_sets_back_unchanged() {
    let mut ed = editor_from("-[a]>b\n");
    super::language::attach_host(&mut ed, "");
    ed.state
        .config
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .expect("a test identity registers");
    run_set(&mut ed, "buffer language=rust").unwrap();
    for key in all_setting_keys().iter().filter(|key| **key != THEME_KEY) {
        for scope in setting_scopes(key) {
            assert_round_trips(&mut ed, scope, key);
        }
    }
    let intrinsic: Vec<String> = BufferIntrinsicOption::ALL
        .into_iter()
        .map(|opt| assert_round_trips(&mut ed, Scope::Buffer, opt.name()))
        .collect();
    assert_eq!(intrinsic, ["rust", "lf"]);
    assert!(
        all_setting_keys()
            .iter()
            .any(|key| setting_scopes(key).contains(&Scope::Pane)),
        "the pane scope is covered"
    );
}
