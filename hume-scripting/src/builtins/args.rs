//! One marshalling vocabulary for builtins: plain `SteelVal` decoders, a
//! `FromSteelVal` newtype for buffer-id params, free-fn decoders for wire
//! position / text-edit params (`WirePos` is a `hume-rope` type, so the
//! orphan rule rules out `FromSteelVal for WirePos` here — a plain function
//! is used for its sibling `WireTextEdit` decoder too, for one calling
//! convention across both), and the shared list/tuple decoders every
//! multi-field setter builds on.
//!
//! `#f`-means-absent is decoded only by this module's `optional_*` family —
//! enforced by `cargo test absent_marker_is_decoded_only_in_args_rs`
//! (`arch-lints/tests/absent_decode.rs`).

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

/// A symbol argument that may be `#f` (absent) — unlike `optional_string_arg`,
/// rejects a plain string: a caller wanting this shape is decoding a Scheme
/// keyword like `'error`, not free text, and `string_arg`'s string-or-symbol
/// leniency would silently widen what it accepts.
pub(crate) fn optional_symbol_arg(
    val: SteelVal,
    ctx_name: &str,
) -> Result<Option<String>, SteelErr> {
    match val {
        SteelVal::BoolV(false) => Ok(None),
        SteelVal::SymbolV(s) => Ok(Some(s.to_string())),
        _ => steel::stop!(TypeMismatch => "{}: expected a symbol or #f", ctx_name),
    }
}

/// Maps an already-decoded keyword string to one of a fixed set of enum
/// values via a `(spelling, value)` table — the "must be 'a, 'b, or 'c, got
/// 'x" shape shared by every Steel enum-keyword decoder (`#:match`,
/// `#:truncate`, `#:anchor`, `#:kind`, bind-mode). Callers
/// extract the string themselves first (`string_arg` for a string-or-symbol
/// argument, a stricter symbol-only check where the wire contract insists on
/// a bare symbol) — this helper is only about the mapping and the error
/// message, so those extraction strictness differences aren't flattened.
pub(crate) fn symbol_enum_arg<T: Copy>(
    s: &str,
    ctx_name: &str,
    variants: &[(&str, T)],
) -> Result<T, SteelErr> {
    for (name, value) in variants {
        if *name == s {
            return Ok(*value);
        }
    }
    let names: Vec<&str> = variants.iter().map(|(name, _)| *name).collect();
    steel::stop!(Generic => "{}: must be {}, got '{}'", ctx_name, format_symbol_choices(&names), s)
}

