//! Helix-compatible TOML theme loader.
//!
//! Supports:
//! - `inherits = "parent"` — the parent and child TOML documents (`[palette]`
//!   included) are merged before anything is resolved to a color, so a
//!   child's palette override reaches every scope that references it, not
//!   just the scopes the child redeclares. Child wins on conflict, in both
//!   the palette and the scope table.
//! - `[palette]` — named-color indirection; palette names are resolved after
//!   the whole `inherits` chain is merged, so they appear as `fg`/`bg` values
//!   in scope entries just once, from the final merged document
//! - The sixteen ANSI terminal color names (`red`, `light-gray`, …) as a
//!   fallback for any color value not found in the palette — see
//!   `ANSI_COLORS`
//! - Flat dotted keys: `"keyword.function" = { fg = "red", modifiers = ["bold"] }`
//! - Real TOML section headers: `[keyword.function]` / `fg = "red"` is
//!   equivalent to the flat form above — promoted to the same dotted scope
//!   name before anything else runs, matching the theme editor's own
//!   `walkScopes` (`tools/theme-editor/src/lib/toml.js`)
//! - Shorthand string values: `"keyword" = "red"` sets `fg` from the named color
//!
//! Every error names the theme file it came from, tracing back through an
//! `inherits` chain to the document that actually defined the offending key.
//!
//! A malformed *entry* — a bad color, an unknown modifier, a scope value in
//! the wrong shape (Helix's `rainbow` bracket array, for instance) — doesn't
//! fail the load. Helix collects these as warnings and gives the offending
//! key a default style rather than discarding the whole theme, and this
//! loader does the same (see `resolve_theme_table`'s doc for exactly what
//! stays fatal instead).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};

use hume_grid::Rgb;

use crate::theme::Theme;
use crate::theme::error::ThemeError;
use crate::types::{Modifiers, ResolvedStyle, UnderlineStyle};

const MAX_DEPTH: usize = 8;

/// Top-level keys that carry loader configuration rather than a scope entry.
/// `validate_reserved_keys` and `merge_raw_themes` still name each one
/// individually — they need per-key type expectations and merge behavior —
/// but every plain "is this a scope key?" test below goes through
/// [`is_reserved`] so a third reserved key needs only one new entry here.
const RESERVED_KEYS: [&str; 2] = ["inherits", "palette"];

