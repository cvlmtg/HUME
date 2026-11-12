use super::*;
use crate::editor::settings::{Scope, SettingKey, THEME, TableKey};

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
        @"'language' cannot be read with :get global (valid scopes: buffer)"
    );
}

#[test]
fn get_a_real_setting_in_an_unknown_scope_names_its_scopes() {
    let mut ed = editor_from("-[a]>b\n");
    insta::assert_snapshot!(
        get_error(&mut ed, "glob indent-width"),
        @"'indent-width' cannot be read with :get glob (valid scopes: global, buffer)"
    );
    insta::assert_snapshot!(
        get_error(&mut ed, "glob language"),
        @"'language' cannot be read with :get glob (valid scopes: buffer)"
    );
}

#[test]
fn get_without_a_key_shows_its_usage() {
    let mut ed = editor_from("-[a]>b\n");
    insta::assert_snapshot!(get_error(&mut ed, "global"), @"Usage: :get global|buffer|pane key");
}

#[test]
fn get_with_a_value_shows_its_usage() {
    let mut ed = editor_from("-[a]>b\n");
    insta::assert_snapshot!(get_error(&mut ed, "global theme=x"), @"Usage: :get global|buffer|pane key");
}

#[test]
fn typed_set_unknown_setting_errors() {
    let mut ed = editor_from("-[a]>b\n");
    let result = run_set(&mut ed, "global foo=bar");
    assert!(result.is_err(), "nonexistent setting must error");
    let msg = result.unwrap_err().message().to_owned();
    assert!(
        msg.contains("unknown setting") && msg.contains("foo"),
        "error should say the setting is unknown: {msg}"
    );
}

#[test]
fn typed_set_garbage_scope_on_real_key_errors() {
    let mut ed = editor_from("-[a]>b\n");
    let result = run_set(&mut ed, "bogus indent-width=2");
    assert!(result.is_err(), "garbage scope token must error");
    let msg = result.unwrap_err().message().to_owned();
    assert!(
        msg.contains("indent-width") && msg.contains("global") && msg.contains("buffer"),
        "error should name the key and its real valid scopes: {msg}"
    );
}

/// `complete_set` tolerates a stray double space before the key, so
/// `typed_set` must accept the same input on Enter. Otherwise Tab-completing
/// through a double space produces a command line that errors.
#[test]
fn typed_set_tolerates_double_space_before_key() {
    let mut ed = editor_from("-[a]>b\n");
    let result = run_set(&mut ed, "global  indent-width=2");
    assert!(
        result.is_ok(),
        "double space before key must not error: {:?}",
        result.err().map(|e| e.message().to_owned())
    );
    assert_eq!(ed.state.settings.indent_width, 2);
}

#[test]
fn typed_set_pane_unknown_setting_errors() {
    let mut ed = editor_from("-[a]>b\n");
    let result = run_set(&mut ed, "pane foo=bar");
    assert!(result.is_err(), "unknown setting must error");
    let msg = result.unwrap_err().message().to_owned();
    assert!(
        msg.contains("unknown setting") && msg.contains("foo"),
        "error should name the unknown key: {msg}"
    );
}

/// A real setting that just isn't pane-eligible (`indent-width` takes global
/// and buffer scope) must be rejected with its own valid scopes, not
/// accepted or confused with an unknown-setting error.
#[test]
fn typed_set_pane_ineligible_key_errors() {
    let mut ed = editor_from("-[a]>b\n");
    let result = run_set(&mut ed, "pane indent-width=2");
    assert!(result.is_err(), "pane-ineligible key must error");
    let msg = result.unwrap_err().message().to_owned();
    assert!(
        msg.contains("indent-width") && msg.contains("global") && msg.contains("buffer"),
        "error should name the key and its actual valid scopes: {msg}"
    );
}

#[test]
fn typed_set_pane_language_errors() {
    let mut ed = editor_from("-[a]>b\n");
    let err = run_set(&mut ed, "pane language=rust").expect_err("language takes no pane scope");
    insta::assert_snapshot!(
        err.message(),
        @"'language' cannot be set with :set pane (valid scopes: buffer)"
    );
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
    let mut intrinsic = Vec::new();
    for setting in
        SettingKey::all().filter(|setting| *setting != SettingKey::Table(TableKey::Global(THEME)))
    {
        for scope in setting.scopes() {
            let value = assert_round_trips(&mut ed, scope, setting.name());
            if let SettingKey::Intrinsic(_) = setting {
                intrinsic.push(value);
            }
        }
    }
    assert_eq!(intrinsic, ["rust", "lf"]);
    assert!(
        SettingKey::all().any(|setting| setting.scopes().any(|scope| scope == Scope::Pane)),
        "the pane scope is covered"
    );
}
