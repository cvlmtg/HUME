use hume_grid::Rgb;
use std::path::Path;

use super::*;
use crate::types::Modifiers;
use tempfile::TempDir;

// ── Test fixture helpers ──────────────────────────────────────────────────

fn write_theme(dir: &Path, name: &str, content: &str) {
    std::fs::write(dir.join(format!("{name}.toml")), content).expect("failed to write test theme");
}

fn paths(dir: &Path) -> Vec<PathBuf> {
    vec![dir.to_path_buf()]
}

/// Unwrap the one `ThemeError::InFile` wrapper a file-sourced error carries,
/// for tests that assert on the inner variant regardless of which file it came
/// from.
///
/// Deliberately not recursive: attribution must happen exactly once, or the
/// message prints two paths for one error and the outer (less specific) one
/// wins the reader's eye. Every test that reaches for the inner variant
/// therefore also guards against a double wrap.
fn inner(err: &ThemeError) -> &ThemeError {
    match err {
        ThemeError::InFile { error, .. } => {
            assert!(
                !matches!(**error, ThemeError::InFile { .. }),
                "error attributed to a file twice: {err}"
            );
            error
        }
        other => other,
    }
}

/// The path a file-sourced error is attributed to, for tests that assert on
/// the attribution itself rather than on the error inside it.
fn attributed_file(err: &ThemeError) -> &Path {
    match err {
        ThemeError::InFile { path, .. } => path,
        other => panic!("expected an InFile-attributed error, got: {other}"),
    }
}

/// The warning among `loaded.warnings` matching `predicate` — panics unless
/// exactly one does. `predicate` sees each warning through `inner`, so it
/// matches on the payload regardless of file attribution; the returned
/// reference is the original (possibly `InFile`-wrapped) entry, so a caller
/// that also wants the attributed path can pass it to `attributed_file`.
fn the_warning<'a>(
    loaded: &'a LoadedTheme,
    predicate: impl Fn(&ThemeError) -> bool,
) -> &'a ThemeError {
    let matches: Vec<&ThemeError> = loaded
        .warnings
        .iter()
        .filter(|w| predicate(inner(w)))
        .collect();
    match matches.as_slice() {
        [w] => w,
        _ => panic!(
            "expected exactly one warning matching the predicate, got: {:?}",
            loaded.warnings
        ),
    }
}

/// A malformed key still resolves — to the default (empty) style Helix also
/// gives a bad key, not to whatever its dot-notation ancestor would supply.
/// This is what "warn and load" means in practice: the key isn't dropped,
/// its slot is just empty, so it still blocks fallback the way a real entry
/// would. Only holds when *nothing* in the entry parsed — see
/// `assert_bad_style_field` for the partial case.
fn assert_resolves_to_default_style(loaded: &LoadedTheme, key: &'static str) {
    assert_eq!(
        loaded.theme.resolve_by_name(crate::types::Scope(key)),
        ResolvedStyle::default(),
        "'{key}' must resolve to an empty style, not fall through to a dot-notation ancestor"
    );
}

/// Assert `loaded`'s one `BadStyleField` warning has this payload, and that
/// `key` resolves to `expected_style` — the fields that parsed cleanly
/// alongside the bad one, per `parse_style_table`'s partial-style behavior.
/// The payload check alone can't tell strict-type rejections apart — every
/// one produces the same variant, and only `field`/`expected` say which
/// fired — so a mislabelled arm would otherwise pass.
fn assert_bad_style_field(
    loaded: &LoadedTheme,
    key: &'static str,
    field: &str,
    expected: &str,
    expected_style: ResolvedStyle,
) {
    let warning = the_warning(loaded, |w| matches!(w, ThemeError::BadStyleField { .. }));
    match inner(warning) {
        ThemeError::BadStyleField {
            key: k,
            field: f,
            expected: e,
        } => {
            assert_eq!((k.as_str(), f.as_str(), *e), (key, field, expected));
        }
        other => panic!("expected BadStyleField, got: {other}"),
    }
    assert_eq!(
        loaded.theme.resolve_by_name(crate::types::Scope(key)),
        expected_style,
        "'{key}' must resolve to the fields that parsed cleanly alongside the bad one"
    );
}

/// The behavior this whole warn-vs-fail split exists for: two independently
/// malformed entries each get collected as their own warning rather than the
/// load stopping at the first one, and every other, well-formed entry in the
/// same document loads normally — mirroring Helix's own `build_theme_values`,
/// which collects every bad key's warning instead of aborting on the first.
#[test]
fn multiple_malformed_entries_each_produce_their_own_warning() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "several_bad",
        r##"
"keyword" = "nonexistent_color"
"comment" = { fg = "#ffffff", modifiers = ["wiggly"] }
"constant" = "#00ff00"
"##,
    );
    let loaded = load_theme("several_bad", &paths(dir.path())).unwrap();

    the_warning(
        &loaded,
        |w| matches!(w, ThemeError::BadColor { key, .. } if key == "keyword"),
    );
    the_warning(
        &loaded,
        |w| matches!(w, ThemeError::BadModifier { key, .. } if key == "comment"),
    );
    assert_eq!(
        loaded.warnings.len(),
        2,
        "each bad key gets its own warning"
    );

    // The one well-formed scope in the same document is unaffected.
    let cn = loaded
        .theme
        .resolve_by_name(crate::types::Scope("constant"));
    assert_eq!(cn.fg, Some(Rgb(0, 0xff, 0)));
}

// ── Flat scope / happy path ───────────────────────────────────────────────

#[test]
fn flat_scope_happy_path() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "test",
        r##"
"keyword" = "#ff0000"
"keyword.function" = { fg = "#00ff00", modifiers = ["bold"] }
"ui.cursor" = { fg = "#ffffff", bg = "#000000" }
"##,
    );

    let theme = load_theme("test", &paths(dir.path())).unwrap().theme;

    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(kw.fg, Some(Rgb(0xff, 0, 0)));
    assert_eq!(kw.bg, None);

    let kw_fn = theme.resolve_by_name(crate::types::Scope("keyword.function"));
    assert_eq!(kw_fn.fg, Some(Rgb(0, 0xff, 0)));
    assert!(kw_fn.modifiers.contains(Modifiers::BOLD));

    // Fallback: "keyword.operator" → "keyword"
    let kw_op = theme.resolve_by_name(crate::types::Scope("keyword.operator"));
    assert_eq!(kw_op.fg, Some(Rgb(0xff, 0, 0)));
}

// ── Palette indirection ───────────────────────────────────────────────────

#[test]
fn palette_indirection() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "pal",
        r##"
"keyword"  = "red"
"comment"  = { fg = "green", modifiers = ["italic"] }
"constant" = "#abcdef"

