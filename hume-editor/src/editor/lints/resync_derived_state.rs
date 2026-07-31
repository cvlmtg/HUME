//! # `resync_derived_state` completeness
//!
//! `settings::has_declared_resync` and
//! `editor::settings_ops::resync_derived_state` are two independent sources
//! for the same fact — which settings have a derived-state effect —
//! kept in sync only by a one-directional `debug_assert!` inside
//! `resync_derived_state` itself: a key that declares `resync: true` in
//! `define_settings!` but has no matching match arm panics immediately. The
//! reverse has no check at all: a match arm added for a key that never
//! declared `resync: true` compiles and runs fine on every `:set`, but
//! `reset_globals`'s `has_declared_resync`-gated loop silently skips it on
//! `:reload-config`, so the effect never resyncs after a reload.
//! `resync_derived_state_arms_all_declare_resync_true` extracts every
//! string-literal match-arm pattern from `resync_derived_state`'s source and
//! asserts `has_declared_resync` is true for each, closing the other
//! direction.

/// Every string literal used as a match-arm pattern inside
/// `editor::settings_ops::resync_derived_state`'s `match key { ... }` —
/// the key names that function actually has code for, regardless of
/// whether `define_settings!` declared `resync: true` for them. Brace-depth
/// tracked (same technique as `plugin_manifest::manifest_declared_commands`)
/// so the scan stops exactly at the function's own closing brace, not some
/// unrelated later `"..."` literal.
fn resync_derived_state_arm_keys(src: &str) -> std::collections::BTreeSet<String> {
    let fn_start = src
        .find("fn resync_derived_state(")
        .expect("fn resync_derived_state not found in settings_ops.rs");
    let body_start = src[fn_start..]
        .find('{')
        .expect("no opening brace for resync_derived_state")
        + fn_start;
    let mut depth = 0i32;
    let mut end = body_start;
    for (i, c) in src[body_start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = body_start + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    src[body_start..end]
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix('"')?;
            let (key, after) = rest.split_once('"')?;
            let after = after.trim_start();
            (after.starts_with("=>") || after.starts_with("if")).then(|| key.to_string())
        })
        .collect()
}

/// Fail oracle: add a match arm to `resync_derived_state` for a key that
/// never declares `resync: true` in `define_settings!` — this test fails
/// naming the key, catching the direction the function's own
/// `debug_assert!` cannot (that one only catches "declared but no arm").
#[test]
fn resync_derived_state_arms_all_declare_resync_true() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
    let path = std::path::Path::new(&manifest).join("src/editor/settings_ops.rs");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

    let undeclared: Vec<String> = resync_derived_state_arm_keys(&src)
        .into_iter()
        .filter(|key| !crate::settings::has_declared_resync(key))
        .collect();

    assert!(
        undeclared.is_empty(),
        "resync_derived_state has a match arm for {undeclared:?} but \
         define_settings! doesn't declare `resync: true` for it — \
         reset_globals's has_declared_resync-gated loop would silently \
         skip resyncing it on :reload-config"
    );
}
