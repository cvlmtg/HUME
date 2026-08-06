//! One marshalling vocabulary for builtins: plain `SteelVal` decoders,
//! `FromSteelVal` newtypes for buffer-id / position / text-edit params, and
//! the shared list/tuple decoders every multi-field setter builds on.
//!
//! No other module hand-rolls one of these decodes — enforced by
//! `rg 'expect\("len checked"\)' hume-scripting/src` returning empty.

use std::ops::RangeInclusive;
use std::path::PathBuf;

use steel::rerrs::{ErrorKind, SteelErr};
use steel::rvals::{FromSteelVal, SteelVal};

use hume_engine::pipeline::BufferId;

use super::errors::generic_err;

// ── Plain decoders ──────────────────────────────────────────────────────────
//
// Calling convention: `(val, ctx_name: &str)`, one Rust param per Steel arg
// (the convention `register_fn_with_ctx` builtins use — not a `&[SteelVal]`
// slice). `ctx_name` names the argument for the error message.

/// A string. Accepts both strings and symbols, since Scheme callers often
/// pass unquoted symbol literals where a string is semantically expected.
pub(crate) fn string_arg(val: SteelVal, ctx_name: &str) -> Result<String, SteelErr> {
    match val {
        SteelVal::StringV(s) => Ok(s.to_string()),
        SteelVal::SymbolV(s) => Ok(s.to_string()),
        _ => steel::stop!(TypeMismatch => "{}: expected a string", ctx_name),
    }
}

/// A string argument that may be `#f` (absent).
pub(crate) fn optional_string_arg(
    val: SteelVal,
    ctx_name: &str,
) -> Result<Option<String>, SteelErr> {
    match val {
        SteelVal::BoolV(false) => Ok(None),
        other => Ok(Some(string_arg(other, ctx_name)?)),
    }
}

/// A filesystem path, from a Steel string.
pub(crate) fn path_arg(val: SteelVal, ctx_name: &str) -> Result<PathBuf, SteelErr> {
    match val {
        SteelVal::StringV(s) => Ok(PathBuf::from(s.as_str())),
        _ => steel::stop!(TypeMismatch => "{}: expected a string path", ctx_name),
    }
}

/// A path argument that may be `#f` (absent).
pub(crate) fn optional_path_arg(
    val: SteelVal,
    ctx_name: &str,
) -> Result<Option<PathBuf>, SteelErr> {
    match val {
        SteelVal::BoolV(false) => Ok(None),
        SteelVal::StringV(s) => Ok(Some(PathBuf::from(s.as_str()))),
        _ => steel::stop!(TypeMismatch => "{}: expected a string path or #f", ctx_name),
    }
}

/// A non-negative integer, as `usize`. Callers needing `u64` (timer ids,
/// millisecond durations) cast at the call site.
pub(crate) fn usize_arg(val: SteelVal, ctx_name: &str) -> Result<usize, SteelErr> {
    match val {
        SteelVal::IntV(n) if n >= 0 => Ok(n as usize),
        _ => steel::stop!(TypeMismatch => "{}: expected a non-negative integer", ctx_name),
    }
}

/// A non-negative-integer argument that may be `#f` (absent).
pub(crate) fn optional_usize_arg(val: SteelVal, ctx_name: &str) -> Result<Option<usize>, SteelErr> {
    match val {
        SteelVal::BoolV(false) => Ok(None),
        other => Ok(Some(usize_arg(other, ctx_name)?)),
    }
}

/// A signed integer.
pub(crate) fn int_arg(val: SteelVal, ctx_name: &str) -> Result<i64, SteelErr> {
    match val {
        SteelVal::IntV(n) => Ok(n as i64),
        _ => steel::stop!(TypeMismatch => "{}: expected an integer", ctx_name),
    }
}

/// A bool.
pub(crate) fn bool_arg(val: SteelVal, ctx_name: &str) -> Result<bool, SteelErr> {
    match val {
        SteelVal::BoolV(b) => Ok(b),
        _ => steel::stop!(TypeMismatch => "{}: expected a bool", ctx_name),
    }
}

/// A Steel list, unpacked to a `Vec<SteelVal>`.
pub(crate) fn list_items(val: SteelVal, ctx_name: &str) -> Result<Vec<SteelVal>, SteelErr> {
    match val {
        SteelVal::ListV(list) => Ok(list.into_iter().collect()),
        _ => steel::stop!(TypeMismatch => "{}: expected a list", ctx_name),
    }
}

/// A Steel list of strings, unpacked to a `Vec<String>`.
pub(crate) fn list_to_strings(val: SteelVal, ctx_name: &str) -> Result<Vec<String>, SteelErr> {
    list_items(val, ctx_name)?
        .into_iter()
        .map(|v| string_arg(v, ctx_name))
        .collect()
}

