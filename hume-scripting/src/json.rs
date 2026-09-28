//! Total, bidirectional conversion between `serde_json::Value` and `SteelVal`,
//! plus [`JsonHandle`], the opaque handle for external JSON.
//!
//! ```text
//! json -> steel:  null    -> Void        (not #f, so false round-trips distinctly)
//!                 bool    -> BoolV
//!                 number  -> IntV if i64-representable; BigNum if only
//!                             u64-representable (exact); NumV (f64) otherwise
//!                 string  -> StringV
//!                 array   -> ListV
//!                 object  -> HashMapV with STRING keys (JSON keys are data
//!                             like "rust-analyzer.cargo", not identifiers)
//!
//! steel -> json:  inverse of the above; SymbolV also becomes a string; a
//!                 JsonHandle unwraps to its resolved value; anything else
//!                 (closures, ports, ...) is an error naming its kind.
//! ```
//!
//! `json_to_steel`/`steel_to_json` serve HUME-authored JSON that Scheme edits
//! directly (the `lsp-*-params` builders, error maps) and know nothing about
//! LSP. [`to_steel_handle`] serves external JSON that Scheme only reads.

use std::sync::Arc;

use num_traits::ToPrimitive;
use steel::HashMap as SteelHashMap;
use steel::gc::{Gc, ShareableMut as _};
use steel::rvals::{Custom, IntoSteelVal as _, SteelVal, as_underlying_type};

use hume_rope::position_encoding::PositionEncoding;

/// Converts a `serde_json::Value` into the equivalent `SteelVal`. Total:
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
        // In (i64::MAX, u64::MAX]: not i64-representable, but still an
        // exact integer (e.g. a large id/hash field). Falling back to f64
        // here would silently lose precision; BigNum represents it exactly
        // instead, so a value echoed back through steel_to_json still
        // matches what the server sent.
        SteelVal::BigNum(Gc::new(u.into()))
    } else {
        // Not representable as an integer at all: a genuine float.
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
/// than [`JsonHandle`], …). The error names the offending kind rather than
/// silently producing `null`.
pub(crate) fn steel_to_json(v: &SteelVal) -> Result<serde_json::Value, String> {
    match v {
        SteelVal::Void => Ok(serde_json::Value::Null),
        SteelVal::BoolV(b) => Ok(serde_json::Value::Bool(*b)),
        SteelVal::IntV(i) => Ok(serde_json::Value::Number((*i as i64).into())),
        // Only ever produced (by json_to_steel) for a u64-range JSON integer,
        // so it always fits back into u64 exactly, but Steel code could in
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
        // reconversion cost: no walk of the handle's contents, just a clone
        // of the `Value` it already points at.
        SteelVal::Custom(_) => downcast_json_handle(v)
            .map(|h| h.value().clone())
            .ok_or_else(|| format!("cannot convert {} to JSON", type_name(v))),
        other => Err(format!("cannot convert {} to JSON", type_name(other))),
    }
}

/// A short, readable label for error messages. Not exhaustive over every
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
/// step has already proven it resolves (see [`JsonHandle::resolve`]), so a
/// handle's path is always valid against its own root by construction.
/// `Arc<str>` (not `Box<str>`) so extending a path by one `Seg` is a
/// refcount bump per existing segment, not a string copy.
#[derive(Debug, Clone)]
pub(crate) enum Seg {
    Key(Arc<str>),
    Index(usize),
}

/// Opaque handle onto a JSON value shared from a common root: what every
/// external JSON crossing (LSP responses, `lsp-capabilities`, `json-parse`,
/// ...) hands Scheme instead of a deep decode. `json-ref`/`json-list` navigate
/// it by extending `path`, sharing the `Arc` root, so drilling into one field
/// of a large capabilities blob copies nothing.
///
/// `origin` records where the root came from, and every child handle inherits
/// it, so a position found several `json-ref`s deep still knows which
/// server's encoding it uses. Equality and hashing ignore `origin` and
/// compare [`JsonHandle::value`] only.
#[derive(Debug, Clone)]
pub struct JsonHandle {
    root: Arc<serde_json::Value>,
    path: Arc<[Seg]>,
    origin: WireOrigin,
}

/// Where a [`JsonHandle`]'s root value came from (see the field's own doc).
#[derive(Debug, Clone, Copy)]
pub enum WireOrigin {
    /// A response from a server negotiated at this encoding: every wire
    /// position anywhere in the tree is counted in it.
    Server(PositionEncoding),
    /// Not a server response: `json-parse`, `lsp-capabilities`, or a
    /// hashmap/handle a plugin built by hand. No encoding to decode a wire
    /// position with (see [`JsonHandle::position_encoding`]).
    Local,
}

/// Resolves one path step against `v`, or `None` if `v` isn't the matching
/// container kind or the key/index doesn't exist.
fn step<'v>(v: &'v serde_json::Value, seg: &Seg) -> Option<&'v serde_json::Value> {
    match (v, seg) {
        (serde_json::Value::Object(map), Seg::Key(k)) => map.get(k.as_ref()),
        (serde_json::Value::Array(arr), Seg::Index(i)) => arr.get(*i),
        _ => None,
    }
}