[palette]
red    = "#cc241d"
green  = "#98971a"
"##,
    );

    let theme = load_theme("pal", &paths(dir.path())).unwrap().theme;

    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(kw.fg, Some(Rgb(0xcc, 0x24, 0x1d)));

    let cm = theme.resolve_by_name(crate::types::Scope("comment"));
    assert_eq!(cm.fg, Some(Rgb(0x98, 0x97, 0x1a)));
    assert!(cm.modifiers.contains(Modifiers::ITALIC));

    // Literal hex still works (not in palette).
    let cn = theme.resolve_by_name(crate::types::Scope("constant"));
    assert_eq!(cn.fg, Some(Rgb(0xab, 0xcd, 0xef)));
}

// ── ANSI colour names ────────────────────────────────────────────────────

#[test]
fn ansi_color_name_resolves_with_no_warning() {
    let dir = TempDir::new().unwrap();
    write_theme(dir.path(), "ansi", r##""keyword" = "red""##);

    let loaded = load_theme("ansi", &paths(dir.path())).unwrap();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);

    let kw = loaded.theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(kw.fg, Some(Rgb(0xcd, 0x00, 0x00)));
}

#[test]
fn every_ansi_color_name_resolves() {
    // Independent oracle: xterm's own default palette, not a read-back of
    // the loader's own table — catches a typo'd name or transposed value
    // that a self-referential check would miss. Each color is used as its
    // own scope key (an arbitrary string as far as the loader is concerned),
    // so one theme document exercises the whole table in a single load.
    let names_and_values = [
        ("black", Rgb(0x00, 0x00, 0x00)),
        ("red", Rgb(0xcd, 0x00, 0x00)),
        ("green", Rgb(0x00, 0xcd, 0x00)),
        ("yellow", Rgb(0xcd, 0xcd, 0x00)),
        ("blue", Rgb(0x00, 0x00, 0xee)),
        ("magenta", Rgb(0xcd, 0x00, 0xcd)),
        ("cyan", Rgb(0x00, 0xcd, 0xcd)),
        ("light-gray", Rgb(0xe5, 0xe5, 0xe5)),
        ("gray", Rgb(0x7f, 0x7f, 0x7f)),
        ("light-red", Rgb(0xff, 0x00, 0x00)),
        ("light-green", Rgb(0x00, 0xff, 0x00)),
        ("light-yellow", Rgb(0xff, 0xff, 0x00)),
        ("light-blue", Rgb(0x5c, 0x5c, 0xff)),
        ("light-magenta", Rgb(0xff, 0x00, 0xff)),
        ("light-cyan", Rgb(0x00, 0xff, 0xff)),
        ("white", Rgb(0xff, 0xff, 0xff)),
    ];

    let dir = TempDir::new().unwrap();
    let body: String = names_and_values
        .iter()
        .map(|(name, _)| format!("\"{name}\" = \"{name}\"\n"))
        .collect();
    write_theme(dir.path(), "ansi-all", &body);

    let loaded = load_theme("ansi-all", &paths(dir.path())).unwrap();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);

    for (name, expected) in names_and_values {
        let resolved = loaded.theme.resolve_by_name(crate::types::Scope(name));
        assert_eq!(resolved.fg, Some(expected), "name '{name}' resolved wrong");
    }
}

#[test]
fn theme_palette_entry_overrides_the_built_in_ansi_name() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "ansi-override",
        r##"
"keyword" = "red"

[palette]
red = "#123456"
"##,
    );

    let theme = load_theme("ansi-override", &paths(dir.path()))
        .unwrap()
        .theme;
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(kw.fg, Some(Rgb(0x12, 0x34, 0x56)));
}

#[test]
fn a_color_name_helix_does_not_define_still_warns() {
    let dir = TempDir::new().unwrap();
    write_theme(dir.path(), "notansi", r##""keyword" = "orange""##);

    let loaded = load_theme("notansi", &paths(dir.path())).unwrap();
    the_warning(
        &loaded,
        |w| matches!(w, ThemeError::BadColor { key, value } if key == "keyword" && value == "orange"),
    );
    assert_resolves_to_default_style(&loaded, "keyword");
}

// ── Inheritance ───────────────────────────────────────────────────────────

#[test]
fn inherits_child_overrides_parent() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "base",
        r##"
"keyword"   = "#ff0000"
"comment"   = "#888888"
"constant"  = "#abcdef"
"##,
    );
    write_theme(
        dir.path(),
        "child",
        r##"
inherits = "base"
"keyword" = "#00ff00"
"##,
    );

    let theme = load_theme("child", &paths(dir.path())).unwrap().theme;

    // Child overrides "keyword".
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(kw.fg, Some(Rgb(0, 0xff, 0)));

    // Parent's "comment" and "constant" still present.
    let cm = theme.resolve_by_name(crate::types::Scope("comment"));
    assert_eq!(cm.fg, Some(Rgb(0x88, 0x88, 0x88)));
    let cn = theme.resolve_by_name(crate::types::Scope("constant"));
    assert_eq!(cn.fg, Some(Rgb(0xab, 0xcd, 0xef)));
}

#[test]
fn inherits_child_palette_names_union_with_no_collision() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "base2",
        r##"
"keyword" = "red"

[palette]
red = "#ff0000"
"##,
    );
    write_theme(
        dir.path(),
        "child2",
        r##"
inherits = "base2"

"comment" = "blue"

[palette]
blue = "#0000ff"
"##,
    );

    let theme = load_theme("child2", &paths(dir.path())).unwrap().theme;

    // Parent's own palette entry still resolves the parent's scope.
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(kw.fg, Some(Rgb(0xff, 0, 0)));

    // Child's palette entry resolves the child's own scope. No name
    // collision here — see the tests below for what a collision does.
    let cm = theme.resolve_by_name(crate::types::Scope("comment"));
    assert_eq!(cm.fg, Some(Rgb(0, 0, 0xff)));
}

#[test]
fn inherits_child_scope_override_can_reference_parent_palette_name() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "base5",
        r##"
"keyword" = "#888888"

[palette]
accent = "#ff8800"
"##,
    );
    write_theme(
        dir.path(),
        "child5",
        r##"
inherits = "base5"
"keyword" = "accent"
"##,
    );

    let theme = load_theme("child5", &paths(dir.path())).unwrap().theme;

    // Child redeclares "keyword" using "accent" — a name only the parent
    // defines. The parent's palette must be visible to the child's own
    // overrides, not just to the scopes the child leaves untouched.
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(kw.fg, Some(Rgb(0xff, 0x88, 0x00)));
}

#[test]
fn inherits_child_palette_override_reskins_unmodified_parent_scope() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "base6",
        r##"