fn is_reserved(key: &str) -> bool {
    RESERVED_KEYS.contains(&key)
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// A theme resolved from a TOML document, plus a warning for every entry
/// that was malformed on its own. A non-empty `warnings` doesn't mean the
/// load failed — [`Theme`] is always fully usable — it means some key in
/// the document didn't come through and now carries an empty style instead.
/// See `resolve_theme_table`'s doc for exactly which problems fail the
/// load rather than landing here.
pub struct LoadedTheme {
    /// Fully usable regardless of `warnings` — a malformed entry lands with
    /// an empty style, never with a missing or half-built one.
    pub theme: Theme,
    /// One entry per malformed key found while resolving `theme`. Empty for
    /// a theme that loaded cleanly.
    pub warnings: Vec<ThemeError>,
}

/// Load a theme by name from the given ordered search paths.
///
/// `search_paths` is searched in order; the first `<name>.toml` file found
/// wins — except when resolving an `inherits` parent that names a
/// higher-priority file already visited earlier in the chain (a theme
/// shadowing one of the same name), in which case that candidate is skipped
/// in favor of the next. Child scopes override parent scopes from `inherits`
/// chains.
///
/// Returns a fully-resolved, un-baked [`Theme`] wrapped in a [`LoadedTheme`]
/// — check its `warnings` if you want to surface them. Call [`Theme::bake`]
/// with the live [`crate::theme::ScopeRegistry`] before the first render.
pub fn load_theme(name: &str, search_paths: &[PathBuf]) -> Result<LoadedTheme, ThemeError> {
    let mut visited: FxHashSet<PathBuf> = FxHashSet::default();
    let raw = load_raw_recursive(name, search_paths, &mut visited, 0, None)?;
    Ok(resolve_theme_table(raw))
}

/// Parse a theme from a TOML string.
///
/// Supports palette indirection and all scope value forms, but does **not**
/// support `inherits` — the document must be a self-contained leaf. Passing a
/// document with a string `inherits` returns [`ThemeError::NotFound`] for the
/// named parent; a non-string `inherits` returns [`ThemeError::BadReservedKey`]
/// instead, same as [`load_theme`].
///
/// Intended for embedded themes (e.g. `include_str!` in the binary). Has no
/// file of its own, so its errors — fatal or collected as [`LoadedTheme::warnings`]
/// — are never wrapped in [`ThemeError::InFile`].
pub fn parse_theme(toml_str: &str) -> Result<LoadedTheme, ThemeError> {
    let mut visited: FxHashSet<PathBuf> = FxHashSet::default();
    // Empty search_paths: any `inherits` key will fail with NotFound, which is
    // the correct behaviour for a self-contained embedded document.
    let raw = parse_raw_recursive(toml_str, None, &[], &mut visited, 0)?;
    Ok(resolve_theme_table(raw))
}

// ---------------------------------------------------------------------------
// Recursive loader
// ---------------------------------------------------------------------------
//
// An `inherits` chain is merged as raw TOML — parent and child tables, palette
// included — and resolved to colors exactly once, after the whole chain is
// flat. This is Helix's own model (`helix-view::theme::merge_themes` +
// `helix-loader::merge_toml_values`): a child's palette override must reach
// every scope that references it, including ones the child never redeclares,
// which is how a real Helix light/dark theme pair works (the light variant
// overrides a handful of palette names and nothing else). Resolving per level
// instead — the parent to concrete colors, then only letting the child's own
// palette recolor scopes it explicitly restates — cannot support that: the
// parent's palette is gone by the time the child is parsed.

/// A merged-but-unresolved theme document, plus per-key file provenance.
///
/// Built up through an `inherits` chain by `merge_raw_themes`, in parallel
/// with the table itself, so a resolve-time error can name the document that
/// actually defined the offending entry — information the merge would
/// otherwise erase. Scope keys and `[palette]` names are tracked separately
/// because they merge separately: a child overriding one palette name leaves
/// every other name pointing at the file that did define it. Both empty for
/// an embedded ([`parse_theme`]) document, which has no file to attribute to.
///
/// `warnings` carries flatten-time warnings (a misspelled style attribute
/// found by `walk_scope`) already attributed to this document's file — they
/// happen before `origins`/`palette_origins` exist, so they can't wait to be
/// attributed alongside `resolve_theme_table`'s own warnings the way a bad
/// color or modifier does.
struct RawTheme {
    table: toml::Table,
    origins: FxHashMap<String, Arc<Path>>,
    palette_origins: FxHashMap<String, Arc<Path>>,
    warnings: Vec<ThemeError>,
}

fn load_raw_recursive(
    name: &str,
    search_paths: &[PathBuf],
    visited: &mut FxHashSet<PathBuf>,
    depth: usize,
    requested_by: Option<&Path>,
) -> Result<RawTheme, ThemeError> {
    if depth > MAX_DEPTH {
        return Err(attribute(
            ThemeError::MaxDepth {
                name: name.to_owned(),
            },
            requested_by,
        ));
    }

    let (source, path) =
        find_theme_file(name, search_paths, visited).map_err(|e| attribute(e, requested_by))?;
    parse_raw_recursive(&source, Some(&path), search_paths, visited, depth)
}

/// Parse one TOML document, merging in its `inherits` parent (via
/// `load_raw_recursive`, when `search_paths` is non-empty) before returning —
/// the result carries no unresolved `inherits` chain of its own.
///
/// `path` is this document's own file, when it has one (`None` for an
/// embedded [`parse_theme`] string) — used both to attribute this document's
/// own parse/validation errors and, if it declares `inherits`, as the
/// requester attributed to a failure resolving its parent.
fn parse_raw_recursive(
    source: &str,
    path: Option<&Path>,
    search_paths: &[PathBuf],
    visited: &mut FxHashSet<PathBuf>,
    depth: usize,
) -> Result<RawTheme, ThemeError> {
    let table: toml::Table = source
        .parse()
        .map_err(|e| attribute(ThemeError::Parse(e), path))?;
    validate_reserved_keys(&table).map_err(|e| attribute(e, path))?;
    let mut flatten_warnings = Vec::new();
    let table = flatten_scopes(table, &mut flatten_warnings);
    let warnings = flatten_warnings
        .into_iter()
        .map(|e| attribute(e, path))
        .collect();

    let origins = origins_for(table.keys().filter(|k| !is_reserved(k)), path);
    let palette_origins = origins_for(
        table
            .get("palette")
            .and_then(|v| v.as_table())
            .into_iter()
            .flat_map(|t| t.keys()),
        path,
    );
    let raw = RawTheme {
        table,
        origins,
        palette_origins,
        warnings,
    };

    match raw.table.get("inherits").and_then(|v| v.as_str()) {
        Some(parent_name) => {
            let parent_name = parent_name.to_owned();
            let parent_raw =
                load_raw_recursive(&parent_name, search_paths, visited, depth + 1, path)?;
            Ok(merge_raw_themes(parent_raw, raw))
        }
        None => Ok(raw),
    }
}

/// Wrap `err` as having come from `path`, when known — the identity a
/// recursive lookup's caller (`requested_by`) or a document's own parse step
/// already has in hand. `None` (an embedded document with no file of its
/// own) leaves `err` unwrapped.
fn attribute(err: ThemeError, path: Option<&Path>) -> ThemeError {
    match path {
        Some(p) => ThemeError::InFile {
            path: p.to_path_buf(),
            error: Box::new(err),
        },
        None => err,
    }
}

/// Map each of `keys` to `path` — one `Arc` shared by every entry, since a
/// document has exactly one identity. `None` (an embedded document) produces
/// an empty map: nothing to attribute to.
fn origins_for<'a>(
    keys: impl Iterator<Item = &'a String>,
    path: Option<&Path>,
) -> FxHashMap<String, Arc<Path>> {
    let Some(path) = path else {
        return FxHashMap::default();
    };
    let origin: Arc<Path> = Arc::from(path);
    keys.map(|k| (k.clone(), Arc::clone(&origin))).collect()
}

