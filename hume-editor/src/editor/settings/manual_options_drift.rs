//! # User-manual option-table drift
//!
//! `user-manual/docs/settings.md`'s "Global options"/"Buffer options"
//! tables are a hand-maintained mirror of `SettingKey::all()`. Nothing else
//! keeps the two in sync; `user_manual_option_tables_match_every_setting`
//! scans both tables for every backtick-quoted first-column key and diffs
//! the set against the code's key list in both directions, catching a key
//! added to `define_settings!` without a manual row (or vice versa). It also
//! cross-checks each documented key's own scope, catching a row moved to the
//! wrong table (e.g. a global-only key documented under "Buffer options").
//!
//! Lives here rather than in `arch-lints` (the workspace's other
//! architectural lints, `absent_decode`/`init_example`/etc.): those scan
//! source text as strings and never link against the crates they check, but
//! this lint calls `SettingKey`/`Scope` directly, while
//! `arch-lints` has zero dependencies by design (see its own module doc),
//! never constructing an `Editor` or linking against `hume-editor` at all;
//! moving this lint there would mean adding that dependency, destroying the
//! property the crate exists for. Not worth it for one lint that already
//! has a natural home next to the settings it inspects.

/// The text between the first occurrence of `heading` and the next
/// top-level (`\n## `) heading, used to scope a key scan to just one
/// section of a markdown doc, not the whole file.
fn section_after<'a>(text: &'a str, heading: &str) -> &'a str {
    let start = text
        .find(heading)
        .unwrap_or_else(|| panic!("heading '{heading}' not found in settings.md"));
    let after = &text[start + heading.len()..];
    let end = after.find("\n## ").unwrap_or(after.len());
    &after[..end]
}

/// Every `` `key` `` in a markdown table's leading cell: a line trimmed
/// to start with `` | ` `` (no other content in this file's tables looks
/// like that once scoped to one `## `-delimited section).
fn first_cell_keys(section: &str) -> std::collections::BTreeSet<String> {
    section
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("| `")?;
            let (key, _) = rest.split_once('`')?;
            (!key.is_empty()).then(|| key.to_string())
        })
        .collect()
}

/// Every key in `define_settings!` (any section) must appear in
/// `user-manual/docs/settings.md`'s option tables, and every table key
/// must be a real setting. A failure names the key and the side it is
/// missing from.
#[test]
fn user_manual_option_tables_match_every_setting() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set: run via `cargo test`");
    let manual_path = std::path::Path::new(&manifest).join("../user-manual/docs/settings.md");
    let text = std::fs::read_to_string(&manual_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", manual_path.display()));

    let global_documented = first_cell_keys(section_after(&text, "## Global options"));
    let buffer_documented = first_cell_keys(section_after(&text, "## Buffer options"));
    let mut documented = global_documented.clone();
    documented.extend(buffer_documented.clone());

    let code_keys: std::collections::BTreeSet<String> = super::SettingKey::all()
        .map(|setting| setting.name().to_string())
        .collect();

    let missing_from_docs: Vec<_> = code_keys.difference(&documented).collect();
    let stale_in_docs: Vec<_> = documented.difference(&code_keys).collect();

    // A key present in *some* table isn't necessarily under the *right*
    // one: a name-set diff alone can't catch a row filed under the
    // wrong heading. Cross-check each documented key's own table
    // against the scopes its section gives it.
    use super::{Scope, SettingKey};
    let misplaced: Vec<String> = [
        (&global_documented, Scope::Global, "Global"),
        (&buffer_documented, Scope::Buffer, "Buffer"),
    ]
    .into_iter()
    .flat_map(|(keys, scope, heading)| {
        keys.iter()
            .filter(move |k| SettingKey::named(k).and_then(|s| s.target(scope)).is_none())
            .map(move |k| format!("'{k}' is under {heading} options but has no {scope} scope"))
    })
    .collect();

    assert!(
        missing_from_docs.is_empty() && stale_in_docs.is_empty() && misplaced.is_empty(),
        "\nuser-manual/docs/settings.md option tables drifted from \
         SettingKey::all().\n\
         In code but missing from the docs tables: {missing_from_docs:?}\n\
         In the docs tables but not a real setting key: {stale_in_docs:?}\n\
         Documented under the wrong heading: {misplaced:?}\n"
    );
}
