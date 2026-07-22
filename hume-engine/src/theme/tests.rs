use super::*;
use ratatui::style::Color;

fn make_theme() -> Theme {
    let mut styles = HashMap::new();
    styles.insert(
        "keyword",
        ResolvedStyle {
            fg: Some(Color::Blue),
            ..Default::default()
        },
    );
    styles.insert(
        "keyword.operator",
        ResolvedStyle {
            fg: Some(Color::Cyan),
            ..Default::default()
        },
    );
    Theme::new(styles, ResolvedStyle::default())
}

// ── Theme::resolve (baked path) ──────────────────────────────────────

#[test]
fn direct_lookup() {
    let mut reg = ScopeRegistry::new();
    let id = reg.intern("keyword.operator");
    let mut theme = make_theme();
    theme.bake(&reg);
    assert_eq!(theme.resolve(id).fg, Some(Color::Cyan));
}

#[test]
fn fallback_to_parent() {
    // "keyword.function" is not in the map; falls back to "keyword".
    let mut reg = ScopeRegistry::new();
    let id = reg.intern("keyword.function");
    let mut theme = make_theme();
    theme.bake(&reg);
    assert_eq!(theme.resolve(id).fg, Some(Color::Blue));
}

#[test]
fn fallback_to_default() {
    let mut reg = ScopeRegistry::new();
    let id = reg.intern("comment");
    let mut theme = make_theme();
    theme.bake(&reg);
    assert_eq!(theme.resolve(id).fg, None);
}

#[test]
fn bake_resolves_all_interned_scopes() {
    // After bake(), both a direct hit and a fallback scope are O(1).
    let mut reg = ScopeRegistry::new();
    let kw = reg.intern("keyword");
    let kw_op = reg.intern("keyword.operator");
    let kw_fn = reg.intern("keyword.function"); // not in map → falls back
    let mut theme = make_theme();
    theme.bake(&reg);
    assert_eq!(theme.resolve(kw).fg, Some(Color::Blue));
    assert_eq!(theme.resolve(kw_op).fg, Some(Color::Cyan));
    assert_eq!(theme.resolve(kw_fn).fg, Some(Color::Blue)); // fallback baked in
}

#[test]
fn bake_if_stale_rebakes_scopes_interned_after_bake() {
    let mut reg = ScopeRegistry::new();
    let kw = reg.intern("keyword");
    let mut theme = make_theme();
    theme.bake(&reg);
    assert_eq!(theme.baked.len(), reg.len());

    // Intern a new scope after bake() — this id is now unbaked.
    let kw_op = reg.intern("keyword.operator");
    assert!(
        theme.baked.len() < reg.len(),
        "registry must have outgrown baked"
    );

    theme.bake_if_stale(&reg);
    assert_eq!(theme.baked.len(), reg.len());
    // Independent oracle: the themed color, not the pre-rebake default.
    assert_eq!(theme.resolve(kw_op).fg, Some(Color::Cyan));
    assert_eq!(theme.resolve(kw).fg, Some(Color::Blue));

    // No new scopes interned — bake_if_stale is a no-op.
    let baked_before = theme.baked.clone();
    theme.bake_if_stale(&reg);
    assert_eq!(theme.baked, baked_before);
}

#[test]
fn multi_level_fallback() {
    let mut styles = HashMap::new();
    styles.insert(
        "a.b",
        ResolvedStyle {
            fg: Some(Color::Green),
            ..Default::default()
        },
    );
    let mut theme = Theme::new(styles, ResolvedStyle::default());

    let mut reg = ScopeRegistry::new();
    let abc = reg.intern("a.b.c");
    let abcd = reg.intern("a.b.c.d");
    theme.bake(&reg);

    assert_eq!(theme.resolve(abc).fg, Some(Color::Green));
    assert_eq!(theme.resolve(abcd).fg, Some(Color::Green));
}

#[test]
fn empty_theme_returns_default() {
    let mut reg = ScopeRegistry::new();
    let any = reg.intern("anything");
    let empty_str = reg.intern("");
    let mut theme = Theme::default();
    theme.bake(&reg);
    assert_eq!(theme.resolve(any), ResolvedStyle::default());
    assert_eq!(theme.resolve(empty_str), ResolvedStyle::default());
}

// ── Theme::resolve_by_name (slow path, no bake needed) ───────────────