/// Rejects a reserved top-level key with the wrong TOML value type, checked
/// once per document before any merge — so an error names the document that
/// is actually malformed, rather than surfacing later as a `BadColor` on
/// some unrelated inherited scope once a merge has silently discarded the
/// key's real content.
fn validate_reserved_keys(table: &toml::Table) -> Result<(), ThemeError> {
    if let Some(v) = table.get("inherits")
        && v.as_str().is_none()
    {
        return Err(ThemeError::BadReservedKey {
            key: "inherits",
            expected: "a string",
        });
    }
    if let Some(v) = table.get("palette")
        && v.as_table().is_none()
    {
        return Err(ThemeError::BadReservedKey {
            key: "palette",
            expected: "a table",
        });
    }
    Ok(())
}

/// Merge a child [`RawTheme`] onto its parent's, child wins.
///
/// `[palette]` merges key-by-key — a child's named color overrides the
/// parent's same-named one, and non-conflicting names from both sides
/// survive. Every other top-level key is a scope entry, and the child's
/// value replaces the parent's *wholesale* when present, rather than
/// merging field-by-field: matching Helix's own merge (depth exhausted below
/// the top level, `merge_toml_values(_, _, 1)`), and matching what a partial
/// override table means to a theme author — `{ bg = "bg2" }` is meant to
/// *become* the scope's new style, not patch one field of the parent's.
///
/// Both origin maps merge in lockstep with the table so provenance stays
/// correct: an entry the child overrides now points at the child's file, and
/// every entry it leaves alone still points at whichever ancestor defined it.
fn merge_raw_themes(parent: RawTheme, child: RawTheme) -> RawTheme {
    let mut table = parent.table;
    let mut child_table = child.table;

    // Spent — a merged document carries no inherits chain of its own.
    child_table.remove("inherits");

    // `validate_reserved_keys` has already rejected a non-table `palette` on
    // either side, so both `Value::Table` matches below are infallible.
    let merged_palette = match (table.remove("palette"), child_table.remove("palette")) {
        (Some(toml::Value::Table(mut p)), Some(toml::Value::Table(c))) => {
            for (k, v) in c {
                p.insert(k, v);
            }
            Some(toml::Value::Table(p))
        }
        (p, c) => c.or(p),
    };

    for (key, value) in child_table {
        table.insert(key, value);
    }
    if let Some(palette) = merged_palette {
        table.insert("palette".to_owned(), palette);
    }

    let mut origins = parent.origins;
    origins.extend(child.origins);
    let mut palette_origins = parent.palette_origins;
    palette_origins.extend(child.palette_origins);
    let mut warnings = parent.warnings;
    warnings.extend(child.warnings);
    RawTheme {
        table,
        origins,
        palette_origins,
        warnings,
    }
}