"ui.background" = { bg = "bg0" }

[palette]
bg0 = "#111111"
"##,
    );
    write_theme(
        dir.path(),
        "child6",
        r##"
inherits = "base6"

[palette]
bg0 = "#eeeeee"
"##,
    );

    let theme = load_theme("child6", &paths(dir.path())).unwrap().theme;

    // Child never redeclares "ui.background" — it only overrides "bg0" in
    // its own palette. A Helix-style light/dark variant pair depends on
    // exactly this: reskin the palette, keep every inherited scope's
    // structure untouched.
    let bg = theme.resolve_by_name(crate::types::Scope("ui.background"));
    assert_eq!(bg.bg, Some(Rgb(0xee, 0xee, 0xee)));
}

// ── `ui.text` → `default` fold ────────────────────────────────────────────

#[test]
fn ui_text_folds_into_default() {
    let theme = parse_theme(r##""ui.text" = { fg = "#d0d0d0" }"##)
        .unwrap()
        .theme;
    assert_eq!(theme.default.fg, Some(Rgb(0xd0, 0xd0, 0xd0)));
}

#[test]
fn inherits_child_overrides_parent_ui_text_default() {
    let dir = TempDir::new().unwrap();
    write_theme(dir.path(), "base3", r##""ui.text" = { fg = "#111111" }"##);
    write_theme(
        dir.path(),
        "child3",
        r##"
inherits = "base3"
"ui.text" = { fg = "#222222" }
"##,
    );

    let theme = load_theme("child3", &paths(dir.path())).unwrap().theme;

    // The merge replaces the parent's "ui.text" with the child's wholesale;
    // `default` is folded once, from that final merged value.
    assert_eq!(theme.default.fg, Some(Rgb(0x22, 0x22, 0x22)));
}

#[test]
fn inherits_child_without_ui_text_keeps_parent_default() {
    let dir = TempDir::new().unwrap();
    write_theme(dir.path(), "base4", r##""ui.text" = { fg = "#111111" }"##);
    write_theme(dir.path(), "child4", r#"inherits = "base4""#);

    let theme = load_theme("child4", &paths(dir.path())).unwrap().theme;

    // Child never redeclares "ui.text" — the merge keeps the parent's entry
    // untouched, and `default` folds from that same value.
    assert_eq!(theme.default.fg, Some(Rgb(0x11, 0x11, 0x11)));
}

#[test]
fn inherits_child_ui_text_override_replaces_modifiers_wholesale() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "base7",
        r##""ui.text" = { fg = "#111111", modifiers = ["bold"] }"##,
    );
    write_theme(
        dir.path(),
        "child7",
        r##"
inherits = "base7"
"ui.text" = { fg = "#222222" }
"##,
    );

    let theme = load_theme("child7", &paths(dir.path())).unwrap().theme;

    // Child's "ui.text" replaces the parent's whole style for that scope —
    // it does not layer on top of it. A child that drops "modifiers" must
    // not still carry the parent's bold into `default`.
    assert_eq!(theme.default.fg, Some(Rgb(0x22, 0x22, 0x22)));
    assert!(
        !theme.default.modifiers.contains(Modifiers::BOLD),
        "child's ui.text override dropped modifiers; default must not stay bold"
    );
}

// ── Cycle detection ───────────────────────────────────────────────────────

#[test]
fn cycle_is_detected() {
    let dir = TempDir::new().unwrap();
    write_theme(dir.path(), "a", r#"inherits = "b""#);
    write_theme(dir.path(), "b", r#"inherits = "a""#);

    let err = load_theme("a", &paths(dir.path()))
        .err()
        .expect("expected an Err result");
    assert!(
        matches!(inner(&err), ThemeError::Cycle { .. }),
        "expected Cycle error, got: {err}"
    );
}

/// A config-dir theme that shadows a bundled theme of the same name must
/// still be able to `inherits` it — the shadowed (lower-priority) copy is a
/// distinct file, not the shadowing one itself.
#[test]
fn shadowing_theme_can_inherit_the_theme_it_shadows() {
    let shadowing_dir = TempDir::new().unwrap();
    let shadowed_dir = TempDir::new().unwrap();

    write_theme(
        shadowing_dir.path(),
        "gruvbox",
        r##"
inherits = "gruvbox"

[palette]
bg0 = "#eeeeee"
"##,
    );
    write_theme(
        shadowed_dir.path(),
        "gruvbox",
        r##"
"ui.background" = { bg = "bg0" }

[palette]
bg0 = "#111111"
"##,
    );

    let search_paths = vec![
        shadowing_dir.path().to_path_buf(),
        shadowed_dir.path().to_path_buf(),
    ];
    let theme = load_theme("gruvbox", &search_paths)
        .unwrap_or_else(|e| panic!("expected the shadowed file to resolve as parent, got: {e}"))
        .theme;

    let bg = theme.resolve_by_name(crate::types::Scope("ui.background"));
    assert_eq!(bg.bg, Some(Rgb(0xee, 0xee, 0xee)));
}

// ── Max depth ─────────────────────────────────────────────────────────────

#[test]
fn max_depth_is_detected() {
    let dir = TempDir::new().unwrap();
    // Chain: t0 → t1 → … → t9 (10 levels, exceeds MAX_DEPTH=8)
    for i in 0..=9usize {
        let content = if i < 9 {
            format!("inherits = \"t{}\"", i + 1)
        } else {
            r##""keyword" = "#ff0000""##.to_owned()
        };
        write_theme(dir.path(), &format!("t{i}"), &content);
    }

    let err = load_theme("t0", &paths(dir.path()))
        .err()
        .expect("expected an Err result");
    assert!(
        matches!(inner(&err), ThemeError::MaxDepth { .. }),
        "expected MaxDepth error, got: {err}"
    );
}

/// The other side of the limit: a chain that reaches exactly `MAX_DEPTH` must
/// load. Without this, the `depth > MAX_DEPTH` comparison could be off by one
/// in the permissive direction and `max_depth_is_detected` would still pass.
#[test]
fn chain_at_exactly_max_depth_loads() {
    let dir = TempDir::new().unwrap();
    // d0 inherits d1 … d8 defines the scope: d8 is loaded at depth
    // MAX_DEPTH (8), the deepest level allowed.
    for i in 0..=MAX_DEPTH {
        let content = if i < MAX_DEPTH {
            format!("inherits = \"d{}\"", i + 1)
        } else {
            r##""keyword" = "#ff0000""##.to_owned()
        };
        write_theme(dir.path(), &format!("d{i}"), &content);
    }

    let theme = load_theme("d0", &paths(dir.path()))
        .unwrap_or_else(|e| panic!("a chain of exactly MAX_DEPTH must load, got: {e}"))
        .theme;
    assert_eq!(
        theme.resolve_by_name(crate::types::Scope("keyword")).fg,
        Some(Rgb(0xff, 0, 0))
    );
}

/// A theme naming itself has no lower-priority file to fall through to, so
/// the search exhausts every candidate and reports the cycle rather than
/// recursing until `MAX_DEPTH`.
#[test]
fn self_inherits_is_a_cycle() {
    let dir = TempDir::new().unwrap();
    write_theme(dir.path(), "loop", r#"inherits = "loop""#);

    let err = load_theme("loop", &paths(dir.path()))
        .err()
        .expect("expected an Err result");
    assert!(
        matches!(inner(&err), ThemeError::Cycle { name } if name == "loop"),
        "expected Cycle naming loop, got: {err}"
    );
}

// ── Not found ─────────────────────────────────────────────────────────────

#[test]
fn not_found_returns_error() {
    let dir = TempDir::new().unwrap();
    let err = load_theme("nonexistent", &paths(dir.path()))
        .err()
        .expect("expected an Err result");
    assert!(matches!(inner(&err), ThemeError::NotFound { .. }));
}

// ── Bad palette reference ─────────────────────────────────────────────────

#[test]
fn bad_palette_ref_becomes_a_warning() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "bad",
        r#"
"keyword" = "nonexistent_color"
"#,
    );
    let loaded = load_theme("bad", &paths(dir.path())).unwrap();
    the_warning(&loaded, |w| matches!(w, ThemeError::BadColor { .. }));
    assert_resolves_to_default_style(&loaded, "keyword");
}

// ── Malformed reserved keys ────────────────────────────────────────────────

#[test]
fn non_string_inherits_is_error() {
    let err = parse_theme(r#"inherits = 42"#)
        .err()
        .expect("expected an Err result for a non-string inherits");
    assert!(
        matches!(
            inner(&err),
            ThemeError::BadReservedKey { key, expected }
                if *key == "inherits" && *expected == "a string"
        ),
        "expected BadReservedKey naming inherits, got: {err}"
    );
}

#[test]
fn non_table_palette_is_error() {
    let err = parse_theme(r#"palette = "light""#)
        .err()
        .expect("expected an Err result for a non-table palette");
    assert!(
        matches!(
            inner(&err),
            ThemeError::BadReservedKey { key, expected }
                if *key == "palette" && *expected == "a table"
        ),
        "expected BadReservedKey naming palette, got: {err}"
    );
}

#[test]
fn parent_side_non_table_palette_is_error() {
    let dir = TempDir::new().unwrap();
    write_theme(dir.path(), "badpal_parent", r#"palette = "light""#);
    write_theme(
        dir.path(),
        "badpal_child",
        r##"
inherits = "badpal_parent"

[palette]
bg0 = "#111111"
"##,
    );

    let err = load_theme("badpal_child", &paths(dir.path()))
        .err()
        .expect("expected an Err result for a malformed parent palette");
    assert!(
        matches!(
            inner(&err),
            ThemeError::BadReservedKey { key, .. } if *key == "palette"
        ),
        "expected BadReservedKey naming palette, got: {err}"
    );
    // The whole point of this case: the child's own palette is well-formed.
    assert_eq!(
        attributed_file(&err).file_name().unwrap(),
        "badpal_parent.toml"
    );
}

#[test]
fn empty_string_color_becomes_a_warning() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "empty_color",
        r#""constant" = { fg = "white", bg = "" }"#,
    );
    let loaded = load_theme("empty_color", &paths(dir.path())).unwrap();
    the_warning(&loaded, |w| matches!(w, ThemeError::BadColor { .. }));
    let cn = loaded
        .theme
        .resolve_by_name(crate::types::Scope("constant"));
    assert_eq!(
        cn.fg,
        Some(Rgb(0xff, 0xff, 0xff)),
        "the valid fg must survive the bad bg alongside it"
    );
    assert_eq!(cn.bg, None);
}

// ── Bad modifier ─────────────────────────────────────────────────────────

#[test]
fn bad_modifier_becomes_a_warning() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "bad_mod",
        r##"
"keyword" = { fg = "#ff0000", modifiers = ["wiggly"] }
"##,
    );
    let loaded = load_theme("bad_mod", &paths(dir.path())).unwrap();
    the_warning(&loaded, |w| matches!(w, ThemeError::BadModifier { .. }));
    let kw = loaded.theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(
        kw.fg,
        Some(Rgb(0xff, 0, 0)),
        "the valid fg must survive the bad modifier alongside it"
    );
    assert_eq!(kw.modifiers, Modifiers::empty());
}

// ── crossed_out (Helix name for strikethrough) ────────────────────────────

#[test]
fn crossed_out_is_accepted_as_strikethrough() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "helix_compat",
        r##"
"keyword" = { fg = "#ff0000", modifiers = ["crossed_out"] }
"##,
    );
    let theme = load_theme("helix_compat", &paths(dir.path()))
        .unwrap()
        .theme;
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    assert!(kw.modifiers.contains(Modifiers::STRIKETHROUGH));
}