/// A Steel list of `("KEY" . "VALUE")` dotted pairs, unpacked to
/// `Vec<(String, String)>` — the wire shape for `register-lsp-server!`'s
/// `#:env`.
pub(crate) fn list_to_env_pairs(
    val: SteelVal,
    ctx_name: &str,
) -> Result<Vec<(String, String)>, SteelErr> {
    list_items(val, ctx_name)?
        .into_iter()
        .map(|entry| {
            let (key, value) = pair_fields(entry, ctx_name, "(\"KEY\" . \"VALUE\")")?;
            Ok((string_arg(key, ctx_name)?, string_arg(value, ctx_name)?))
        })
        .collect()
}

/// A Steel list of single-character strings, unpacked to a `Vec<char>`.
pub(crate) fn chars_arg(val: SteelVal, ctx_name: &str) -> Result<Vec<char>, SteelErr> {
    list_to_strings(val, ctx_name)?
        .into_iter()
        .map(|s| {
            let mut it = s.chars();
            match (it.next(), it.next()) {
                (Some(c), None) => Ok(c),
                _ => steel::stop!(Generic =>
                    "{}: each entry must be exactly one character, got {:?}", ctx_name, s),
            }
        })
        .collect()
}

/// Converts `val` to the wire-shaped JSON a request/notification `params`
/// (or an `#:init-options`/`#:settings` blob) expects — always an object (or
/// array), never a bare scalar. Rejects a bool explicitly: several callers
/// pass through a value that is `#f` when absent, and without this check
/// that would silently reach the wire as `params: false` instead of
/// erroring at the boundary.
pub(crate) fn json_params(val: SteelVal, ctx_name: &str) -> Result<serde_json::Value, SteelErr> {
    if matches!(val, SteelVal::BoolV(_)) {
        steel::stop!(TypeMismatch => "{}: expected a hashmap, got a boolean", ctx_name);
    }
    crate::json::steel_to_json(&val).map_err(|e| generic_err(format!("{ctx_name}: {e}")))
}

/// A JSON-blob argument that may be `#f` (absent).
pub(crate) fn optional_json_arg(
    val: SteelVal,
    ctx_name: &str,
) -> Result<Option<serde_json::Value>, SteelErr> {
    match val {
        SteelVal::BoolV(false) => Ok(None),
        other => Ok(Some(json_params(other, ctx_name)?)),
    }
}

// ── Fixed-arity list decoders ────────────────────────────────────────────────

/// Unpacks `val` as a list and errors unless its length falls in `arity` —
/// the shared shape check every fixed-field entry (a decoration setter's
/// tuple, a position pair, a text edit) opens with.
pub(crate) fn checked_fields(
    val: SteelVal,
    ctx_name: &str,
    arity: RangeInclusive<usize>,
    shape: &str,
) -> Result<Vec<SteelVal>, SteelErr> {
    let fields = list_items(val, ctx_name)?;
    if !arity.contains(&fields.len()) {
        steel::stop!(Generic => "{}: each entry must be {}", ctx_name, shape);
    }
    Ok(fields)
}

/// Decodes a Steel list of fixed-arity tuples into `Vec<T>` — the shared
/// skeleton every tuple-shaped decoration setter (`set-signs!`,
/// `set-extra-highlights!`, `set-eol-text!`, and
/// `set-virtual-lines!`'s inner `'segments` list, …) opens with: unpack the
/// outer list, check each entry's arity against `shape`, then hand the
/// checked, index-safe slice to `row` for field-specific decoding.
/// `set-virtual-lines!`'s own outer `lines` list is hashmap-shaped instead
/// (`virtual_line_specs` in `builtins/decorations.rs`) and doesn't go
/// through this.
pub(crate) fn tuple_list<T>(
    val: SteelVal,
    ctx_name: &str,
    arity: RangeInclusive<usize>,
    shape: &str,
    mut row: impl FnMut(&[SteelVal]) -> Result<T, SteelErr>,
) -> Result<Vec<T>, SteelErr> {
    list_items(val, ctx_name)?
        .into_iter()
        .map(|entry| {
            let fields = checked_fields(entry, ctx_name, arity.clone(), shape)?;
            row(&fields)
        })
        .collect()
}

// ── Dotted-pair decoders/encoders ────────────────────────────────────────────

/// Unpacks `val` as a dotted pair `(car . cdr)` — the shared decode for wire
/// shapes that are semantically a 2-tuple (a position, a range). Rejects a
/// proper 2-element list: the wire format is a pair, not a list.
pub(crate) fn pair_fields(
    val: SteelVal,
    ctx_name: &str,
    shape: &str,
) -> Result<(SteelVal, SteelVal), SteelErr> {
    match val {
        SteelVal::Pair(p) => Ok((p.car(), p.cdr())),
        _ => steel::stop!(Generic => "{}: each entry must be {}", ctx_name, shape),
    }
}