/// Resolve one fully-merged theme document into a [`LoadedTheme`]. Called
/// once, after any `inherits` chain has already been flattened into `table`
/// — so a child's palette override is visible to every scope that
/// references it, including ones neither the child nor any intermediate
/// ancestor restates.
///
/// A malformed palette entry or scope entry is collected into `warnings` and
/// given a stand-in (the palette entry is dropped; the scope gets a default,
/// all-`None` style) rather than failing the whole load — matching Helix's
/// own `build_theme_values`, which keeps the theme and warns on a bad key
/// rather than discarding the file over it. This is a deliberate exception to
/// this project's fail-fast default: a theme is content someone is actively
/// trying to get onto their screen, and one mistyped color, or a scope Helix
/// has that HUME doesn't fully support (the `rainbow` bracket array, say),
/// shouldn't cost them the rest of an otherwise-good theme.
///
/// Everything that stays fatal instead is a problem with the *document*, not
/// one entry in it — unparseable TOML, a missing or cyclic `inherits`
/// parent, a malformed `inherits`/`palette` key. All of those are raised
/// earlier, in `parse_raw_recursive`/`load_raw_recursive`, before this
/// function ever runs — there is no partial document to warn-and-continue
/// from. A misspelled style attribute (`walk_scope`'s own malformed entry) is
/// collected as a warning the same way, just earlier — see `RawTheme::warnings`.
fn resolve_theme_table(raw: RawTheme) -> LoadedTheme {
    let RawTheme {
        table,
        origins,
        palette_origins,
        mut warnings,
    } = raw;

    // ── Parse [palette] (if any) ──────────────────────────────────────────────
    // Palette entries must be #rgb/#rrggbb literals — matching where Helix
    // itself draws the line: its `ThemePalette::try_from` parses each palette
    // value before the built-in ANSI names are merged in, so a name here
    // (`red = "red"`) doesn't resolve there either. A malformed entry is
    // warned and recorded as `None` (declared but broken) rather than simply
    // omitted — omitting it would let a scope referencing the same name fall
    // through to an ANSI/hex match instead of getting its own cascading
    // warning from `resolve_color` below.
    let mut palette: FxHashMap<String, Option<Rgb>> = FxHashMap::default();
    if let Some(pal_table) = table.get("palette").and_then(|v| v.as_table()) {
        for (k, v) in pal_table {
            let origin = palette_origins.get(k).map(|p| &**p);
            let hex = match v.as_str() {
                Some(s) => s,
                None => {
                    warnings.push(attribute(bad_style_field("palette", k, "a string"), origin));
                    palette.insert(k.clone(), None);
                    continue;
                }
            };
            match parse_hex_color(hex) {
                Ok(color) => {
                    palette.insert(k.clone(), Some(color));
                }
                Err(_) => {
                    warnings.push(attribute(
                        ThemeError::BadColor {
                            key: format!("palette.{k}"),
                            field: None,
                            value: hex.to_owned(),
                        },
                        origin,
                    ));
                    palette.insert(k.clone(), None);
                }
            }
        }
    }

    // ── Parse scope entries ───────────────────────────────────────────────────
    // A malformed entry is warned and given a default (all-`None`) style
    // rather than omitted, so it still blocks the dot-notation fallback chain
    // the way a real entry would — an explicit `keyword.function`, even a
    // broken one, must not silently fall through to `keyword`'s style. A style
    // *table* with only one bad field is the partial exception: the fields
    // that did parse are kept (see `parse_style_table`), so only a shorthand
    // string or a wrong-shaped value (`parse_scope_value`'s `Err` arm) falls
    // back to a fully empty style.
    let mut scopes: FxHashMap<String, ResolvedStyle> = FxHashMap::default();
    for (key, value) in &table {
        // Reserved keys — not scope entries.
        if is_reserved(key) {
            continue;
        }
        let origin = origins.get(key).map(|p| &**p);
        let style = match parse_scope_value(key, value, &palette) {
            Ok((style, field_warnings)) => {
                warnings.extend(field_warnings.into_iter().map(|e| attribute(e, origin)));
                style
            }
            Err(e) => {
                warnings.push(attribute(e, origin));
                ResolvedStyle::default()
            }
        };
        scopes.insert(key.clone(), style);
    }

    // `ui.text` (Helix's base-foreground convention) folds into `default`, the
    // style every cell starts from (see `style::apply_styles`). Without this,
    // plain/unhighlighted text has `fg: None` → renders as the terminal's own
    // default colour, which the pane dim has no numeric value to blend and so
    // leaves at full strength in an unfocused pane.
    let default = scopes.get("ui.text").copied().unwrap_or_default();

    LoadedTheme {
        theme: Theme::from_owned(scopes, default),
        warnings,
    }
}