/// HUME used to also accept its own "strikethrough" name alongside Helix's
/// "crossed_out" — dropped: Helix compatibility beats a HUME-only alias with
/// no reason to exist once the real name works everywhere.
#[test]
fn strikethrough_is_no_longer_accepted_as_a_modifier_name() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "old_alias",
        r##"
"keyword" = { fg = "#ff0000", modifiers = ["strikethrough"] }
"##,
    );
    let loaded = load_theme("old_alias", &paths(dir.path())).unwrap();
    the_warning(
        &loaded,
        |w| matches!(w, ThemeError::BadModifier { key, value } if key == "keyword" && value == "strikethrough"),
    );
    let kw = loaded.theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(
        kw.fg,
        Some(Rgb(0xff, 0, 0)),
        "the valid fg must survive the bad modifier alongside it"
    );
    assert_eq!(kw.modifiers, Modifiers::empty());
}

// ── Bad underline style ───────────────────────────────────────────────────

#[test]
fn bad_underline_becomes_a_warning() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "bad_underline",
        r##"
"keyword" = { fg = "#ff0000", underline = "squiggly" }
"##,
    );
    let loaded = load_theme("bad_underline", &paths(dir.path())).unwrap();
    the_warning(&loaded, |w| matches!(w, ThemeError::BadUnderline { .. }));
    let kw = loaded.theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(
        kw.fg,
        Some(Rgb(0xff, 0, 0)),
        "the valid fg must survive the bad underline alongside it"
    );
    assert_eq!(kw.underline, UnderlineStyle::None);
}

