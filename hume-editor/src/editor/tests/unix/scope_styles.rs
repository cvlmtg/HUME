//! `set-scope-styles!` across a session: styles from `init.scm` and from a
//! plugin layered onto the themes the editor installs.

use super::*;

use super::super::before_save::{warning_texts, warnings};
use super::language_config::{stage, start};
use hume_engine::types::Scope;
use hume_grid::Rgb;

const SAND: &str = r##"
"ui.text" = { fg = "#010101" }
"keyword" = { fg = "#111111" }
"markup.heading.1" = { fg = "#121212" }
"##;
const OTHER: &str = r##""keyword" = { fg = "#222222" }"##;
const GOLDEN: &str = r##"
palette = { gold = "#ffd700" }
"keyword" = { fg = "#333333" }
"##;

const SCRIPT_RGB: Rgb = Rgb(0xab, 0xcd, 0xef);

/// A session's directories with `sand` and the other themes on disk, the
/// real prelude, and `plugin` as the user plugin `user/tp`.
fn session(init: &str, plugin: &str) -> (Editor, ReloadFixture) {
    session_with_themes(
        init,
        plugin,
        &[("sand", SAND), ("other", OTHER), ("golden", GOLDEN)],
    )
}

/// [`session`] with only `themes` on disk.
fn session_with_themes(
    init: &str,
    plugin: &str,
    themes: &[(&str, &str)],
) -> (Editor, ReloadFixture) {
    let fixture = stage("", &[], init, plugin);
    let theme_dir = fixture.runtime_dir.join("themes");
    std::fs::create_dir_all(&theme_dir).unwrap();
    for (name, body) in themes {
        std::fs::write(theme_dir.join(format!("{name}.toml")), body).unwrap();
    }
    start(fixture)
}

fn style(ed: &Editor, scope: &'static str) -> hume_engine::types::ResolvedStyle {
    ed.view.theme.resolve_by_name(Scope(scope))
}

#[test]
fn a_user_style_overrides_the_theme_and_survives_a_theme_switch() {
    let (mut ed, _fixture) = session(
        r##"(set-scope-styles! (list (hash 'scope "keyword" 'fg "#abcdef" 'modifiers '(bold))))"##,
        "",
    );
    assert_eq!(style(&ed, "keyword").fg, Some(SCRIPT_RGB));
    assert!(
        style(&ed, "keyword")
            .modifiers
            .contains(hume_grid::Modifiers::BOLD)
    );

    type_cmd(&mut ed, ":theme other");

    assert_eq!(ed.state.shown_theme, "other");
    assert_eq!(style(&ed, "keyword").fg, Some(SCRIPT_RGB));
}

#[test]
fn a_plugin_style_sits_beneath_the_theme_per_scope_name() {
    let (ed, _fixture) = session(
        r#"(load-plugin! "user/tp")"#,
        r##"(set-scope-styles! (list (hash 'scope "keyword" 'fg "#abcdef")
                                    (hash 'scope "markup.heading.1.line" 'bg "#abcdef")))"##,
    );
    assert_eq!(style(&ed, "keyword").fg, Some(Rgb(0x11, 0x11, 0x11)));
    let line = style(&ed, "markup.heading.1.line");
    assert_eq!((line.fg, line.bg), (None, Some(SCRIPT_RGB)));
    assert_eq!(
        style(&ed, "markup.heading.1.text").fg,
        Some(Rgb(0x12, 0x12, 0x12))
    );
}

#[test]
fn a_palette_name_set_before_its_theme_loads_resolves_without_a_warning() {
    let (ed, _fixture) = session(
        r##"(set-scope-styles! (list (hash 'scope "keyword" 'fg "gold")))
            (set-option! "theme" "golden")"##,
        "",
    );
    assert_eq!(style(&ed, "keyword").fg, Some(Rgb(0xff, 0xd7, 0x00)));
    assert_eq!(warning_texts(&ed), Vec::<String>::new());
}

#[test]
fn an_unresolved_color_name_warns_at_each_theme_install() {
    let (mut ed, _fixture) = session(
        r##"(set-scope-styles! (list (hash 'scope "keyword" 'fg "gold" 'bg "#abcdef")))"##,
        "",
    );
    assert_eq!(ed.state.shown_theme, "sand");
    assert_eq!(warnings(&ed), 1, "{:?}", warning_texts(&ed));
    let keyword = style(&ed, "keyword");
    assert_eq!((keyword.fg, keyword.bg), (None, Some(SCRIPT_RGB)));

    type_cmd(&mut ed, ":theme other");
    assert_eq!(warnings(&ed), 2, "{:?}", warning_texts(&ed));

    type_cmd(&mut ed, ":theme golden");
    assert_eq!(warnings(&ed), 2, "{:?}", warning_texts(&ed));
    assert_eq!(style(&ed, "keyword").fg, Some(Rgb(0xff, 0xd7, 0x00)));
}