// ---------------------------------------------------------------------------
// Scope flattening
// ---------------------------------------------------------------------------

/// Style-table fields — a table keyed by any of these is a scope's style,
/// never a container to recurse into.
const STYLE_KEYS: [&str; 4] = ["fg", "bg", "underline", "modifiers"];

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
fn flatten_scopes(table: toml::Table, warnings: &mut Vec<ThemeError>) -> toml::Table {
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

// ---------------------------------------------------------------------------
// Scope value parsing
// ---------------------------------------------------------------------------

/// Parse one TOML scope entry into a `ResolvedStyle`, plus a warning for
/// every individually malformed field inside a style table (the `Ok` arm's
/// second element — empty for a clean table). Helix supports two forms:
/// - `"keyword" = "red"` — shorthand; sets `fg` only
/// - `"keyword" = { fg = "red", bg = "black", modifiers = ["bold"] }` — full form
///
/// Only the shorthand string form and an outright wrong-shaped value
/// (`BadScopeValue` — neither a string nor a table) fail outright: a
/// shorthand color is the entry's only field, so nothing partial survives a
/// bad one, and a value with no style table to draw fields from has nothing
/// to salvage either.
fn parse_scope_value(
    key: &str,
    value: &toml::Value,
    palette: &FxHashMap<String, Option<Rgb>>,
) -> Result<(ResolvedStyle, Vec<ThemeError>), ThemeError> {
    match value {
        // Shorthand: `"keyword" = "red"` sets fg only.
        toml::Value::String(s) => {
            let fg = Some(resolve_color(key, None, s, palette)?);
            Ok((
                ResolvedStyle {
                    fg,
                    ..Default::default()
                },
                Vec::new(),
            ))
        }
        toml::Value::Table(t) => Ok(parse_style_table(key, t, palette)),
        other => Err(ThemeError::BadScopeValue {
            key: key.to_owned(),
            value: format!("{other:?}"),
        }),
    }
}

fn bad_style_field(key: &str, field: &str, expected: &'static str) -> ThemeError {
    ThemeError::BadStyleField {
        key: key.to_owned(),
        field: field.to_owned(),
        expected,
    }
}

/// Read `t.get(lookup)` as a string and run `parse` on it, warning (into
/// `warnings`) and returning `None` if the TOML value isn't a string or
/// `parse` itself rejects it — the shared shape behind `fg`/`bg`/
/// `underline.color`/`underline.style`, which differ only in `lookup`'s key
/// (`"color"`/`"style"` inside the nested `underline` table, same as
/// `display` everywhere else) and what `parse` does with the string. `None`
/// on the absent-key path too, so every caller can write `if let Some(v) =
/// str_field(...) { style.x = v }` uniformly instead of branching on
/// presence itself.
fn str_field<T>(
    key: &str,
    t: &toml::map::Map<String, toml::Value>,
    lookup: &str,
    display: &'static str,
    parse: impl FnOnce(&str) -> Result<T, ThemeError>,
    warnings: &mut Vec<ThemeError>,
) -> Option<T> {
    let v = t.get(lookup)?;
    match v.as_str() {
        Some(s) => match parse(s) {
            Ok(val) => Some(val),
            Err(e) => {
                warnings.push(e);
                None
            }
        },
        None => {
            warnings.push(bad_style_field(key, display, "a string"));
            None
        }
    }
}

/// Parse a scope's style table field by field. A malformed field is warned
/// and left unset rather than discarding the fields around it — matching
/// Helix's own `build_theme_values`, which keeps a partially-built style
/// rather than throwing it away over one bad key. A bad item inside
/// `modifiers` gets the same treatment at the item level: the valid items
/// beside it still apply.
fn parse_style_table(
    key: &str,
    t: &toml::map::Map<String, toml::Value>,
    palette: &FxHashMap<String, Option<Rgb>>,
) -> (ResolvedStyle, Vec<ThemeError>) {
    let mut style = ResolvedStyle::default();
    let mut warnings = Vec::new();

    if let Some(c) = str_field(
        key,
        t,
        "fg",
        "fg",
        |s| resolve_color(key, None, s, palette),
        &mut warnings,
    ) {
        style.fg = Some(c);
    }
    if let Some(c) = str_field(
        key,
        t,
        "bg",
        "bg",
        |s| resolve_color(key, None, s, palette),
        &mut warnings,
    ) {
        style.bg = Some(c);
    }
    if let Some(v) = t.get("underline") {
        if let Some(s) = v.as_str() {
            match parse_underline(key, s) {
                Ok(u) => style.underline = u,
                Err(e) => warnings.push(e),
            }
        } else if let Some(ut) = v.as_table() {
            // `underline = { color = "#...", style = "..." }` (Helix extended form)
            if let Some(c) = str_field(
                key,
                ut,
                "color",
                "underline.color",
                |s| resolve_color(key, Some("underline.color"), s, palette),
                &mut warnings,
            ) {
                style.underline_color = Some(c);
            }
            if let Some(u) = str_field(
                key,
                ut,
                "style",
                "underline.style",
                |s| parse_underline(key, s),
                &mut warnings,
            ) {
                style.underline = u;
            }
        } else {
            warnings.push(bad_style_field(key, "underline", "a string or a table"));
        }
    }
    if let Some(v) = t.get("modifiers") {
        match v.as_array() {
            Some(arr) => {
                for item in arr {
                    match item.as_str() {
                        Some(s) => match s {
                            // Helix exposes underline as a modifier; route it to the
                            // dedicated underline field so underline has a single
                            // source of truth. A more specific `underline = "..."`
                            // key (parsed above) wins.
                            "underlined" => {
                                if style.underline == UnderlineStyle::None {
                                    style.underline = UnderlineStyle::Solid;
                                }
                            }
                            _ => match parse_modifier(key, s) {
                                Ok(m) => style.modifiers |= m,
                                Err(e) => warnings.push(e),
                            },
                        },
                        None => {
                            warnings.push(bad_style_field(key, "modifiers", "an array of strings"))
                        }
                    }
                }
            }
            None => warnings.push(bad_style_field(key, "modifiers", "an array")),
        }
    }

    (style, warnings)
}

// ---------------------------------------------------------------------------
// Colour resolution
// ---------------------------------------------------------------------------

/// `field` is passed straight to [`ThemeError::BadColor`] — `None` for a
/// scope's own `fg`/`bg`/shorthand, `Some` for the nested `underline.color`,
/// whose failure `key` alone can't distinguish from those.
fn resolve_color(
    key: &str,
    field: Option<&'static str>,
    s: &str,
    palette: &FxHashMap<String, Option<Rgb>>,
) -> Result<Rgb, ThemeError> {
    // Palette reference takes priority. A palette entry that was itself
    // malformed (`Some(None)`, already warned when `[palette]` was parsed)
    // must not fall through to an ANSI/hex guess at the same name — that
    // would silently recolor every referencing scope instead of cascading
    // the warning.
    match palette.get(s) {
        Some(Some(color)) => return Ok(*color),
        Some(None) => {
            return Err(ThemeError::BadColor {
                key: key.to_owned(),
                field,
                value: s.to_owned(),
            });
        }
        None => {}
    }
    // Built-in ANSI name — a theme's own palette entry of the same name
    // (checked above) overrides it, matching how Helix's `ThemePalette::new`
    // merges a theme's palette over its default name map.
    if let Some(&(_, color)) = ANSI_COLORS.iter().find(|(name, _)| *name == s) {
        return Ok(color);
    }
    // Hex literal.
    parse_hex_color(s).map_err(|_| ThemeError::BadColor {
        key: key.to_owned(),
        field,
        value: s.to_owned(),
    })
}

/// The sixteen terminal colour names Helix themes may use in place of a hex
/// literal, resolved to fixed RGB values rather than the terminal's actual
/// configured colours.
///
/// HUME requires a numeric [`Rgb`] to blend for the inactive-pane dim
/// ([`Rgb::lerp`](hume_grid::Rgb::lerp)), and there is no way to ask a
/// terminal what its `red` currently renders as: `termina` only models OSC
/// 10-19 (cursor/foreground/background), not the OSC 4 palette-query xterm
/// extension a name like `red` would need. So a name means the same fixed
/// colour on every terminal, rather than tracking a user's own palette
/// customization the way the *terminal's* rendering of `red` would.
///
/// Values are the xterm default palette — verified against Helix's own
/// `Color` → backend conversions (`helix-view/src/graphics.rs`), which agree
/// on every index across both its crossterm and termina impls. Note the
/// index order: `gray` is 8 (bright black) and `light-gray` is 7 (the
/// non-bright palette's *white* slot) — Helix's mapping, not a typo here.
const ANSI_COLORS: [(&str, Rgb); 16] = [
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

fn parse_hex_color(s: &str) -> Result<Rgb, ()> {
    let hex = s.strip_prefix('#').ok_or(())?;
    // `from_str_radix` accepts a leading sign, which would make `#+f0000`
    // parse as `Rgb(15, 0, 0)`. Nothing but hex digits is a colour here.
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(());
    }
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| ())?;
            let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| ())?;
            let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| ())?;
            Ok(Rgb(r, g, b))
        }
        3 => {
            // Expand shorthand #rgb → #rrggbb.
            let r = u8::from_str_radix(&hex[0..1], 16).map_err(|_| ())?;
            let g = u8::from_str_radix(&hex[1..2], 16).map_err(|_| ())?;
            let b = u8::from_str_radix(&hex[2..3], 16).map_err(|_| ())?;
            Ok(Rgb(r * 17, g * 17, b * 17))
        }
        _ => Err(()),
    }
}

