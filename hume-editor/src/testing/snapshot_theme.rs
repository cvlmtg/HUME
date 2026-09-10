//! `gruvbox.toml`, embedded for renderer snapshot tests that assert exact
//! colors. Those tests exercise seam/junction/dimming *rendering mechanics*,
//! not the default theme's palette — pinning them to a stable theme means
//! retuning `sand.toml` (the compiled-in default) never forces an unrelated
//! snapshot re-record. `gruvbox.toml` is a vendored upstream file rather
//! than one HUME tunes for its own sake, so it stays stable for the same
//! reason `sand.toml` doesn't serve this role.
//!
//! Lives here rather than in `ui::theme` (where it once did): it is a test
//! fixture used from `editor::tests`, not a `ui::theme` internal, and its
//! crate-wide reach is a fixture's ordinary shape rather than a leak.

const SNAPSHOT_THEME_TOML: &str = include_str!("../../../runtime/themes/gruvbox.toml");

pub(crate) fn build_snapshot_theme() -> hume_engine::theme::Theme {
    let loaded = hume_engine::theme::loader::parse_theme(SNAPSHOT_THEME_TOML)
        .expect("embedded gruvbox.toml must parse — file is compile-time embedded");
    // A real `assert!`, not `debug_assert!` — see `ui::theme::build_default_theme`'s
    // own copy of this reasoning.
    assert!(
        loaded.warnings.is_empty(),
        "embedded gruvbox.toml produced load warnings: {:?}",
        loaded.warnings
    );
    loaded.theme
}