/// Walks `path` from `start` one [`step`] at a time: the one navigation
/// loop [`JsonHandle::value`], [`JsonHandle::lookup`], [`JsonHandle::contains`],
/// and [`JsonHandle::resolve`] all go through, so a path is only ever walked
/// once per call. `Ok` is the resolved value; `Err` is the value the failing
/// step started from, the segment that failed, and the segments consumed so
/// far (relative to `start`): everything [`miss_reason`] needs to classify
/// *why*, without a second walk to find the failure point again.
fn walk<'v, 's>(
    start: &'v serde_json::Value,
    path: &'s [Seg],
) -> Result<&'v serde_json::Value, (&'v serde_json::Value, &'s Seg, &'s [Seg])> {
    let mut current = start;
    for (i, seg) in path.iter().enumerate() {
        match step(current, seg) {
            Some(next) => current = next,
            None => return Err((current, seg, &path[..i])),
        }
    }
    Ok(current)
}

/// The wrong-kind/missing-key/out-of-range text `resolve` raises, given the
/// value a failed step started from, the segment that failed, and the path
/// so far. Re-matches `(current, seg)` to classify *why* [`step`] returned
/// `None`, using the same four shapes `step` itself distinguishes.
fn miss_reason(current: &serde_json::Value, seg: &Seg, so_far: &[Seg], ctx_name: &str) -> String {
    match (current, seg) {
        (serde_json::Value::Object(_), Seg::Key(k)) => {
            format!("{ctx_name}: no key {k:?} at {}", path_repr(so_far))
        }
        (serde_json::Value::Array(arr), Seg::Index(i)) => format!(
            "{ctx_name}: index {i} out of range (length {}) at {}",
            arr.len(),
            path_repr(so_far)
        ),
        (serde_json::Value::Object(_), Seg::Index(i)) => format!(
            "{ctx_name}: expected an array to index {i} at {}, found an object",
            path_repr(so_far)
        ),
        (serde_json::Value::Array(_), Seg::Key(k)) => format!(
            "{ctx_name}: expected an object to look up {k:?} at {}, found an array",
            path_repr(so_far)
        ),
        (_, seg) => format!(
            "{ctx_name}: cannot look up {} at {}: not an object or array",
            seg_repr(seg),
            path_repr(so_far)
        ),
    }
}

/// Renders a path the way `json-ref`'s error text names it: `$` for the
/// root, then `.key` / `[index]` per step, e.g. `$.items[2].label`.
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
/// container (object/array) becomes a [`JsonHandle`] at `path()`, tagged with
/// `origin` (inherited from the parent handle a navigation method calls this
/// from), a scalar crosses natively via [`json_to_steel`] (so `(string=? s
/// "x")`/`(= n 5)` just work with no accessor), the same three-way split
/// `json_to_steel` uses for a scalar, but a container never gets walked into
/// Steel structures here. `path` is a closure rather than an already-built
/// `Arc<[Seg]>` so a scalar result (most `json-list` elements, most object
/// fields) costs no path allocation at all. Only a container result ever
/// calls it.
fn scalar_or_child(
    v: &serde_json::Value,
    root: &Arc<serde_json::Value>,
    origin: WireOrigin,
    path: impl FnOnce() -> Arc<[Seg]>,
) -> SteelVal {
    match v {
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => JsonHandle {
            root: Arc::clone(root),
            path: path(),
            origin,
        }
        .into_steel_val(),
        scalar => json_to_steel(scalar),
    }
}