// ---------------------------------------------------------------------------
// Modifier parsing
// ---------------------------------------------------------------------------

fn parse_modifier(key: &str, s: &str) -> Result<Modifiers, ThemeError> {
    match s {
        "bold" => Ok(Modifiers::BOLD),
        "italic" => Ok(Modifiers::ITALIC),
        "crossed_out" => Ok(Modifiers::STRIKETHROUGH),
        "dim" => Ok(Modifiers::DIM),
        "reversed" => Ok(Modifiers::REVERSED),
        "hidden" => Ok(Modifiers::HIDDEN),
        "slow_blink" => Ok(Modifiers::SLOW_BLINK),
        "rapid_blink" => Ok(Modifiers::RAPID_BLINK),
        // Treat unrecognized modifiers as errors so themes don't silently lose styling.
        _ => Err(ThemeError::BadModifier {
            key: key.to_owned(),
            value: s.to_owned(),
        }),
    }
}

fn parse_underline(key: &str, s: &str) -> Result<UnderlineStyle, ThemeError> {
    match s {
        "line" => Ok(UnderlineStyle::Solid),
        "curl" => Ok(UnderlineStyle::Wavy),
        "dotted" => Ok(UnderlineStyle::Dotted),
        "dashed" => Ok(UnderlineStyle::Dashed),
        "double_line" => Ok(UnderlineStyle::Double),
        _ => Err(ThemeError::BadUnderline {
            key: key.to_owned(),
            value: s.to_owned(),
        }),
    }
}

