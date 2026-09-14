//! Real-TOML-section-header flattening (`[ui.cursor]` -> `"ui.cursor"`).

use crate::theme::error::ThemeError;

use super::{bad_style_field, is_reserved};

/// Style-table fields — a table keyed by any of these is a scope's style,
/// never a container to recurse into.
pub(super) const STYLE_KEYS: [&str; 4] = ["fg", "bg", "underline", "modifiers"];

/// `expected` text for a key found in a style table that isn't a style field.
/// Spells out [`STYLE_KEYS`] for the error message — `expected` is a
/// `&'static str`, so the list can't be joined at runtime; keep the two in
/// step.
const STYLE_KEY_LIST: &str = "one of fg, bg, underline, modifiers";

/// Promote real TOML section headers (`[ui]` / `text = "..."`, `[ui.cursor]`
/// / `fg = ...`) into HUME's flat dotted-key scope names, matching the theme
/// editor's own `walkScopes` (`tools/theme-editor/src/lib/toml.js`) so a
/// theme authored either way loads identically. Runs on each document
/// individually, before an `inherits` merge — merging must compare a
/// parent's flat `"ui.text"` against a child's `[ui]` / `text` on the same
/// key, or which one wins depends on table iteration order instead of the
/// child always winning.
///
/// `warnings` collects a misspelled style attribute found along the way (see
/// `walk_scope`) — flattening itself never fails a load.
///
/// One document may spell the same scope both ways (`"ui.text" = "red"` beside
/// `[ui]` / `text = "blue"`); TOML sees two distinct keys, so this is legal
/// input rather than a duplicate-key error it could reject. The flat key wins:
/// `toml::Table` is a `BTreeMap` here (the crate's `preserve_order` feature is
/// deliberately off — see hume-engine's `Cargo.toml`), so `"ui"` sorts before
/// `"ui.text"` and the flat key's insert lands second. Pinned by test, since
/// it would otherwise be an accident of a dependency's feature flags.
pub(super) fn flatten_scopes(table: toml::Table, warnings: &mut Vec<ThemeError>) -> toml::Table {
    let mut out = toml::Table::new();
    for (key, value) in table {
        if is_reserved(&key) {
            out.insert(key, value);
            continue;
        }
        walk_scope(&mut out, key, value, warnings);
    }
    out
}

/// Recursively flattens one scope's value into `out`, under dotted key
/// `path`. A table's entries split into style fields (kept as `path`'s own
/// style table) and everything else (recursed as `"<path>.<key>"`). `path`
/// is emitted as a scope in its own right whenever it carries a style field,
/// or the table is empty — `"ui.cursor.insert" = {}` deliberately blocks the
/// dot-fallback chain and must not be dropped. A table holding *only*
/// sub-tables (a pure container, e.g. `[ui.cursor]` with just
/// `[ui.cursor.match]` beneath it) emits nothing for `path` itself — this is
/// what keeps the dot-fallback chain from stopping on an empty entry.
///
/// A scalar entry *beside* a style field is dropped rather than recursed:
/// once a table is known to be a style, `underline_style = "curl"` and
/// `text = "#fff"` are indistinguishable, and treating either as a child
/// scope silently invents a name nothing resolves. So it's warned instead —
/// the same treatment `resolve_theme_table` gives every other malformed
/// entry — and the rest of the style table is kept, exactly as one bad field
/// inside a genuine style table (`parse_style_table`) is. A sub-table beside
/// a style field stays a child, since nothing else it could be. The dropped
/// spelling has a flat equivalent that still works: give the child its own
/// top-level `"ui.text"` key.
fn walk_scope(
    out: &mut toml::Table,
    path: String,
    value: toml::Value,
    warnings: &mut Vec<ThemeError>,
) {
    let toml::Value::Table(t) = value else {
        // Shorthand string (or an outright bad type — `parse_scope_value`
        // reports that as `BadScopeValue`) — insert unchanged.
        out.insert(path, value);
        return;
    };

    let mut style = toml::Table::new();
    let mut children = Vec::new();
    for (k, v) in t {
        if STYLE_KEYS.contains(&k.as_str()) {
            style.insert(k, v);
        } else {
            children.push((k, v));
        }
    }

    if !style.is_empty() {
        children.retain(|(k, v)| {
            if v.is_table() {
                true
            } else {
                warnings.push(bad_style_field(&path, k, STYLE_KEY_LIST));
                false
            }
        });
    }

    if !style.is_empty() || children.is_empty() {
        out.insert(path.clone(), toml::Value::Table(style));
    }
    for (k, v) in children {
        walk_scope(out, format!("{path}.{k}"), v, warnings);
    }
}
