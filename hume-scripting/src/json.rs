//! Total, bidirectional conversion between `serde_json::Value` and `SteelVal`,
//! plus [`JsonHandle`] — the opaque, rooted handle every *external* JSON
//! crossing (an LSP response, capabilities, a notification's params, a
//! completion item, a `json-parse` result) uses instead of a deep decode.
//!
//! Mapping table:
//!
//! ```text
//! json -> steel:  null    -> Void        (NOT #f — false must round-trip distinctly)
//!                 bool    -> BoolV
//!                 number  -> IntV when i64-representable; BigNum when
//!                             u64-representable but not i64 (exact, no
//!                             precision loss); NumV (f64) otherwise
//!                 string  -> StringV
//!                 array   -> ListV
//!                 object  -> HashMapV with STRING keys (not symbols — JSON keys are
//!                             arbitrary data like "rust-analyzer.cargo", not identifiers)
//!
//! steel -> json:  inverse of the above; SymbolV is also accepted as a string
//!                 (Steel code may build hashmaps with symbol keys); a
//!                 JsonHandle unwraps to its own resolved value; anything
//!                 else unrepresentable (closures, ports, …) is a hard error
//!                 naming the offending value's kind.
//! ```
//!
//! `json_to_steel`/`steel_to_json` are for HUME-authored JSON that Scheme is
//! meant to edit directly (the `lsp-*-params` builders, `completion-top`,
//! the native `{"code" "message"}` error map) — this is deliberately generic
//! JSON, knowing nothing about LSP shapes. [`to_steel_handle`] is the
//! opposite direction's funnel: external JSON nobody in Scheme is meant to
//! rebuild field-by-field.

use std::sync::Arc;

use num_traits::ToPrimitive;
use steel::HashMap as SteelHashMap;
use steel::gc::{Gc, ShareableMut as _};
use steel::rvals::{Custom, IntoSteelVal as _, SteelVal, as_underlying_type};

/// Converts a `serde_json::Value` into the equivalent `SteelVal`. Total —
/// every JSON value has a representation, so this never fails.
pub fn json_to_steel(v: &serde_json::Value) -> SteelVal {
    match v {
        serde_json::Value::Null => SteelVal::Void,
        serde_json::Value::Bool(b) => SteelVal::BoolV(*b),
        serde_json::Value::Number(n) => number_to_steel(n),
        serde_json::Value::String(s) => SteelVal::StringV(s.as_str().into()),
        serde_json::Value::Array(items) => {
            let items: Vec<SteelVal> = items.iter().map(json_to_steel).collect();
            SteelVal::ListV(items.into())
        }
        serde_json::Value::Object(map) => {
            let mut hm = SteelHashMap::new();
            for (key, value) in map {
                hm.insert(SteelVal::StringV(key.as_str().into()), json_to_steel(value));
            }
            SteelVal::HashMapV(Gc::new(hm).into())
        }
    }
}

fn number_to_steel(n: &serde_json::Number) -> SteelVal {
    if let Some(i) = n.as_i64() {
        SteelVal::IntV(i as isize)
    } else if let Some(u) = n.as_u64() {
        // In (i64::MAX, u64::MAX] — not i64-representable, but still an
        // exact integer (e.g. a large id/hash field). Falling back to f64
        // here would silently lose precision; BigNum represents it exactly
        // instead, so a value echoed back through steel_to_json still
        // matches what the server sent.
        SteelVal::BigNum(Gc::new(u.into()))
    } else {
        // Not representable as an integer at all — a genuine float.
        // serde_json::Number is always finite and, without the
        // `arbitrary_precision` feature (which this workspace does not
        // enable), always convertible to f64.
        SteelVal::NumV(
            n.as_f64()
                .expect("serde_json::Number is f64-representable without arbitrary_precision"),
        )
    }
}