// ---------------------------------------------------------------------------
// File discovery
// ---------------------------------------------------------------------------

/// A theme name safe to use as one filesystem path segment: non-empty, no
/// `.`/`..`, no path separator, no NUL, and no `:` or `"`.
///
/// The `:` rejection matters on Windows specifically: a name like `c:evil`
/// makes `PathBuf::push` treat it as a drive-relative root, replacing the
/// search directory entirely instead of joining onto it. NUL and the empty
/// string are rejected here rather than left to the filesystem so both come
/// back as "no such theme": an empty name would otherwise probe for a hidden
/// `.toml` in every search dir, and a NUL surfaces from `read_to_string` as
/// `ErrorKind::InvalidInput`, which the search loop doesn't skip on and would
/// report as an I/O failure.
///
/// Accepts exactly the same set as `hume_platform::path::is_safe_segment` and
/// `core:stdlib`'s `stdlib/safe-path-segment?` (`runtime/plugins/core/stdlib/plugin.scm`).
/// Kept as its own copy because `hume-engine` deliberately depends on no
/// platform layer — taking one for a six-line predicate would pull `termina`
/// and `nix` into the renderer and everything downstream of it.
fn is_safe_theme_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c != '/' && c != '\\' && c != '"' && c != '\0' && c != ':')
}