#[test]
fn resolve_by_name_direct() {
    let theme = make_theme();
    assert_eq!(
        theme.resolve_by_name(Scope("keyword.operator")).fg,
        Some(Color::Cyan)
    );
}

#[test]
fn resolve_by_name_fallback() {
    let theme = make_theme();
    assert_eq!(
        theme.resolve_by_name(Scope("keyword.function")).fg,
        Some(Color::Blue)
    );
}

#[test]
fn resolve_by_name_default() {
    let theme = make_theme();
    assert_eq!(theme.resolve_by_name(Scope("comment")).fg, None);
}

// ── UiScopes: populated eagerly in new(), no bake() required ─────────

#[test]
fn ui_scopes_available_before_bake() {
    let mut styles = HashMap::new();
    styles.insert(
        "ui.cursorline",
        ResolvedStyle {
            bg: Some(Color::Blue),
            ..Default::default()
        },
    );
    let theme = Theme::new(styles, ResolvedStyle::default());
    // theme.bake() NOT called — ui.cursorline must still be correct.
    assert_eq!(theme.ui.cursorline.bg, Some(Color::Blue));
}

#[test]
fn window_focused_falls_back_to_window_when_unset() {
    let mut styles = HashMap::new();
    styles.insert(
        "ui.window",
        ResolvedStyle {
            fg: Some(Color::Rgb(0x80, 0x80, 0x80)),
            ..Default::default()
        },
    );
    // No "ui.window.focused" entry — dot-notation must fall back to "ui.window".
    let theme = Theme::new(styles, ResolvedStyle::default());
    assert_eq!(
        theme.ui.window_focused.fg,
        Some(Color::Rgb(0x80, 0x80, 0x80))
    );
    assert_eq!(theme.ui.window_focused, theme.ui.window);
}

#[test]
fn window_focused_uses_its_own_entry_when_set() {
    let mut styles = HashMap::new();
    styles.insert(
        "ui.window",
        ResolvedStyle {
            fg: Some(Color::Rgb(0x80, 0x80, 0x80)),
            ..Default::default()
        },
    );
    styles.insert(
        "ui.window.focused",
        ResolvedStyle {
            fg: Some(Color::Rgb(0xff, 0x80, 0x00)),
            ..Default::default()
        },
    );
    let theme = Theme::new(styles, ResolvedStyle::default());
    assert_eq!(
        theme.ui.window_focused.fg,
        Some(Color::Rgb(0xff, 0x80, 0x00))
    );
    assert_ne!(theme.ui.window_focused, theme.ui.window);
}

// ── ScopeRegistry ────────────────────────────────────────────────────

#[test]
fn registry_intern_is_stable() {
    let mut reg = ScopeRegistry::new();
    let a1 = reg.intern("keyword");
    let a2 = reg.intern("keyword"); // second intern returns same id
    assert_eq!(a1, a2);
}

#[test]
fn registry_different_names_get_different_ids() {
    let mut reg = ScopeRegistry::new();
    let a = reg.intern("keyword");
    let b = reg.intern("variable");
    assert_ne!(a, b);
}

#[test]
fn registry_name_of_round_trips() {
    let mut reg = ScopeRegistry::new();
    let id = reg.intern("type.builtin");
    assert_eq!(reg.name_of(id), "type.builtin");
}

#[test]
fn registry_get_returns_none_for_unknown() {
    let reg = ScopeRegistry::new();
    assert_eq!(reg.get("unknown"), None);
}

// ── Theme::from_owned (loader path) ──────────────────────────────────

#[test]
fn from_owned_resolves_same_as_new() {
    let static_styles: HashMap<&'static str, ResolvedStyle> = {
        let mut m = HashMap::new();
        m.insert(
            "keyword",
            ResolvedStyle {
                fg: Some(Color::Blue),
                ..Default::default()
            },
        );
        m
    };
    let owned_styles: FxHashMap<String, ResolvedStyle> = {
        let mut m = FxHashMap::default();
        m.insert(
            "keyword".to_string(),
            ResolvedStyle {
                fg: Some(Color::Blue),
                ..Default::default()
            },
        );
        m
    };
    let t1 = Theme::new(static_styles, ResolvedStyle::default());
    let t2 = Theme::from_owned(owned_styles, ResolvedStyle::default());
    assert_eq!(
        t1.resolve_by_name(Scope("keyword.function")).fg,
        t2.resolve_by_name(Scope("keyword.function")).fg,
    );
}