/// HUME used to also accept "solid"/"wavy"/"undercurl" alongside Helix's own
/// "line"/"curl" — dropped for the same reason as `strikethrough` above.
#[test]
fn hume_only_underline_aliases_are_no_longer_accepted() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "old_underline_aliases",
        r##"
"a" = { underline = "solid" }
"b" = { underline = "wavy" }
"c" = { underline = "undercurl" }
"##,
    );
    let loaded = load_theme("old_underline_aliases", &paths(dir.path())).unwrap();
    for (key, value) in [("a", "solid"), ("b", "wavy"), ("c", "undercurl")] {
        the_warning(
            &loaded,
            |w| matches!(w, ThemeError::BadUnderline { key: k, value: v } if k == key && v == value),
        );
        assert_resolves_to_default_style(&loaded, key);
    }
}

// ── double_line underline (Helix's own name) ──────────────────────────────

#[test]
fn double_line_underline_is_accepted() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "double_underline",
        r##"
"keyword" = { fg = "#ffffff", underline = "double_line" }
"##,
    );
    let theme = load_theme("double_underline", &paths(dir.path()))
        .unwrap()
        .theme;
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(kw.underline, UnderlineStyle::Double);
}

// ── Shorthand 3-digit hex ─────────────────────────────────────────────────

#[test]
fn shorthand_hex_expands_correctly() {
    let dir = TempDir::new().unwrap();
    write_theme(dir.path(), "short", r##""keyword" = "#f0a""##);
    let theme = load_theme("short", &paths(dir.path())).unwrap().theme;
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    // #f0a → #ff00aa
    assert_eq!(kw.fg, Some(Rgb(0xff, 0x00, 0xaa)));
}

// ── Path traversal rejection ──────────────────────────────────────────────

#[test]
fn path_traversal_is_rejected() {
    let dir = TempDir::new().unwrap();
    let err = load_theme("../etc/passwd", &paths(dir.path()))
        .err()
        .expect("expected an Err result");
    assert!(matches!(inner(&err), ThemeError::NotFound { .. }));
}

#[test]
fn drive_relative_segment_is_rejected() {
    assert!(!is_safe_theme_name("c:evil"));
}

#[test]
fn quote_embedded_segment_is_rejected() {
    assert!(!is_safe_theme_name("a\"b"));
}

#[test]
fn empty_segment_is_rejected() {
    assert!(!is_safe_theme_name(""));
}

#[test]
fn nul_embedded_segment_is_rejected() {
    assert!(!is_safe_theme_name("a\0b"));
}

/// A NUL in the name must be refused by the name check rather than by the
/// filesystem: `read_to_string` reports it as `InvalidInput`, which is not the
/// `NotFound` kind the search loop skips on, so it would otherwise surface as
/// a confusing `ThemeError::Io` instead of "no such theme".
#[test]
fn nul_embedded_name_reports_not_found_not_io() {
    let dir = TempDir::new().unwrap();
    let err = load_theme("a\0b", &paths(dir.path()))
        .err()
        .expect("expected an Err result");
    assert!(
        matches!(inner(&err), ThemeError::NotFound { .. }),
        "expected NotFound, got: {err}"
    );
}

/// A higher-priority candidate that exists but can't be read (here: a
/// directory sitting where `<name>.toml` should be a file) must not shadow a
/// working lower-priority one — search order is a priority list, and one
/// broken candidate must not take the whole search down with it.
#[test]
fn unreadable_higher_priority_candidate_falls_through_to_the_next_search_dir() {
    let broken_dir = TempDir::new().unwrap();
    std::fs::create_dir(broken_dir.path().join("sand.toml")).unwrap();

    let working_dir = TempDir::new().unwrap();
    write_theme(working_dir.path(), "sand", r##""keyword" = "#ff0000""##);

    let search_paths = vec![
        broken_dir.path().to_path_buf(),
        working_dir.path().to_path_buf(),
    ];
    let theme = load_theme("sand", &search_paths).unwrap().theme;
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(kw.fg, Some(Rgb(0xff, 0, 0)));
}

// ── parse_theme ───────────────────────────────────────────────────────────

#[test]
fn parse_theme_handles_palette_indirection() {
    let toml = r##"
"ui.cursor" = { fg = "black", bg = "white" }
"ui.virtual" = { fg = "dark_gray" }

[palette]
black     = "#000000"
white     = "#ffffff"
dark_gray = "#808080"
"##;
    let theme = super::parse_theme(toml).unwrap().theme;

    let cursor = theme.resolve_by_name(crate::types::Scope("ui.cursor"));
    // Independent oracle: expected colors derived directly from palette hex values.
    assert_eq!(cursor.fg, Some(Rgb(0x00, 0x00, 0x00)));
    assert_eq!(cursor.bg, Some(Rgb(0xff, 0xff, 0xff)));

    let virt = theme.resolve_by_name(crate::types::Scope("ui.virtual"));
    assert_eq!(virt.fg, Some(Rgb(0x80, 0x80, 0x80)));
}

#[test]
fn parse_theme_rejects_inherits() {
    let toml = r#"inherits = "base""#;
    // With empty search_paths, `load_raw_recursive("base", &[], …)` must fail NotFound.
    let err = super::parse_theme(toml)
        .err()
        .expect("expected Err for inherits in embedded theme");
    assert!(
        matches!(inner(&err), ThemeError::NotFound { .. }),
        "expected NotFound, got: {err}"
    );
}

// ── Modifiers and underline styles ────────────────────────────────────────

#[test]
fn multiple_modifiers_parse_correctly() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "mods",
        r##"
"keyword" = { fg = "#ffffff", modifiers = ["bold", "italic"] }
"##,
    );
    let theme = load_theme("mods", &paths(dir.path())).unwrap().theme;
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    // Expected: Modifiers::BOLD | Modifiers::ITALIC (constructed independently)
    let expected = Modifiers::BOLD | Modifiers::ITALIC;
    assert_eq!(kw.modifiers, expected);
}

#[test]
fn underline_style_is_parsed() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "underline",
        r##"
"keyword" = { fg = "#ffffff", underline = "curl" }
"comment" = { fg = "#888888", underline = "line" }
"##,
    );
    let theme = load_theme("underline", &paths(dir.path())).unwrap().theme;
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    let cm = theme.resolve_by_name(crate::types::Scope("comment"));
    assert_eq!(kw.underline, UnderlineStyle::Wavy);
    assert_eq!(cm.underline, UnderlineStyle::Solid);
}

// ── Full Helix modifier set ───────────────────────────────────────────────

#[test]
fn all_modifiers_parse_correctly() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "all_mods",
        r##"