/// Finds and reads the first `<name>.toml` in `search_paths` not already in
/// `visited`, inserting its canonical path into `visited` before returning
/// its source alongside that same canonical path (the file's identity for
/// error attribution).
///
/// A candidate whose canonical path is already in `visited` is a cycle
/// through *that specific file* — but not necessarily through `name`: a
/// config-dir theme shadowing a bundled theme of the same name legitimately
/// `inherits`s the bundled (lower-priority, distinct-file) copy, so a match
/// on the first, higher-priority candidate must not end the search. Skip and
/// keep scanning (matching Helix's own `Loader::path`); only report `Cycle`
/// once every candidate has been exhausted this way.
///
/// Reads each candidate first (matching on `NotFound` to skip to the next
/// search dir), then canonicalizes the path for the visited-check. Canonicalize
/// failure after a successful read uses the unresolved path as the cycle key —
/// safe because a deleted-after-read file cannot form a cycle.
///
/// A candidate that exists but can't be read for some other reason (a
/// directory left in its place, a permissions error) is likewise skipped
/// rather than aborting the whole search: search order is a priority list,
/// and one broken higher-priority candidate must not shadow a working
/// lower-priority one — most concretely the bundled copy a config-dir theme
/// of the same name would otherwise make unreachable. The first such error is
/// remembered and only reported if nothing later in the list works either, so
/// a real problem still surfaces instead of silently becoming `NotFound`.
fn find_theme_file(
    name: &str,
    search_paths: &[PathBuf],
    visited: &mut FxHashSet<PathBuf>,
) -> Result<(String, PathBuf), ThemeError> {
    if !is_safe_theme_name(name) {
        return Err(ThemeError::NotFound {
            name: name.to_owned(),
        });
    }
    let filename = format!("{name}.toml");
    let mut cycle_found = false;
    let mut io_error = None;
    for dir in search_paths {
        let candidate = dir.join(&filename);
        match std::fs::read_to_string(&candidate) {
            Ok(source) => {
                // Canonicalize after read — residual race only affects cycle-key
                // accuracy, not file content. Not a security prefix check.
                let canonical =
                    std::fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());
                if visited.insert(canonical.clone()) {
                    return Ok((source, canonical));
                }
                cycle_found = true;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                io_error.get_or_insert(ThemeError::Io {
                    name: name.to_owned(),
                    path: candidate,
                    error: e,
                });
            }
        }
    }
    if cycle_found {
        return Err(ThemeError::Cycle {
            name: name.to_owned(),
        });
    }
    if let Some(e) = io_error {
        return Err(e);
    }
    Err(ThemeError::NotFound {
        name: name.to_owned(),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
