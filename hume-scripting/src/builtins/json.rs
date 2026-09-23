//! `(json-parse str)` — general-purpose JSON string decoding for Steel — and
//! the `JsonHandle` accessor builtins (`json-ref`, `json-contains?`,
//! `json-list`, `json-array?`, `json-object?`) every handle-shaped crossing
//! (`hume-scripting/src/json.rs`) reads through.
//!
//! `json-parse` is not LSP-specific: any plugin data pipeline that embeds a
//! JSON blob as a Scheme string literal (rather than reconstructing the same
//! structure as nested Scheme data) needs this to get it back out.
//! `core:lsp`'s seeded server catalog (`registration.scm`) is the first
//! caller — settings are generated as a single canonical JSON string rather
//! than a nested tagged-alist/vector-array Scheme literal.

use std::sync::Arc;

use steel::rvals::SteelVal;

use crate::json::{JsonHandle, Seg, downcast_json_handle, to_steel_handle};

use super::SteelResult;
use super::args::{list_items, string_arg};
use super::errors::generic_err;

/// `(json-parse str)` -> the decoded value, via the same handle funnel
/// external JSON crosses through everywhere else — a container becomes a
/// `JsonHandle` (read with `json-ref`/`json-contains?`/`json-list`), a
/// scalar crosses natively, and top-level `null` is `Void`. No context
/// gate — pure data parsing, callable from init.scm, plugin load, or a
/// command/hook body alike. Raises (does not silently return `#f`) on
/// malformed JSON: a corrupt seeded data file is a build-time bug, not a
/// runtime condition to tolerate.
pub(crate) fn json_parse(s: SteelVal) -> SteelResult {
    let s = string_arg(s, "json-parse")?;
    let value: serde_json::Value =
        serde_json::from_str(&s).map_err(|e| generic_err(format!("json-parse: {e}")))?;
    Ok(to_steel_handle(Arc::new(value)))
}

// ── JsonHandle accessors ──────────────────────────────────────────────────────

fn handle_arg(val: SteelVal, ctx_name: &str) -> Result<JsonHandle, steel::rerrs::SteelErr> {
    downcast_json_handle(&val)
        .ok_or_else(|| generic_err(format!("{ctx_name}: expected a JSON handle, got {val:?}")))
}

/// A path segment: a string (or symbol) object key, or a non-negative
/// integer array index — the same two shapes `json-ref`/`json-contains?`'s
/// variadic Scheme wrappers (`bootstrap.scm`) collect into the list this
/// decodes.
fn seg_arg(val: &SteelVal, ctx_name: &str) -> Result<Seg, steel::rerrs::SteelErr> {
    match val {
        SteelVal::StringV(s) => Ok(Seg::Key(s.as_str().into())),
        SteelVal::SymbolV(s) => Ok(Seg::Key(s.as_str().into())),
        SteelVal::IntV(n) if *n >= 0 => Ok(Seg::Index(*n as usize)),
        _ => steel::stop!(TypeMismatch =>
            "{}: path segment must be a string key or a non-negative integer index", ctx_name),
    }
}

fn path_arg(segs: SteelVal, ctx_name: &str) -> Result<Vec<Seg>, steel::rerrs::SteelErr> {
    list_items(segs, ctx_name)?
        .iter()
        .map(|v| seg_arg(v, ctx_name))
        .collect()
}

/// `(%json-ref handle segs)` — Rust side of `(json-ref handle seg ...)`.
/// Raises naming the full path on a missing key, an out-of-range index, or
/// indexing into the wrong container kind — see `JsonHandle::resolve`.
pub(crate) fn json_ref(handle: SteelVal, segs: SteelVal) -> SteelResult {
    let handle = handle_arg(handle, "json-ref")?;
    let path = path_arg(segs, "json-ref")?;
    if path.is_empty() {
        steel::stop!(Generic => "json-ref: expected at least one path segment");
    }
    handle.resolve(&path, "json-ref").map_err(generic_err)
}

/// `(%json-contains? handle segs)` — Rust side of `(json-contains? handle
/// seg ...)`. `#t` iff the path resolves; a `null` at the end still counts
/// as present, matching `hash-contains?`'s own rule on a decoded hashmap.
pub(crate) fn json_contains(handle: SteelVal, segs: SteelVal) -> SteelResult {
    let handle = handle_arg(handle, "json-contains?")?;
    let path = path_arg(segs, "json-contains?")?;
    Ok(SteelVal::BoolV(handle.contains(&path)))
}

/// `(json-list j)` — a JSON array handle to a Steel list of its elements
/// (sub-handles for a container element, native values for a scalar one).
/// Raises if `j` isn't an array.
pub(crate) fn json_list(handle: SteelVal) -> SteelResult {
    let handle = handle_arg(handle, "json-list")?;
    let items = handle.list_items("json-list").map_err(generic_err)?;
    Ok(SteelVal::ListV(items.into()))
}

/// `(json-array? v)` — total predicate, `#f` for any non-handle value.
pub(crate) fn is_json_array(val: SteelVal) -> bool {
    downcast_json_handle(&val).is_some_and(|h| h.value().is_array())
}

/// `(json-object? v)` — total predicate, `#f` for any non-handle value.
pub(crate) fn is_json_object(val: SteelVal) -> bool {
    downcast_json_handle(&val).is_some_and(|h| h.value().is_object())
}

#[cfg(test)]
mod tests;