"keyword" = { modifiers = ["bold","italic","crossed_out","dim","reversed","hidden","slow_blink","rapid_blink"] }
"##,
    );
    let theme = load_theme("all_mods", &paths(dir.path())).unwrap().theme;
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    let expected = Modifiers::BOLD
        | Modifiers::ITALIC
        | Modifiers::STRIKETHROUGH
        | Modifiers::DIM
        | Modifiers::REVERSED
        | Modifiers::HIDDEN
        | Modifiers::SLOW_BLINK
        | Modifiers::RAPID_BLINK;
    assert_eq!(kw.modifiers, expected);
}

#[test]
fn underlined_modifier_maps_to_solid() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "underlined_mod",
        r##"
"keyword" = { fg = "#ffffff", modifiers = ["underlined"] }
"##,
    );
    let theme = load_theme("underlined_mod", &paths(dir.path()))
        .unwrap()
        .theme;
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(kw.underline, UnderlineStyle::Solid);
    assert_eq!(kw.modifiers, Modifiers::empty());
}

#[test]
fn underline_key_wins_over_underlined_modifier() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "underline_priority",
        r##"
"keyword" = { underline = "curl", modifiers = ["underlined"] }
"##,
    );
    let theme = load_theme("underline_priority", &paths(dir.path()))
        .unwrap()
        .theme;
    let kw = theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(kw.underline, UnderlineStyle::Wavy);
}

// ── Nested TOML section headers ───────────────────────────────────────────

#[test]
fn nested_section_header_becomes_dotted_scope() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "nested",
        r##"
[ui]
text = "#d0d0d0"
"##,
    );
    let theme = load_theme("nested", &paths(dir.path())).unwrap().theme;

    let text = theme.resolve_by_name(crate::types::Scope("ui.text"));
    assert_eq!(text.fg, Some(Rgb(0xd0, 0xd0, 0xd0)));
    assert_eq!(theme.default.fg, Some(Rgb(0xd0, 0xd0, 0xd0)));

    // A container-only "ui" table must emit nothing for itself — otherwise
    // it poisons the dot-fallback chain and every unset ui.* scope resolves
    // to an empty style instead of falling through to `default`.
    let other = theme.resolve_by_name(crate::types::Scope("ui.other"));
    assert_eq!(other, theme.default);
}

#[test]
fn nested_section_header_style_table() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "nested_cursor",
        r##"
[ui.cursor]
fg = "#ffffff"
bg = "#000000"
"##,
    );
    let theme = load_theme("nested_cursor", &paths(dir.path()))
        .unwrap()
        .theme;
    let cursor = theme.resolve_by_name(crate::types::Scope("ui.cursor"));
    assert_eq!(cursor.fg, Some(Rgb(0xff, 0xff, 0xff)));
    assert_eq!(cursor.bg, Some(Rgb(0, 0, 0)));
}

/// A table that carries a style field is that scope's style, so a scalar
/// sibling in it is a misspelled style attribute, not a child scope — there is
/// no way to tell `text = "#fff"` here from `underline_style = "curl"`, and
/// reading it as a child silently invents a scope nothing resolves. The child
/// meaning stays available: write `"ui.text"` as its own key. Warned and
/// dropped, not fatal — the scope's own `fg` still loads.
#[test]
fn scalar_sibling_of_a_style_field_is_an_unknown_attribute() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "nested_mix",
        r##"
[ui]
fg = "#aaaaaa"
text = "#ffffff"
"##,
    );
    let loaded = load_theme("nested_mix", &paths(dir.path())).unwrap();
    the_warning(
        &loaded,
        |w| matches!(w, ThemeError::BadStyleField { key, field, .. } if key == "ui" && field == "text"),
    );
    let ui = loaded.theme.resolve_by_name(crate::types::Scope("ui"));
    assert_eq!(ui.fg, Some(Rgb(0xaa, 0xaa, 0xaa)));
}

/// Warned and dropped rather than failing the whole load — one typo in a
/// hand-authored theme must not cost the rest of an otherwise-good file.
#[test]
fn misspelled_style_attribute_names_itself_not_a_phantom_scope() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "typo",
        r##"
"keyword" = { fg = "#ff0000", underline_style = "curl" }
"##,
    );
    let loaded = load_theme("typo", &paths(dir.path())).unwrap();
    the_warning(
        &loaded,
        |w| matches!(w, ThemeError::BadStyleField { key, field, .. } if key == "keyword" && field == "underline_style"),
    );
    let kw = loaded.theme.resolve_by_name(crate::types::Scope("keyword"));
    assert_eq!(kw.fg, Some(Rgb(0xff, 0, 0)));
}

/// A table with no style field of its own is a pure container, so its scalar
/// entries stay child scopes — the shorthand-string form of a section header.
#[test]
fn scalar_child_of_a_container_table_is_still_a_scope() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "container",
        r##"
[ui]
text = "#ffffff"
"##,
    );
    let theme = load_theme("container", &paths(dir.path())).unwrap().theme;
    let text = theme.resolve_by_name(crate::types::Scope("ui.text"));
    assert_eq!(text.fg, Some(Rgb(0xff, 0xff, 0xff)));
}

/// A table child alongside a style field is unambiguous — `[ui.cursor]` with
/// `fg` plus `[ui.cursor.match]` beneath it — and must keep working.
#[test]
fn table_child_alongside_a_style_field_is_still_a_scope() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "table_child",
        r##"
[ui.cursor]
fg = "#ffffff"

[ui.cursor.match]
fg = "#ff0000"
"##,
    );
    let theme = load_theme("table_child", &paths(dir.path())).unwrap().theme;
    assert_eq!(
        theme.resolve_by_name(crate::types::Scope("ui.cursor")).fg,
        Some(Rgb(0xff, 0xff, 0xff))
    );
    assert_eq!(
        theme
            .resolve_by_name(crate::types::Scope("ui.cursor.match"))
            .fg,
        Some(Rgb(0xff, 0x00, 0x00))
    );
}

// ── Palette validation ────────────────────────────────────────────────────

#[test]
fn non_hex_palette_value_becomes_a_warning_and_the_scope_referencing_it_too() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "badpal",
        r##"
"keyword" = "crimson"

[palette]
crimson = "red"
"##,
    );
    let loaded = load_theme("badpal", &paths(dir.path())).unwrap();

    // The malformed palette entry itself...
    the_warning(
        &loaded,
        |w| matches!(w, ThemeError::BadColor { key, value } if key == "palette.crimson" && value == "red"),
    );
    // ...and, since the entry never made it into the palette, the scope
    // that referenced it by name gets its own cascading warning too — the
    // load doesn't try to guess a colour for a name it just dropped.
    the_warning(
        &loaded,
        |w| matches!(w, ThemeError::BadColor { key, value } if key == "keyword" && value == "crimson"),
    );
    assert_resolves_to_default_style(&loaded, "keyword");
}

