//! # User-manual option-table drift
//!
//! `user-manual/docs/configuration.md`'s "Global options"/"Buffer options"
//! tables are a hand-maintained mirror of `settings::all_setting_keys()`
//! (plus `"language"`, documented but excluded from that list by design —
//! see `settings.rs`'s module doc). Nothing else keeps the two in sync;
//! `user_manual_option_tables_match_all_setting_keys` scans both tables for
//! every backtick-quoted first-column key and diffs the set against the
//! code's key list in both directions, catching a key added to
//! `define_settings!` without a manual row (or vice versa). It also
//! cross-checks each documented key's own scope, catching a row moved to the
//! wrong table (e.g. a global-only key documented under "Buffer options").

use super::{first_cell_keys, section_after};

/// Fail oracle: add a new entry to `define_settings!` (any section) or
/// to `user-manual/docs/configuration.md`'s option tables without the
/// matching change on the other side — this test fails naming the key
/// and which direction it's missing.
#[test]
fn user_manual_option_tables_match_all_setting_keys() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");
    let manual_path = std::path::Path::new(&manifest).join("../user-manual/docs/configuration.md");
    let text = std::fs::read_to_string(&manual_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", manual_path.display()));

    let global_documented = first_cell_keys(section_after(&text, "## Global options"));
    let buffer_documented = first_cell_keys(section_after(&text, "## Buffer options"));
    let mut documented = global_documented.clone();
    documented.extend(buffer_documented.clone());

    let mut code_keys: std::collections::BTreeSet<String> = crate::settings::all_setting_keys()
        .iter()
        .map(|k| k.to_string())
        .collect();
    // "language" has no define_settings! entry by design (see
    // settings.rs's module doc) but is documented in the Buffer options
    // table, so it's added here rather than to all_setting_keys() itself.
    code_keys.insert("language".to_string());

    let missing_from_docs: Vec<_> = code_keys.difference(&documented).collect();
    let stale_in_docs: Vec<_> = documented.difference(&code_keys).collect();

    // A key present in *some* table isn't necessarily under the *right*
    // one — a name-set diff alone can't catch a row filed under the
    // wrong heading. Cross-check each documented key's own table
    // against its declared scope.
    use crate::settings::Scope;
    let misplaced: Vec<String> = global_documented
        .iter()
        .filter(|k| k.as_str() != "language")
        .filter(|k| !crate::settings::setting_scopes(k).contains(&Scope::Global))
        .map(|k| format!("'{k}' is under Global options but its scope list has no Global"))
        .chain(
            buffer_documented
                .iter()
                .filter(|k| k.as_str() != "language") // buffer-only by special case, no scope entry
                .filter(|k| !crate::settings::setting_scopes(k).contains(&Scope::Buffer))
                .map(|k| format!("'{k}' is under Buffer options but its scope list has no Buffer")),
        )
        .collect();

    assert!(
        missing_from_docs.is_empty() && stale_in_docs.is_empty() && misplaced.is_empty(),
        "\nuser-manual/docs/configuration.md option tables drifted from \
         settings::all_setting_keys().\n\
         In code but missing from the docs tables: {missing_from_docs:?}\n\
         In the docs tables but not a real setting key: {stale_in_docs:?}\n\
         Documented under the wrong heading: {misplaced:?}\n"
    );
}
