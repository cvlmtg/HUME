//! `set-scope-styles!` across a session: styles from `init.scm` and from a
//! plugin layered onto the themes the editor installs.

use super::*;

use super::super::scripting_grammar::prelude_source;
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
    let fixture = ReloadFixture::new(init);
    let plugin_dir = fixture.data_dir.join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), plugin).unwrap();
    let scheme_dir = fixture.runtime_dir.join("scheme");
    std::fs::create_dir_all(&scheme_dir).unwrap();
    std::fs::write(scheme_dir.join("prelude.scm"), prelude_source()).unwrap();
    std::fs::write(scheme_dir.join("languages.scm"), "").unwrap();
    let themes = fixture.runtime_dir.join("themes");
    std::fs::create_dir_all(&themes).unwrap();
    for (name, body) in [("sand", SAND), ("other", OTHER), ("golden", GOLDEN)] {
        std::fs::write(themes.join(format!("{name}.toml")), body).unwrap();
    }

    let mut ed = editor_from("-[a]>b\n");
    ed.state.dirs = fixture.dirs();
    ed.init_scripting(&mut Default::default());
    (ed, fixture)
}

fn style(ed: &Editor, scope: &'static str) -> hume_engine::types::ResolvedStyle {
    ed.view.theme.resolve_by_name(Scope(scope))
}

fn warnings(ed: &Editor) -> Vec<String> {
    ed.state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Warning)
        .map(|e| e.text.clone())
        .collect()
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
    assert_eq!(warnings(&ed), Vec::<String>::new());
}

#[test]
fn an_unresolved_color_name_warns_at_each_theme_install() {
    let (mut ed, _fixture) = session(
        r##"(set-scope-styles! (list (hash 'scope "keyword" 'fg "gold" 'bg "#abcdef")))"##,
        "",
    );
    assert_eq!(ed.state.shown_theme, "sand");
    assert_eq!(warnings(&ed).len(), 1, "{:?}", warnings(&ed));
    let keyword = style(&ed, "keyword");
    assert_eq!((keyword.fg, keyword.bg), (None, Some(SCRIPT_RGB)));

    type_cmd(&mut ed, ":theme other");
    assert_eq!(warnings(&ed).len(), 2, "{:?}", warnings(&ed));

    type_cmd(&mut ed, ":theme golden");
    assert_eq!(warnings(&ed).len(), 2, "{:?}", warnings(&ed));
    assert_eq!(style(&ed, "keyword").fg, Some(Rgb(0xff, 0xd7, 0x00)));
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
fn reload_config_drops_styles_init_no_longer_sets() {
    let (mut ed, fixture) = session(
        r##"(set-scope-styles! (list (hash 'scope "keyword" 'fg "#abcdef")))"##,
        "",
    );
    assert_eq!(style(&ed, "keyword").fg, Some(SCRIPT_RGB));
    std::fs::write(fixture.config_dir.join("init.scm"), "").unwrap();

    type_cmd(&mut ed, ":reload-config");

    assert_eq!(style(&ed, "keyword").fg, Some(Rgb(0x11, 0x11, 0x11)));
}