/// Converts a `SteelVal` into the equivalent `serde_json::Value`. Fails on
/// values with no JSON representation (functions, ports, custom types other
/// than [`JsonHandle`], …) — the error names the offending kind rather than
/// silently producing `null`.
pub(crate) fn steel_to_json(v: &SteelVal) -> Result<serde_json::Value, String> {
    match v {
        SteelVal::Void => Ok(serde_json::Value::Null),
        SteelVal::BoolV(b) => Ok(serde_json::Value::Bool(*b)),
        SteelVal::IntV(i) => Ok(serde_json::Value::Number((*i as i64).into())),
        // Only ever produced (by json_to_steel) for a u64-range JSON integer,
        // so it always fits back into u64 exactly — but Steel code could in
        // principle construct a bigger one directly, hence the checked
        // conversion rather than an infallible one.
        SteelVal::BigNum(b) => b
            .to_u64()
            .map(|u| serde_json::Value::Number(u.into()))
            .ok_or_else(|| "integer too large to represent in JSON".to_string()),
        SteelVal::NumV(n) => serde_json::Number::from_f64(*n)
            .map(serde_json::Value::Number)
            .ok_or_else(|| format!("number is not finite: {n}")),
        SteelVal::StringV(s) => Ok(serde_json::Value::String(s.to_string())),
        SteelVal::SymbolV(s) => Ok(serde_json::Value::String(s.to_string())),
        SteelVal::ListV(items) => {
            let items = items
                .iter()
                .map(steel_to_json)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(serde_json::Value::Array(items))
        }
        SteelVal::HashMapV(hm) => {
            let mut map = serde_json::Map::new();
            for (key, value) in hm.iter() {
                let key = match key {
                    SteelVal::StringV(s) => s.to_string(),
                    SteelVal::SymbolV(s) => s.to_string(),
                    other => {
                        return Err(format!("hashmap key is not a string: {}", type_name(other)));
                    }
                };
                map.insert(key, steel_to_json(value)?);
            }
            Ok(serde_json::Value::Object(map))
        }
        // A handle re-crossing into Rust (e.g. Scheme nests a completion
        // item's `"arguments"` sub-object inside a hash it builds itself for
        // `workspace/executeCommand`) resolves to its own value at zero
        // reconversion cost — no walk of the handle's contents, just a clone
        // of the `Value` it already points at.
        SteelVal::Custom(_) => downcast_json_handle(v)
            .map(|h| h.value().clone())
            .ok_or_else(|| format!("cannot convert {} to JSON", type_name(v))),
        other => Err(format!("cannot convert {} to JSON", type_name(other))),
    }
}

/// A short, readable label for error messages — not exhaustive over every
/// `SteelVal` variant, just the ones plausible enough to show up in a hashmap
/// or list a plugin author built by hand.
fn type_name(v: &SteelVal) -> &'static str {
    match v {
        SteelVal::Closure(_)
        | SteelVal::FuncV(_)
        | SteelVal::BoxedFunction(_)
        | SteelVal::MutFunc(_)
        | SteelVal::BuiltIn(_)
        | SteelVal::ContinuationFunction(_)
        | SteelVal::FutureFunc(_) => "function",
        SteelVal::Custom(_) | SteelVal::CustomStruct(_) => "custom type",
        SteelVal::PortV(_) => "port",
        SteelVal::HashSetV(_) => "hashset",
        SteelVal::VectorV(_) | SteelVal::MutableVector(_) => "vector",
        SteelVal::CharV(_) => "char",
        _ => "unsupported value",
    }
}

// ── JsonHandle ──────────────────────────────────────────────────────────────

/// One step of a [`JsonHandle`]'s path from its root: an object key or an
/// array index. Only ever appended to a handle's path after a navigation
/// step has already proven it resolves — see [`JsonHandle::resolve`] — so a
/// handle's path is always valid against its own root by construction.
#[derive(Debug, Clone)]
pub(crate) enum Seg {
    Key(Box<str>),
    Index(usize),
}

/// Opaque handle onto a JSON value shared from a common root — the funnel
/// every external JSON crossing (an LSP response, `lsp-capabilities`, a
/// notification's params, `on-completion-accept`'s item, `json-parse`)
/// hands Scheme instead of `json_to_steel`'s deep decode into hashmaps and
/// lists nobody reads most of. `json-ref`/`json-list` (`builtins/json.rs`)
/// navigate a handle without ever materializing the subtree they don't
/// touch; a container result from either shares the same root `Arc` rather
/// than cloning the subtree, so drilling into one field of a 5,000-line
/// capabilities blob costs one more `Seg`, not a second deep copy.
///
/// `root` is `Arc`-backed so cloning a handle (including through the Steel
/// value system's own `Clone` requirements) never re-clones the response
/// itself; `path` is `Arc`-backed for the same reason a child handle can
/// share it structurally with a sibling built from the same parent.
#[derive(Debug, Clone)]
pub struct JsonHandle {
    root: Arc<serde_json::Value>,
    path: Arc<[Seg]>,
}

/// Resolves one path step against `v`, or `None` if `v` isn't the matching
/// container kind or the key/index doesn't exist. Shared by
/// [`JsonHandle::value`] (which trusts its own path and `expect`s) and
/// [`JsonHandle::contains`] (which only wants a yes/no).
fn step<'v>(v: &'v serde_json::Value, seg: &Seg) -> Option<&'v serde_json::Value> {
    match (v, seg) {
        (serde_json::Value::Object(map), Seg::Key(k)) => map.get(k.as_ref()),
        (serde_json::Value::Array(arr), Seg::Index(i)) => arr.get(*i),
        _ => None,
    }
}