/// The cascading warning must not turn into a silent recolor: a malformed
/// palette entry whose name happens to collide with a built-in ANSI name
/// (`blue`, here) must not let a referencing scope fall through to that ANSI
/// color once the palette entry itself is dropped.
#[test]
fn malformed_palette_entry_colliding_with_ansi_name_does_not_fall_through_to_it() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "ansi_collision",
        r##"
"function" = "blue"

[palette]
blue = "89b4fa"
"##,
    );
    let loaded = load_theme("ansi_collision", &paths(dir.path())).unwrap();

    the_warning(
        &loaded,
        |w| matches!(w, ThemeError::BadColor { key, value } if key == "palette.blue" && value == "89b4fa"),
    );
    the_warning(
        &loaded,
        |w| matches!(w, ThemeError::BadColor { key, value } if key == "function" && value == "blue"),
    );
    assert_eq!(loaded.warnings.len(), 2);

    let function = loaded
        .theme
        .resolve_by_name(crate::types::Scope("function"));
    assert_ne!(
        function.fg,
        Some(Rgb(0x00, 0x00, 0xee)),
        "must not silently take on ANSI blue once the theme's own 'blue' entry was dropped"
    );
    assert_eq!(function.fg, None);
}

#[test]
fn non_string_palette_value_becomes_a_warning() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "intpal",
        r##"
[palette]
crimson = 16711680
"##,
    );
    let loaded = load_theme("intpal", &paths(dir.path())).unwrap();
    let warning = the_warning(&loaded, |w| matches!(w, ThemeError::BadStyleField { .. }));
    match inner(warning) {
        ThemeError::BadStyleField {
            key,
            field,
            expected,
        } => {
            assert_eq!(
                (key.as_str(), field.as_str(), *expected),
                ("palette", "crimson", "a string")
            );
        }
        other => panic!("expected BadStyleField, got: {other}"),
    }
}

/// The blame belongs to the file whose `[palette]` is malformed, not to the
/// child that happens to reference the name.
#[test]
fn bad_palette_value_in_parent_names_the_parent_file() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "palparent",
        r##"
[palette]
crimson = "red"
"##,
    );
    write_theme(
        dir.path(),
        "palchild",
        r##"
inherits = "palparent"
"keyword" = "crimson"
"##,
    );
    let loaded = load_theme("palchild", &paths(dir.path())).unwrap();
    let warning = the_warning(
        &loaded,
        |w| matches!(w, ThemeError::BadColor { key, .. } if key == "palette.crimson"),
    );
    assert_eq!(
        attributed_file(warning).file_name().unwrap(),
        "palparent.toml",
        "the malformed entry is the parent's own, blame it accordingly"
    );
}

#[test]
fn signed_hex_color_becomes_a_warning() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "signed",
        r##"
"keyword" = "#+f0000"
"##,
    );
    let loaded = load_theme("signed", &paths(dir.path())).unwrap();
    the_warning(
        &loaded,
        |w| matches!(w, ThemeError::BadColor { value, .. } if value == "#+f0000"),
    );
    assert_resolves_to_default_style(&loaded, "keyword");
}

#[test]
fn empty_nested_scope_table_blocks_fallback() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "nested_empty",
        r##"
"ui.cursor" = { fg = "#ffffff" }

[ui.cursor.insert]
"##,
    );
    let theme = load_theme("nested_empty", &paths(dir.path()))
        .unwrap()
        .theme;

    // "ui.cursor.insert" = {} deliberately blocks the dot-fallback chain —
    // it must resolve to an empty style, not to "ui.cursor"'s fg.
    let insert = theme.resolve_by_name(crate::types::Scope("ui.cursor.insert"));
    assert_eq!(insert.fg, None);
}

#[test]
fn child_nested_section_overrides_parent_flat_key() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "flat_base",
        r##""ui.text" = { fg = "#111111" }"##,
    );
    write_theme(
        dir.path(),
        "nested_child",
        r##"
inherits = "flat_base"

[ui]
text = "#222222"
"##,
    );
    let theme = load_theme("nested_child", &paths(dir.path()))
        .unwrap()
        .theme;

    // Flatten must run before the inherits merge — a nested child override
    // and a flat parent key land on the same merged key ("ui.text"), so the
    // child wins outright rather than by iteration-order luck.
    let text = theme.resolve_by_name(crate::types::Scope("ui.text"));
    assert_eq!(text.fg, Some(Rgb(0x22, 0x22, 0x22)));
}

#[test]
fn nested_underline_table_is_a_style_field() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "nested_underline",
        r##"
[ui.text.underline]
color = "#ff0000"
style = "curl"
"##,
    );
    let theme = load_theme("nested_underline", &paths(dir.path()))
        .unwrap()
        .theme;
    let text = theme.resolve_by_name(crate::types::Scope("ui.text"));
    assert_eq!(text.underline, UnderlineStyle::Wavy);
    assert_eq!(text.underline_color, Some(Rgb(0xff, 0, 0)));
}

// ── Strict style-field types ──────────────────────────────────────────────

#[test]
fn non_string_fg_becomes_a_warning() {
    let dir = TempDir::new().unwrap();
    write_theme(dir.path(), "bad_fg", r#""keyword" = { fg = 42 }"#);
    let loaded = load_theme("bad_fg", &paths(dir.path())).unwrap();
    assert_bad_style_field(
        &loaded,
        "keyword",
        "fg",
        "a string",
        ResolvedStyle::default(),
    );
}

#[test]
fn non_string_bg_becomes_a_warning() {
    let dir = TempDir::new().unwrap();
    write_theme(dir.path(), "bad_bg", r#""keyword" = { bg = 42 }"#);
    let loaded = load_theme("bad_bg", &paths(dir.path())).unwrap();
    assert_bad_style_field(
        &loaded,
        "keyword",
        "bg",
        "a string",
        ResolvedStyle::default(),
    );
}

#[test]
fn non_array_modifiers_becomes_a_warning() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "bad_mods_type",
        r##""keyword" = { fg = "#ffffff", modifiers = "bold" }"##,
    );
    let loaded = load_theme("bad_mods_type", &paths(dir.path())).unwrap();
    assert_bad_style_field(
        &loaded,
        "keyword",
        "modifiers",
        "an array",
        ResolvedStyle {
            fg: Some(Rgb(0xff, 0xff, 0xff)),
            ..Default::default()
        },
    );
}

