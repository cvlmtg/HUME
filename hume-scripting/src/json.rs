//! Total, bidirectional conversion between `serde_json::Value` and `SteelVal`.
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
//!                 (Steel code may build hashmaps with symbol keys); anything
//!                 else unrepresentable (closures, custom types, ports, …)
//!                 is a hard error naming the offending value's kind.
//! ```
//!
//! This is deliberately generic JSON — it knows nothing about LSP shapes.

use std::sync::Arc;

use num_traits::ToPrimitive;
use steel::HashMap as SteelHashMap;
use steel::gc::{Gc, ShareableMut as _};
use steel::rvals::{Custom, IntoSteelVal as _, SteelVal};

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
/// values with no JSON representation (functions, ports, custom types, …) —
/// the error names the offending kind rather than silently producing `null`.
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

/// Opaque handle wrapping a JSON value Scheme never reads a field of —
/// `core:lsp`'s own `textDocument/completion` answer, which its own source
/// (`register-completion-source!`'s proc) hands straight to
/// `completion-emit!` without inspecting it. Steel's own decode (via
/// `json_to_steel`/`steel_to_json`) materializes every field into Steel
/// structures — documentation, `data`, per-item edits, all of it — only to
/// have `completion-emit!` immediately convert it right back to JSON, none
/// of it ever read by the source in between. Wrapping the response instead
/// skips both directions of that round trip for data nothing in Scheme
/// touches. `Arc`-backed so cloning the handle (e.g. through the Steel
/// value system's own `Clone` requirements) never re-clones the response
/// itself. No accessor exists — the one reader is `hume-editor`'s own
/// `completion_response_items`, in Rust, once the handle crosses back out
/// through `completion-emit!`.
#[derive(Debug, Clone)]
pub struct JsonHandle(Arc<serde_json::Value>);

impl JsonHandle {
    pub fn new(value: serde_json::Value) -> Self {
        Self(Arc::new(value))
    }

    pub fn value(&self) -> &serde_json::Value {
        &self.0
    }

    /// Convert to a `SteelVal` without returning `Result` — `IntoSteelVal`
    /// for custom types is infallible, matching `SteelBufferId::
    /// into_steel_val`'s own reasoning.
    pub fn into_steel_val(self) -> SteelVal {
        self.into_steelval().expect("JsonHandle into_steelval")
    }
}

impl Custom for JsonHandle {
    fn fmt(&self) -> Option<Result<String, std::fmt::Error>> {
        Some(Ok("#<json>".to_string()))
    }
}

/// `Some` if `val` is a [`JsonHandle`] — the one place `completion-emit!`
/// tells "a whole LSP response" apart from "a list of items".
pub fn downcast_json_handle(val: &SteelVal) -> Option<JsonHandle> {
    if let SteelVal::Custom(v) = val {
        v.read().as_any_ref().downcast_ref::<JsonHandle>().cloned()
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