/// Builds a dotted pair `(a . b)` — the shared encode counterpart to
/// `pair_fields`, via steel-core's public `cons` primitive (the only public
/// pair-construction API; the `Pair` type itself is unnameable outside
/// steel-core).
///
/// `b` must not be a list (including `'()`): steel's `cons` returns a proper
/// `ListV` rather than a `Pair` when `b` is itself list-shaped, and
/// `pair_fields` would then reject the round-trip. Every current caller's
/// cdr is a scalar (`IntV`/position), so this holds in practice.
pub(crate) fn cons_pair(mut a: SteelVal, mut b: SteelVal) -> Result<SteelVal, SteelErr> {
    steel::primitives::lists::cons(&mut a, &mut b)
}

// ── FromSteelVal newtypes ────────────────────────────────────────────────────
//
// Used as typed params in builtin signatures — steel-core's
// `register_fn_with_ctx` wrapper prepends the registered builtin name to any
// `from_steelval` failure automatically, so these messages carry no
// `ctx_name` of their own.

/// A decoded `BufferId` argument. Avoids the inline
/// `downcast_buffer_id(...).ok_or_else(...)` pattern every buffer-touching
/// builtin would otherwise repeat.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BidArg(pub(crate) BufferId);

impl FromSteelVal for BidArg {
    fn from_steelval(val: &SteelVal) -> Result<Self, SteelErr> {
        super::ids::downcast_buffer_id(val)
            .map(BidArg)
            .ok_or_else(|| SteelErr::new(ErrorKind::TypeMismatch, "expected buffer-id".to_string()))
    }
}

impl BidArg {
    /// Checks the wrapped id against `ctx.host.buffers().buffer_exists`,
    /// returning it unwrapped on success — the shared "does this bid still
    /// name an open buffer" existence check every mutating buffer builtin
    /// opens with. `BidArg` itself only validates *type* (that the Steel
    /// value was a buffer-id at all); this is the *liveness* half.
    pub(crate) fn require_live(
        self,
        ctx: &mut crate::SteelCtx,
        builtin_name: &str,
    ) -> Result<BufferId, SteelErr> {
        if ctx.host.buffers().buffer_exists(self.0) {
            Ok(self.0)
        } else {
            Err(generic_err(format!(
                "{builtin_name}: invalid buffer id {:?}",
                self.0
            )))
        }
    }
}

/// A buffer-id argument that may be `#f` (absent — caller wants the
/// implicit default, e.g. `get-option`'s focused-buffer fallback).
pub(crate) fn optional_bid_arg(
    val: SteelVal,
    ctx_name: &str,
) -> Result<Option<BufferId>, SteelErr> {
    match val {
        SteelVal::BoolV(false) => Ok(None),
        other => Ok(Some(super::ids::downcast_buffer_id(&other).ok_or_else(
            || {
                SteelErr::new(
                    ErrorKind::TypeMismatch,
                    format!("{ctx_name}: expected buffer-id or #f"),
                )
            },
        )?)),
    }
}

/// A decoded `(line . col)` wire position — a dotted pair.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PosArg {
    pub(crate) line: usize,
    pub(crate) col: usize,
}

impl FromSteelVal for PosArg {
    fn from_steelval(val: &SteelVal) -> Result<Self, SteelErr> {
        let (line, col) = pair_fields(val.clone(), "position", "(line . col)")?;
        Ok(PosArg {
            line: usize_arg(line, "position")?,
            col: usize_arg(col, "position")?,
        })
    }
}

/// A decoded `((start-line . start-col) (end-line . end-col) text)` LSP
/// text edit entry — outer 3-tuple is a list, inner positions are dotted
/// pairs.
#[derive(Debug)]
pub(crate) struct TextEditArg {
    pub(crate) start: PosArg,
    pub(crate) end: PosArg,
    pub(crate) text: String,
}

impl FromSteelVal for TextEditArg {
    fn from_steelval(val: &SteelVal) -> Result<Self, SteelErr> {
        let fields = checked_fields(
            val.clone(),
            "text edit",
            3..=3,
            "((start-line . start-col) (end-line . end-col) text)",
        )?;
        Ok(TextEditArg {
            start: PosArg::from_steelval(&fields[0])?,
            end: PosArg::from_steelval(&fields[1])?,
            text: string_arg(fields[2].clone(), "text edit")?,
        })
    }
}

#[cfg(test)]
mod tests;