#[test]
fn non_string_modifier_item_becomes_a_warning() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "bad_mod_item",
        r##""keyword" = { modifiers = ["bold", 7] }"##,
    );
    let loaded = load_theme("bad_mod_item", &paths(dir.path())).unwrap();
    assert_bad_style_field(
        &loaded,
        "keyword",
        "modifiers",
        "an array of strings",
        ResolvedStyle {
            modifiers: Modifiers::BOLD,
            ..Default::default()
        },
    );
}

#[test]
fn non_string_underline_becomes_a_warning() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "bad_underline_type",
        r##""keyword" = { fg = "#ffffff", underline = 7 }"##,
    );
    let loaded = load_theme("bad_underline_type", &paths(dir.path())).unwrap();
    assert_bad_style_field(
        &loaded,
        "keyword",
        "underline",
        "a string or a table",
        ResolvedStyle {
            fg: Some(Rgb(0xff, 0xff, 0xff)),
            ..Default::default()
        },
    );
}

#[test]
fn non_string_underline_color_becomes_a_warning() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "bad_uline_color",
        r##""keyword" = { underline = { color = 7 } }"##,
    );
    let loaded = load_theme("bad_uline_color", &paths(dir.path())).unwrap();
    assert_bad_style_field(
        &loaded,
        "keyword",
        "underline.color",
        "a string",
        ResolvedStyle::default(),
    );
}

#[test]
fn non_string_underline_style_becomes_a_warning() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "bad_uline_style",
        r##""keyword" = { underline = { style = 7 } }"##,
    );
    let loaded = load_theme("bad_uline_style", &paths(dir.path())).unwrap();
    assert_bad_style_field(
        &loaded,
        "keyword",
        "underline.style",
        "a string",
        ResolvedStyle::default(),
    );
}

/// A scope value that is neither a string nor a table has no style to build
/// — the shape Helix's `rainbow = [...]` bracket array takes, for instance.
#[test]
fn non_string_non_table_scope_value_becomes_a_warning() {
    let dir = TempDir::new().unwrap();
    write_theme(dir.path(), "bad_scope", r#""keyword" = 42"#);
    let loaded = load_theme("bad_scope", &paths(dir.path())).unwrap();
    the_warning(
        &loaded,
        |w| matches!(w, ThemeError::BadScopeValue { key, .. } if key == "keyword"),
    );
    assert_resolves_to_default_style(&loaded, "keyword");
}

#[test]
fn malformed_toml_is_a_parse_error_naming_its_file() {
    let dir = TempDir::new().unwrap();
    write_theme(dir.path(), "syntax", "\"keyword\" = \n");
    let err = load_theme("syntax", &paths(dir.path()))
        .err()
        .expect("expected an Err result for malformed TOML");
    assert!(
        matches!(inner(&err), ThemeError::Parse(_)),
        "expected Parse, got: {err}"
    );
    assert_eq!(
        attributed_file(&err).file_name().unwrap(),
        "syntax.toml",
        "a parse error must name the file that failed to parse"
    );
}

// ── Error file attribution ────────────────────────────────────────────────

#[test]
fn bad_color_in_parent_names_the_parent_file() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "attrib_parent",
        r#""keyword" = "nonexistent_color""#,
    );
    write_theme(dir.path(), "attrib_child", r#"inherits = "attrib_parent""#);

    let loaded = load_theme("attrib_child", &paths(dir.path())).unwrap();
    let warning = the_warning(&loaded, |w| matches!(w, ThemeError::BadColor { .. }));
    assert_eq!(
        attributed_file(warning).file_name().unwrap(),
        "attrib_parent.toml",
        "expected the warning to name the defining file"
    );
}

#[test]
fn missing_inherits_parent_names_the_requesting_file() {
    let dir = TempDir::new().unwrap();
    write_theme(
        dir.path(),
        "attrib_requester",
        r#"inherits = "does_not_exist""#,
    );

    let err = load_theme("attrib_requester", &paths(dir.path()))
        .err()
        .expect("expected an Err result");
    assert!(
        matches!(inner(&err), ThemeError::NotFound { .. }),
        "expected NotFound, got: {err}"
    );
    let message = err.to_string();
    assert!(
        message.contains("attrib_requester.toml"),
        "expected error to name the requesting file, got: {message}"
    );
}

/// The mirror of `bad_color_in_parent_names_the_parent_file`: when the child
/// restates a key the parent also defines, the child's value is the one that
/// survives the merge, so the child is what the error must name. This is the
/// half of provenance a whole-document attribution would get wrong.
#[test]
fn bad_color_in_child_override_names_the_child_file() {
    let dir = TempDir::new().unwrap();
    write_theme(dir.path(), "ovr_parent", r##""keyword" = "#00ff00""##);
    write_theme(
        dir.path(),
        "ovr_child",
        r#"
inherits = "ovr_parent"
"keyword" = "nonexistent_color"
"#,
    );

    let loaded = load_theme("ovr_child", &paths(dir.path())).unwrap();
    let warning = the_warning(&loaded, |w| matches!(w, ThemeError::BadColor { .. }));
    assert_eq!(
        attributed_file(warning).file_name().unwrap(),
        "ovr_child.toml",
        "the child's override is the value that survived the merge"
    );
    assert_resolves_to_default_style(&loaded, "keyword");
}

/// A parse error in a grandparent must be attributed once, to the grandparent
/// — not re-wrapped by each level that propagated it. `inner` asserts the
/// single wrap; this pins which file it names.
#[test]
fn parse_error_in_a_grandparent_is_attributed_once() {
    let dir = TempDir::new().unwrap();
    write_theme(dir.path(), "gp", "\"keyword\" = \n");
    write_theme(dir.path(), "gp_mid", r#"inherits = "gp""#);
    write_theme(dir.path(), "gp_leaf", r#"inherits = "gp_mid""#);

    let err = load_theme("gp_leaf", &paths(dir.path()))
        .err()
        .expect("expected an Err result");
    assert!(
        matches!(inner(&err), ThemeError::Parse(_)),
        "expected Parse, got: {err}"
    );
    assert_eq!(attributed_file(&err).file_name().unwrap(), "gp.toml");
}

/// The requester is the file that asked for the too-deep parent, so it is the
/// one a reader can go edit.
#[test]
fn max_depth_names_the_requesting_file() {
    let dir = TempDir::new().unwrap();
    for i in 0..=9usize {
        let content = if i < 9 {
            format!("inherits = \"t{}\"", i + 1)
        } else {
            r##""keyword" = "#ff0000""##.to_owned()
        };
        write_theme(dir.path(), &format!("t{i}"), &content);
    }

    let err = load_theme("t0", &paths(dir.path()))
        .err()
        .expect("expected an Err result");
    assert_eq!(
        attributed_file(&err).file_name().unwrap(),
        "t8.toml",
        "the file whose inherits exceeded the limit is the one to name"
    );
}