#[test]
fn init_styles_reach_the_fallback_when_the_default_theme_is_missing() {
    let (ed, _fixture) = session_with_themes(
        r##"(set-scope-styles! (list (hash 'scope "keyword" 'fg "#abcdef")))"##,
        "",
        &[("other", OTHER)],
    );
    assert_eq!(ed.state.shown_theme, "");
    assert_eq!(style(&ed, "keyword").fg, Some(SCRIPT_RGB));
}

#[test]
fn init_styles_reach_the_fallback_when_the_configured_theme_fails() {
    let (ed, _fixture) = session(
        r##"(set-scope-styles! (list (hash 'scope "keyword" 'fg "#abcdef")))
            (with-handler (lambda (e) 0) (set-option! "theme" "missing"))"##,
        "",
    );
    assert_eq!(ed.state.shown_theme, "");
    assert_eq!(style(&ed, "keyword").fg, Some(SCRIPT_RGB));
}

#[test]
fn a_live_call_warns_only_for_its_own_entries() {
    let (mut ed, fixture) = session(
        r##"(set-scope-styles! (list (hash 'scope "keyword" 'fg "gold")))"##,
        "",
    );
    assert_eq!(warnings(&ed), 1, "{:?}", warning_texts(&ed));

    run(
        &mut ed,
        &fixture.config_dir,
        r##"(set-scope-styles! (list (hash 'scope "string" 'fg "#abcdef")))"##,
    );

    assert_eq!(warnings(&ed), 1, "{:?}", warning_texts(&ed));
    assert_eq!(style(&ed, "string").fg, Some(SCRIPT_RGB));
}

#[test]
fn a_call_warns_in_the_order_it_names_its_scopes() {
    let (mut ed, fixture) = session("", "");
    let scopes = ["zeta.x", "alpha.x", "mid.x", "beta.x", "omega.x"];
    let entries: String = scopes
        .iter()
        .map(|scope| format!(r#"(hash 'scope "{scope}" 'fg "gold")"#))
        .collect();

    run(
        &mut ed,
        &fixture.config_dir,
        &format!("(set-scope-styles! (list {entries}))"),
    );

    let warned: Vec<&str> = warning_texts(&ed)
        .iter()
        .filter_map(|text| scopes.iter().copied().find(|scope| text.contains(*scope)))
        .collect();
    assert_eq!(warned, scopes);
}

#[test]
fn a_write_after_startup_restyles_the_live_theme() {
    let (mut ed, fixture) = session("", "");
    assert_eq!(style(&ed, "keyword").fg, Some(Rgb(0x11, 0x11, 0x11)));

    run(
        &mut ed,
        &fixture.config_dir,
        r##"(set-scope-styles! (list (hash 'scope "keyword" 'fg "#abcdef")))"##,
    );

    assert_eq!(style(&ed, "keyword").fg, Some(SCRIPT_RGB));
}

#[test]
fn a_malformed_entry_refuses_the_whole_call() {
    let (mut ed, fixture) = session("", "");
    run(
        &mut ed,
        &fixture.config_dir,
        r##"(with-handler (lambda (e) 0)
              (set-scope-styles! (list (hash 'scope "keyword" 'fg "#abcdef")
                                       (hash 'scope "string" 'modifiers '(blod)))))"##,
    );
    assert_eq!(style(&ed, "keyword").fg, Some(Rgb(0x11, 0x11, 0x11)));
}

#[test]
fn reload_config_drops_styles_the_new_init_does_not_set() {
    let (mut ed, fixture) = session(
        r##"(set-scope-styles! (list (hash 'scope "keyword" 'fg "#abcdef")))"##,
        "",
    );
    assert_eq!(style(&ed, "keyword").fg, Some(SCRIPT_RGB));
    std::fs::write(fixture.config_dir.join("init.scm"), "").unwrap();

    type_cmd(&mut ed, ":reload-config");

    assert_eq!(style(&ed, "keyword").fg, Some(Rgb(0x11, 0x11, 0x11)));
}