/// Renders `["a", "b", "c"]` as `"'a, 'b, or 'c"` (or `"'a or 'b"` for two,
/// `"'a"` for one) — the quoted, Oxford-comma phrasing every enum-keyword
/// error message already used before this helper existed.
fn format_symbol_choices(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [a] => format!("'{a}"),
        [a, b] => format!("'{a} or '{b}"),
        [rest @ .., last] => {
            let rest: Vec<String> = rest.iter().map(|name| format!("'{name}")).collect();
            format!("{}, or '{last}", rest.join(", "))
        }
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

/// A Steel callable — `Closure`, `FuncV`, or `MutFunc`, the same three
/// variants `define-command!`'s own `proc` check accepts. Kept in one place
/// so a second "is this callable" decode (e.g. `live-picker!`'s
/// `#:command`) never drifts from that list.
pub(crate) fn callable_arg(val: SteelVal, ctx_name: &str) -> Result<SteelVal, SteelErr> {
    match val {
        SteelVal::Closure(_) | SteelVal::FuncV(_) | SteelVal::MutFunc(_) => Ok(val),
        _ => steel::stop!(TypeMismatch => "{}: expected a callable, got {:?}", ctx_name, val),
    }
}

/// `(%callable? v)` — backs `live-picker!`'s `#:command` check (see
/// `bootstrap.scm`), which must validate it in Scheme before composing the
/// debounced-respawn wrapper around it: once wrapped, `%live-picker!`'s own
/// `callable_arg` check on `on_query_change` sees only the wrapper closure,
/// not the caller's value. Deliberately `callable_arg`'s `is_ok()`, not
/// Steel's own `function?`/`procedure?` — those accept
/// `BoxedFunction`/`ContinuationFunction`/`BuiltIn` too, wider than the
/// three variants `callable_arg` (and `define-command!`'s `proc` check)
/// accept; a second, looser "is this callable" decode here would be exactly
/// the drift `callable_arg`'s own doc comment says to avoid.
pub(crate) fn is_callable(val: SteelVal) -> bool {
    callable_arg(val, "").is_ok()
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

/// A Steel list of integers, unpacked to `Vec<i32>` — `picker-source-spawn!`'s
/// `#:ok-exit-codes`, an exit-code allowlist. Rejects (rather than silently
/// truncating) a value outside `i32`'s range — every other decoder in this
/// file rejects out-of-domain input the same way, and a wrapped value here
/// would silently allowlist the wrong exit code.
pub(crate) fn list_to_i32s(val: SteelVal, ctx_name: &str) -> Result<Vec<i32>, SteelErr> {
    list_items(val, ctx_name)?
        .into_iter()
        .map(|v| {
            let n = int_arg(v, ctx_name)?;
            match i32::try_from(n) {
                Ok(n) => Ok(n),
                Err(_) => steel::stop!(TypeMismatch => "{}: {} is out of i32 range", ctx_name, n),
            }
        })
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

/// The single entry point for a JSON-shaped argument that might already be
/// a [`JsonHandle`](crate::json::JsonHandle) — an `lsp-request` response
/// threaded straight back in, e.g. `codeAction/resolve`'s params, or a
/// WorkspaceEdit/Location a plugin got from one and hands to
/// `apply-workspace-edit!`/`goto-location!` — or ordinary Steel data a
/// plugin built by hand (`(hash "line" 0 ...)`). An already-handle `val`
/// returns it as-is, no clone; anything else is converted once via
/// [`json_params`] and wrapped fresh. Delegating the non-handle path to
/// `json_params` (rather than the reverse) means the handle path here
/// never pays for a conversion it doesn't need.
pub(crate) fn json_arg(val: SteelVal, ctx_name: &str) -> Result<crate::json::JsonHandle, SteelErr> {
    if let Some(handle) = crate::json::downcast_json_handle(&val) {
        return Ok(handle);
    }
    Ok(crate::json::JsonHandle::new(json_params(val, ctx_name)?))
}

/// Converts `val` to the wire-shaped JSON a request/notification `params`
/// (or an `#:init-options`/`#:settings` blob) expects — an owned `Value` for
/// a caller that has to build an outgoing message (the value must outlive
/// this call to reach the wire). A host-trait method that only *reads* the
/// JSON takes [`json_arg`]'s handle directly instead, borrowing rather than
/// cloning. `steel_to_json`'s own `Custom` arm already unwraps a `JsonHandle`
/// to its resolved value's clone, so an already-handle `val` costs exactly
/// that one clone (unavoidable — the wire message needs an owned copy) with
/// no separate handle check here. Rejects a bool explicitly: several callers
/// pass through a value that is `#f` when absent, and without this check
/// that would silently reach a JSON consumer as `false` instead of erroring
/// at the boundary.
pub(crate) fn json_params(val: SteelVal, ctx_name: &str) -> Result<serde_json::Value, SteelErr> {
    if matches!(val, SteelVal::BoolV(_)) {
        steel::stop!(TypeMismatch => "{}: expected a hashmap or JSON handle, got a boolean", ctx_name);
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

/// A caught `with-handler` exception value that may be `#f` (no exception —
/// the activation succeeded). `#f` decodes as `None`; anything else decodes
/// as the `SteelErr` `with-handler` caught (`with-handler`'s lambda receives
/// the raised error's own `into_steelval()` form, so `SteelErr::from_steelval`
/// round-trips it with its span intact). Used only by
/// `%finish-lazy-activation`/`%finish-manifest-declare!`'s `error` argument —
/// see `bootstrap.scm`'s `%activate-plugin-inline`/`declare-plugin`.
pub(crate) fn optional_steel_error_arg(
    val: SteelVal,
    ctx_name: &str,
) -> Result<Option<SteelErr>, SteelErr> {
    match val {
        SteelVal::BoolV(false) => Ok(None),
        other => SteelErr::from_steelval(&other).map(Some).map_err(|_| {
            generic_err(format!(
                "{ctx_name}: expected #f or an error value, got {other:?}"
            ))
        }),
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

// ── List encoders ─────────────────────────────────────────────────────────────

/// Builds a proper Steel list from already-encoded `SteelVal`s — the shared
/// encode counterpart to `list_items`, for a builtin returning a list rather
/// than decoding one.
pub(crate) fn list_of(items: impl IntoIterator<Item = SteelVal>) -> SteelVal {
    SteelVal::ListV(items.into_iter().collect::<Vec<_>>().into())
}

/// A Steel list of strings — `list_of`'s common case, for a builtin whose
/// result is itself a list of `String`s (a diff hunk's lines, a tokenizer's
/// words) rather than a mix of `SteelVal` shapes.
pub(crate) fn string_list(items: impl IntoIterator<Item = String>) -> SteelVal {
    list_of(items.into_iter().map(|s| SteelVal::StringV(s.into())))
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

/// A dotted-pair argument that may be `#f` (absent) — `pair_fields`'s
/// `optional_*`-family sibling; `ctx_name`/`shape` stay caller-supplied so a
/// `None` decode reads no differently than `pair_fields`'s own error would.
pub(crate) fn optional_pair_fields(
    val: SteelVal,
    ctx_name: &str,
    shape: &str,
) -> Result<Option<(SteelVal, SteelVal)>, SteelErr> {
    match val {
        SteelVal::BoolV(false) => Ok(None),
        other => Ok(Some(pair_fields(other, ctx_name, shape)?)),
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
            Err(self.not_live_err(builtin_name))
        }
    }

    /// The shared "this bid names no open buffer" error — the wording behind
    /// [`require_live`](Self::require_live), for a builtin whose own host
    /// call already does the liveness lookup (so a second `buffer_exists`
    /// check would be redundant) but still needs `require_live`'s message.
    pub(crate) fn not_live_err(self, builtin_name: &str) -> SteelErr {
        generic_err(format!("{builtin_name}: invalid buffer id {:?}", self.0))
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

/// Decodes a `(line . character)` dotted pair into a
/// [`WirePos`](hume_rope::position_encoding::WirePos).
pub(crate) fn wire_pos_arg(
    val: SteelVal,
) -> Result<hume_rope::position_encoding::WirePos, SteelErr> {
    let (line, character) = pair_fields(val, "position", "(line . character)")?;
    Ok(hume_rope::position_encoding::WirePos {
        line: usize_arg(line, "position")?,
        character: usize_arg(character, "position")?,
    })
}

/// Decodes a wire `{"line" "character"}` hashmap. `what` names the calling
/// builtin in the error message — `wire_to_char` (the eventual conversion)
/// is total and clamps rather than errors, so this boundary check is the
/// only place a malformed shape gets caught instead of silently producing a
/// plausible-looking offset.
pub(crate) fn wire_position(
    v: &serde_json::Value,
    what: &str,
) -> Result<hume_rope::position_encoding::WirePos, SteelErr> {
    match (
        v.get("line").and_then(serde_json::Value::as_u64),
        v.get("character").and_then(serde_json::Value::as_u64),
    ) {
        (Some(line), Some(character)) => Ok(hume_rope::position_encoding::WirePos {
            line: line as usize,
            character: character as usize,
        }),
        _ => Err(generic_err(format!(
            "{what}: position must be a hashmap with numeric 'line' and 'character' keys, got {v}"
        ))),
    }
}

/// Decodes a wire `{"range" {"start" ... "end" ...} "newText" ...}`
/// `TextEdit` — a `JsonHandle`'s own resolved value, e.g. one element of a
/// `textDocument/formatting` response handed straight to
/// `apply-text-edits!` — into a [`WireTextEdit`](crate::host::WireTextEdit).
fn wire_text_edit_from_json(
    v: &serde_json::Value,
    ctx_name: &str,
) -> Result<crate::host::WireTextEdit, SteelErr> {
    let range = v
        .get("range")
        .ok_or_else(|| generic_err(format!("{ctx_name}: text edit handle has no 'range' field")))?;
    let start = range
        .get("start")
        .ok_or_else(|| generic_err(format!("{ctx_name}: text edit range has no 'start' field")))
        .and_then(|s| wire_position(s, ctx_name))?;
    let end = range
        .get("end")
        .ok_or_else(|| generic_err(format!("{ctx_name}: text edit range has no 'end' field")))
        .and_then(|e| wire_position(e, ctx_name))?;
    let new_text = v
        .get("newText")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            generic_err(format!(
                "{ctx_name}: text edit handle has no string 'newText' field"
            ))
        })?
        .to_string();
    Ok(crate::host::WireTextEdit {
        range: hume_rope::offset::ExclusiveRange::new(start, end),
        new_text,
    })
}

/// Decodes one `apply-text-edits!` entry into a
/// [`WireTextEdit`](crate::host::WireTextEdit) — either a `JsonHandle` onto
/// a wire `TextEdit` (an unconverted element straight from a
/// `textDocument/formatting`-shaped response, via [`wire_text_edit_from_json`])
/// or the pre-existing
/// `((start-line . start-character) (end-line . end-character) text)`
/// tuple shape (outer 3-tuple a list, inner positions dotted pairs) a
/// plugin builds by hand.
pub(crate) fn wire_text_edit_arg(val: SteelVal) -> Result<crate::host::WireTextEdit, SteelErr> {
    if let Some(handle) = crate::json::downcast_json_handle(&val) {
        return wire_text_edit_from_json(handle.value(), "apply-text-edits!");
    }
    let fields = checked_fields(
        val,
        "text edit",
        3..=3,
        "((start-line . start-character) (end-line . end-character) text) or a JSON handle",
    )?;
    let start = wire_pos_arg(fields[0].clone())?;
    let end = wire_pos_arg(fields[1].clone())?;
    Ok(crate::host::WireTextEdit {
        range: hume_rope::offset::ExclusiveRange::new(start, end),
        new_text: string_arg(fields[2].clone(), "text edit")?,
    })
}

#[cfg(test)]
mod tests;