/// Converts a freshly-received external JSON value to its Steel
/// representation via `scalar_or_child`, rooted at `value` itself. The one
/// call every external crossing makes instead of `json_to_steel`. `origin`
/// tags the root (see [`JsonHandle`]'s own doc).
pub fn to_steel_handle(value: Arc<serde_json::Value>, origin: WireOrigin) -> SteelVal {
    scalar_or_child(value.as_ref(), &value, origin, || Arc::from(Vec::new()))
}

impl JsonHandle {
    /// Wraps a value as a handle unconditionally, even when it happens to be
    /// a scalar, for a caller that already knows it wants a handle
    /// regardless (`json_arg`'s fallback path, `builtins/args.rs`). Always
    /// [`WireOrigin::Local`]: every caller builds `value` itself (a plugin's
    /// own hashmap, `json-parse`'s decode) rather than receiving it from a
    /// server. A real server response only ever reaches Steel through
    /// [`to_steel_handle`], which takes the origin explicitly.
    pub fn new(value: serde_json::Value) -> Self {
        Self {
            root: Arc::new(value),
            path: Arc::from(Vec::new()),
            origin: WireOrigin::Local,
        }
    }

    /// Test-only: builds a handle already tagged as a server response, for a
    /// unit test that decodes wire positions directly against a hand-built
    /// JSON tree rather than going through a real request/response round
    /// trip (`bridge.rs`'s `outcome_to_steel` is the production tagging
    /// site).
    #[cfg(test)]
    pub(crate) fn server_for_test(value: serde_json::Value, encoding: PositionEncoding) -> Self {
        Self {
            root: Arc::new(value),
            path: Arc::from(Vec::new()),
            origin: WireOrigin::Server(encoding),
        }
    }

    /// This handle's tree's negotiated encoding, or `Err` if it didn't come
    /// from a server ([`WireOrigin::Local`]), naming `ctx_name` (the
    /// builtin asking) so the error identifies which call needs a real
    /// response. The funnel every wire-position decode reads its encoding
    /// through. The encoding travels with the response because a `bid`'s
    /// *currently* attached server may have restarted or detached since.
    pub fn position_encoding(&self, ctx_name: &str) -> Result<PositionEncoding, String> {
        match self.origin {
            WireOrigin::Server(encoding) => Ok(encoding),
            WireOrigin::Local => Err(format!(
                "{ctx_name}: not a value from an LSP server response: no encoding to decode a \
                 wire position with"
            )),
        }
    }

    /// The value this handle points at, resolved by walking its `path` from
    /// `root`. `expect`s the walk succeeds, which is sound because a handle's path
    /// is only ever extended by `JsonHandle::resolve` after that exact
    /// step already proved it resolves.
    pub fn value(&self) -> &serde_json::Value {
        walk(&self.root, &self.path)
            .expect("JsonHandle path only grows through a successful navigation step")
    }

    /// Walks `path` from this handle's own position, `None` if any step
    /// doesn't resolve. Wraps a container result in a handle (unlike
    /// [`JsonHandle::contains`], which only needs a yes/no and so calls
    /// [`walk`] directly rather than paying for a child handle nobody wants).
    pub(crate) fn lookup(&self, path: &[Seg]) -> Option<SteelVal> {
        let current = walk(self.value(), path).ok()?;
        Some(scalar_or_child(current, &self.root, self.origin, || {
            self.path.iter().chain(path).cloned().collect()
        }))
    }