/// Renders a path the way `json-ref`'s error text names it: `$` for the
/// root, then `.key` / `[index]` per step — e.g. `$.items[2].label`.
fn path_repr(segs: &[Seg]) -> String {
    let mut s = String::from("$");
    for seg in segs {
        match seg {
            Seg::Key(k) => {
                s.push('.');
                s.push_str(k);
            }
            Seg::Index(i) => {
                s.push('[');
                s.push_str(&i.to_string());
                s.push(']');
            }
        }
    }
    s
}

fn seg_repr(seg: &Seg) -> String {
    match seg {
        Seg::Key(k) => format!("key {k:?}"),
        Seg::Index(i) => format!("index {i}"),
    }
}

/// The Rust→Steel funnel every external JSON value crosses through: a
/// container (object/array) becomes a [`JsonHandle`], a scalar crosses
/// natively (so `(string=? s "x")`/`(= n 5)` just work with no accessor),
/// and `null` is `Void` — same three-way split `json_to_steel` uses for a
/// scalar, but a container never gets walked into Steel structures here.
fn scalar_or_child(
    v: &serde_json::Value,
    root: &Arc<serde_json::Value>,
    path: Vec<Seg>,
) -> SteelVal {
    match v {
        serde_json::Value::Null => SteelVal::Void,
        serde_json::Value::Bool(b) => SteelVal::BoolV(*b),
        serde_json::Value::Number(n) => number_to_steel(n),
        serde_json::Value::String(s) => SteelVal::StringV(s.as_str().into()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => JsonHandle {
            root: Arc::clone(root),
            path: Arc::from(path),
        }
        .into_steel_val(),
    }
}

/// Converts a freshly-received external JSON value to its Steel
/// representation via [`scalar_or_child`], rooted at `value` itself. The one
/// call every external crossing makes instead of `json_to_steel`.
pub fn to_steel_handle(value: Arc<serde_json::Value>) -> SteelVal {
    scalar_or_child(value.as_ref(), &value, Vec::new())
}

impl JsonHandle {
    /// Wraps a value as a handle unconditionally, even when it happens to be
    /// a scalar — for a caller that already knows it wants a handle
    /// regardless (`hume-editor`'s `lsp-request #:raw #t` path treats the
    /// whole non-null response as one handle, by design: an LSP response is
    /// always an object or array by protocol, never a bare scalar).
    /// [`to_steel_handle`] is the general funnel for a value whose shape
    /// isn't known ahead of time; this is for a caller that already knows.
    pub fn new(value: serde_json::Value) -> Self {
        Self::from(Arc::new(value))
    }

    /// The value this handle points at, resolved by walking its `path` from
    /// `root`. `expect`s the walk succeeds — sound because a handle's path
    /// is only ever extended by [`JsonHandle::resolve`] after that exact
    /// step already proved it resolves.
    pub fn value(&self) -> &serde_json::Value {
        let mut v: &serde_json::Value = &self.root;
        for seg in self.path.iter() {
            v = step(v, seg)
                .expect("JsonHandle path only grows through a successful navigation step");
        }
        v
    }

    /// `(json-ref j seg ...)`'s implementation. Walks `path` from this
    /// handle's own position and funnels the result through
    /// [`scalar_or_child`]. `Err` names the full path and the reason: a
    /// missing key, an out-of-range index, or indexing into the wrong
    /// container kind (or a scalar).
    pub(crate) fn resolve(&self, path: &[Seg], ctx_name: &str) -> Result<SteelVal, String> {
        let mut current = self.value();
        let mut so_far: Vec<Seg> = self.path.to_vec();
        for seg in path {
            current = match (current, seg) {
                (serde_json::Value::Object(map), Seg::Key(k)) => map
                    .get(k.as_ref())
                    .ok_or_else(|| format!("{ctx_name}: no key {k:?} at {}", path_repr(&so_far)))?,
                (serde_json::Value::Array(arr), Seg::Index(i)) => arr.get(*i).ok_or_else(|| {
                    format!(
                        "{ctx_name}: index {i} out of range (length {}) at {}",
                        arr.len(),
                        path_repr(&so_far)
                    )
                })?,
                (serde_json::Value::Object(_), Seg::Index(i)) => {
                    return Err(format!(
                        "{ctx_name}: expected an array to index {i} at {}, found an object",
                        path_repr(&so_far)
                    ));
                }
                (serde_json::Value::Array(_), Seg::Key(k)) => {
                    return Err(format!(
                        "{ctx_name}: expected an object to look up {k:?} at {}, found an array",
                        path_repr(&so_far)
                    ));
                }
                (_, seg) => {
                    return Err(format!(
                        "{ctx_name}: cannot look up {} at {} — not an object or array",
                        seg_repr(seg),
                        path_repr(&so_far)
                    ));
                }
            };
            so_far.push(seg.clone());
        }
        Ok(scalar_or_child(current, &self.root, so_far))
    }

    /// `(json-contains? j seg ...)`'s implementation. `#t` iff `path`
    /// resolves — a `null` value at the end still counts as present, same
    /// as `hash-contains?` on a decoded hashmap.
    pub(crate) fn contains(&self, path: &[Seg]) -> bool {
        let mut current = self.value();
        for seg in path {
            match step(current, seg) {
                Some(v) => current = v,
                None => return false,
            }
        }
        true
    }

    /// `(json-list j)`'s implementation. `Err` if this handle isn't an
    /// array.
    pub(crate) fn list_items(&self, ctx_name: &str) -> Result<Vec<SteelVal>, String> {
        match self.value() {
            serde_json::Value::Array(arr) => {
                let mut path = self.path.to_vec();
                let mut out = Vec::with_capacity(arr.len());
                for (i, item) in arr.iter().enumerate() {
                    path.push(Seg::Index(i));
                    out.push(scalar_or_child(item, &self.root, path.clone()));
                    path.pop();
                }
                Ok(out)
            }
            _ => Err(format!("{ctx_name}: expected a JSON array")),
        }
    }

    /// Convert to a `SteelVal` without returning `Result` — `IntoSteelVal`
    /// for custom types is infallible, matching `SteelBufferId::
    /// into_steel_val`'s own reasoning.
    pub fn into_steel_val(self) -> SteelVal {
        self.into_steelval().expect("JsonHandle into_steelval")
    }
}

impl From<Arc<serde_json::Value>> for JsonHandle {
    /// Zero-copy wrap of a value a caller already holds behind an `Arc` —
    /// `lsp-capabilities` (backed by `LspClient::caps_json`) and a
    /// diagnostic's wire `"raw"` field both share their store's own `Arc`
    /// this way instead of cloning the value to build a handle.
    fn from(root: Arc<serde_json::Value>) -> Self {
        Self {
            root,
            path: Arc::from(Vec::new()),
        }
    }
}

impl Custom for JsonHandle {
    fn fmt(&self) -> Option<Result<String, std::fmt::Error>> {
        Some(Ok(format!("#<json {}>", self.value())))
    }

    fn equality_hint(&self, other: &dyn steel::rvals::CustomType) -> bool {
        as_underlying_type::<Self>(other).is_some_and(|o| self.value() == o.value())
    }

    fn try_as_dyn_hash(&self) -> Option<&dyn steel::rvals::DynHash> {
        Some(self)
    }
}

/// Hashes the resolved JSON value, not the handle's identity — needed so
/// `try_as_dyn_hash` agrees with `equality_hint`'s value comparison (two
/// handles onto equal JSON must hash the same to be usable as Steel hash
/// keys). Object hashing is order-insensitive (XOR-combine each entry's own
/// hash) to agree with `serde_json::Value`'s own `PartialEq` on `Map`, which
/// — backed by `indexmap` under this workspace's `preserve_order` feature —
/// already compares as a set of pairs, not a sequence. Array hashing stays
/// order-sensitive, matching array equality.
impl std::hash::Hash for JsonHandle {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        hash_json_value(self.value(), state);
    }
}

