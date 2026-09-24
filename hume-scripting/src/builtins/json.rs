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

use crate::json::{JsonHandle, Seg, WireOrigin, downcast_json_handle, to_steel_handle};

use super::SteelResult;
use super::args::string_arg;
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
    Ok(to_steel_handle(Arc::new(value), WireOrigin::Local))
}

// ── JsonHandle accessors ──────────────────────────────────────────────────────
//
// `json-ref`/`json-contains?`/`json-ref-or` take a variadic path (`j seg
// ...`), which `register_fn!`'s typed-arity table (`builtins/mod.rs`) can't
// express — so, like `path-join`, these are registered directly as
// `SteelVal::FuncV(fn(&[SteelVal]) -> SteelResult)` rather than going
// through that table. This also drops the `%json-ref`/`%json-contains?`
// plus `bootstrap.scm` rest-arg-collecting wrapper layer the table would
// otherwise force: the path never round-trips through a Steel list.

fn handle_arg(val: &SteelVal, ctx_name: &str) -> Result<JsonHandle, steel::rerrs::SteelErr> {
    downcast_json_handle(val)
        .ok_or_else(|| generic_err(format!("{ctx_name}: expected a JSON handle, got {val:?}")))
}

/// A path segment: a string (or symbol) object key, or a non-negative
/// integer array index.
fn seg_arg(val: &SteelVal, ctx_name: &str) -> Result<Seg, steel::rerrs::SteelErr> {
    match val {
        SteelVal::StringV(s) => Ok(Seg::Key(s.as_str().into())),
        SteelVal::SymbolV(s) => Ok(Seg::Key(s.as_str().into())),
        SteelVal::IntV(n) if *n >= 0 => Ok(Seg::Index(*n as usize)),
        _ => steel::stop!(TypeMismatch =>
            "{}: path segment must be a string key or a non-negative integer index", ctx_name),
    }
}

fn path_arg(segs: &[SteelVal], ctx_name: &str) -> Result<Vec<Seg>, steel::rerrs::SteelErr> {
    segs.iter().map(|v| seg_arg(v, ctx_name)).collect()
}

/// `(json-ref j seg ...)`. Raises naming the full path on a missing key, an
/// out-of-range index, or indexing into the wrong container kind — see
/// `JsonHandle::resolve`.
pub(crate) fn json_ref(args: &[SteelVal]) -> SteelResult {
    let [handle, segs @ ..] = args else {
        steel::stop!(ArityMismatch => "json-ref expects a handle and at least one path segment, got {}", args.len());
    };
    if segs.is_empty() {
        steel::stop!(ArityMismatch => "json-ref: expected at least one path segment");
    }
    let handle = handle_arg(handle, "json-ref")?;
    let path = path_arg(segs, "json-ref")?;
    handle.resolve(&path, "json-ref").map_err(generic_err)
}

/// `(json-contains? j seg ...)`. `#t` iff the path resolves; a `null` at
/// the end still counts as present, matching `hash-contains?`'s own rule
/// on a decoded hashmap.
pub(crate) fn json_contains(args: &[SteelVal]) -> SteelResult {
    let [handle, segs @ ..] = args else {
        steel::stop!(ArityMismatch => "json-contains? expects a handle and zero or more path segments, got {}", args.len());
    };
    let handle = handle_arg(handle, "json-contains?")?;
    let path = path_arg(segs, "json-contains?")?;
    Ok(SteelVal::BoolV(handle.contains(&path)))
}

/// `(json-ref-or j default seg ...)` — `json-ref`, but `default` (evaluated
/// eagerly, like `hash-ref`'s own optional third argument) in place of
/// raising when the path doesn't resolve. Still raises if `j` isn't a
/// handle — a caller check we don't want silently swallowed by `default`.
/// Collapses the `(if (json-contains? j seg ...) (json-ref j seg ...)
/// default)` idiom several `core:lsp` files repeated, which walked the path
/// twice; this walks it once via `JsonHandle::lookup`.
pub(crate) fn json_ref_or(args: &[SteelVal]) -> SteelResult {
    let [handle, default, segs @ ..] = args else {
        steel::stop!(ArityMismatch =>
            "json-ref-or expects a handle, a default, and at least one path segment, got {}", args.len());
    };
    if segs.is_empty() {
        steel::stop!(ArityMismatch => "json-ref-or: expected at least one path segment");
    }
    let handle = handle_arg(handle, "json-ref-or")?;
    let path = path_arg(segs, "json-ref-or")?;
    Ok(handle.lookup(&path).unwrap_or_else(|| default.clone()))
}

/// `(json-list j)` — a JSON array handle to a Steel list of its elements
/// (sub-handles for a container element, native values for a scalar one).
/// Raises if `j` isn't an array.
pub(crate) fn json_list(handle: SteelVal) -> SteelResult {
    let handle = handle_arg(&handle, "json-list")?;
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