    /// `(json-ref j seg ...)`'s implementation. `Err` names the full path
    /// and the reason: a missing key, an out-of-range index, or indexing
    /// into the wrong container kind (or a scalar). [`walk`]'s `Err` arm
    /// already carries everything [`miss_reason`] needs, no second walk.
    pub(crate) fn resolve(&self, path: &[Seg], ctx_name: &str) -> Result<SteelVal, String> {
        match walk(self.value(), path) {
            Ok(current) => Ok(scalar_or_child(current, &self.root, self.origin, || {
                self.path.iter().chain(path).cloned().collect()
            })),
            Err((current, seg, so_far)) => {
                let so_far: Vec<Seg> = self.path.iter().chain(so_far).cloned().collect();
                Err(miss_reason(current, seg, &so_far, ctx_name))
            }
        }
    }

    /// `(json-contains? j seg ...)`'s implementation. `#t` iff `path`
    /// resolves. A `null` value at the end still counts as present, same
    /// as `hash-contains?` on a decoded hashmap. Doesn't route through
    /// [`JsonHandle::lookup`]: a container match there still pays for a
    /// child handle nobody wants, where this only needs a yes/no.
    pub(crate) fn contains(&self, path: &[Seg]) -> bool {
        walk(self.value(), path).is_ok()
    }

    /// `(json-list j)`'s implementation. `Err` if this handle isn't an
    /// array.
    pub(crate) fn list_items(&self, ctx_name: &str) -> Result<Vec<SteelVal>, String> {
        match self.value() {
            serde_json::Value::Array(arr) => Ok(arr
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    scalar_or_child(item, &self.root, self.origin, || {
                        self.path
                            .iter()
                            .cloned()
                            .chain(std::iter::once(Seg::Index(i)))
                            .collect()
                    })
                })
                .collect()),
            _ => Err(format!("{ctx_name}: expected a JSON array")),
        }
    }

    /// A child handle onto `key`'s array (or this handle's own value, if
    /// `key` is `None`) at `index`: the one shape a `textDocument/completion`
    /// response's per-item slice needs (a bare `CompletionItem[]` array, or
    /// a `CompletionList`'s `"items"` array), so a Rust caller that already
    /// holds a slice reference into `self.value()` (e.g.
    /// `hume_lsp::completion_item::completion_response_items`'s result) can
    /// hand Steel a live pointer into the original response (sharing this
    /// handle's root `Arc`) instead of cloning the item out. `None` if
    /// either step doesn't resolve, same contract as `JsonHandle::lookup`,
    /// just returning the handle unconditionally rather than routing a
    /// container result through `scalar_or_child`, since the caller already
    /// knows it wants a handle regardless of whether the target is a
    /// container or a scalar (see [`JsonHandle::new`]'s own reasoning).
    pub fn indexed_child(&self, key: Option<&str>, index: usize) -> Option<JsonHandle> {
        let path: Vec<Seg> = key
            .map(|k| Seg::Key(Arc::from(k)))
            .into_iter()
            .chain(std::iter::once(Seg::Index(index)))
            .collect();
        walk(self.value(), &path).ok()?;
        Some(JsonHandle {
            root: Arc::clone(&self.root),
            path: self.path.iter().chain(path.iter()).cloned().collect(),
            origin: self.origin,
        })
    }

    /// Convert to a `SteelVal` without returning `Result`: `IntoSteelVal`
    /// for custom types is infallible, matching `SteelPane::
    /// into_steel_val`'s own reasoning.
    pub fn into_steel_val(self) -> SteelVal {
        self.into_steelval().expect("JsonHandle into_steelval")
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

/// Hashes the resolved JSON value, not the handle's identity, so
/// `try_as_dyn_hash` agrees with `equality_hint`'s value comparison (two
/// handles onto equal JSON must hash the same to be usable as Steel hash
/// keys). Object hashing is order-insensitive (XOR-combine each entry's own
/// hash) to agree with `serde_json::Value`'s own `PartialEq` on `Map`, which
/// (backed by `indexmap` under this workspace's `preserve_order` feature)
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
            // Number derives Hash directly (this workspace doesn't enable
            // its arbitrary_precision feature), so this stays consistent
            // with equality by construction, including its float arm,
            // which hashes +0.0 and -0.0 alike to agree with their PartialEq.
            n.hash(state);
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