fn hash_json_value<H: std::hash::Hasher>(v: &serde_json::Value, state: &mut H) {
    use std::hash::Hash as _;
    use std::hash::Hasher as _;
    match v {
        serde_json::Value::Null => 0u8.hash(state),
        serde_json::Value::Bool(b) => {
            1u8.hash(state);
            b.hash(state);
        }
        serde_json::Value::Number(n) => {
            2u8.hash(state);
            // Number's Display is exact for its own internal representation
            // (integer prints without a decimal point, float with one), so
            // two Numbers equal under PartialEq always print identically —
            // this stays consistent with equality without needing a
            // separate numeric-canonicalization step.
            n.to_string().hash(state);
        }
        serde_json::Value::String(s) => {
            3u8.hash(state);
            s.hash(state);
        }
        serde_json::Value::Array(items) => {
            4u8.hash(state);
            items.len().hash(state);
            for item in items {
                hash_json_value(item, state);
            }
        }
        serde_json::Value::Object(map) => {
            5u8.hash(state);
            let mut acc: u64 = 0;
            for (k, v) in map {
                let mut entry_hasher = std::collections::hash_map::DefaultHasher::new();
                k.hash(&mut entry_hasher);
                hash_json_value(v, &mut entry_hasher);
                acc ^= entry_hasher.finish();
            }
            acc.hash(state);
        }
    }
}

/// `Some` if `val` is a [`JsonHandle`].
pub fn downcast_json_handle(val: &SteelVal) -> Option<JsonHandle> {
    if let SteelVal::Custom(v) = val {
        v.read().as_any_ref().downcast_ref::<JsonHandle>().cloned()
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
