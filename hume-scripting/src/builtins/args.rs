//! One marshalling vocabulary for builtins: plain `SteelVal` decoders,
//! `FromSteelVal` newtypes for pane params (`ArgPane`/`LivePane`, plus
//! `Usize`/`OptUsize` for a `LivePane`-sibling argument that
//! needs the same steel-core-driven, ahead-of-the-closure-body decode;
//! see `BuiltinArg`, the seam that runs `LivePane`'s liveness check),
//! free-fn decoders for wire position / text-edit params (`WirePos` is a
//! `hume-rope` type, so the orphan rule rules out `FromSteelVal for
//! WirePos` here, so a plain function is used for its sibling
//! `WireTextEdit` decoder too, for one calling convention across both),
//! and the shared list/hash decoders every multi-field setter builds on.
//!
//! `#f`-means-absent is decoded only by this module's `optional_*` family
//! (plus `OptUsize`'s own `FromSteelVal` impl, which lives in
//! this same file). Enforced by `cargo test
//! absent_marker_is_decoded_only_in_args_rs` (`arch-lints/tests/
//! absent_decode.rs`).

use std::ops::RangeInclusive;
use std::path::PathBuf;

use steel::rerrs::{ErrorKind, SteelErr};
use steel::rvals::{FromSteelVal, SteelVal};
use termina::event::KeyEvent;

use hume_engine::pipeline::BufferId;

use crate::keys::parse_key_sequence;
use crate::types::PaneHandle;

use super::errors::generic_err;

/// The shared "this bid names no open buffer" error: one wording for every
/// liveness check that raises on a stale `PaneHandle`'s buffer: [`LivePane`]'s
/// own decode-time check, [`LspTargetArg::Buffer`]'s, and every
/// `buffers.rs`/`diff.rs` builtin whose host call already does the liveness
/// lookup (so a second `buffer_exists` check would be redundant) but still
/// needs this message.
pub(crate) fn not_live_err(name: &str, bid: BufferId) -> SteelErr {
    generic_err(format!("{name}: invalid buffer id {bid:?}"))
}

// ── Plain decoders ──────────────────────────────────────────────────────────
//
// Calling convention: `(val, ctx_name: &str)`, one Rust param per Steel arg
// (the convention `register_fn_with_ctx` builtins use, not a `&[SteelVal]`
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

/// A key-spec string (`bind-key!`'s own syntax, decoded via
/// [`parse_key_sequence`]) that must name exactly one chord, not a
/// sequence. Shared by `insert-key!` (one key, no destination for a
/// sequence) and `picker!`/`live-picker!`'s `#:actions` (one chord
/// dispatched at a time).
pub(crate) fn single_key_arg(val: SteelVal, ctx_name: &str) -> Result<KeyEvent, SteelErr> {
    let spec = string_arg(val, ctx_name)?;
    let mut keys = parse_key_sequence(&spec)
        .map_err(|e| generic_err(format!("{ctx_name}: invalid key spec '{spec}': {e}")))?;
    if keys.len() != 1 {
        steel::stop!(Generic => "{}: key spec '{}' must name exactly one key, not a sequence", ctx_name, spec);
    }
    Ok(keys.remove(0))
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

/// A symbol argument that may be `#f` (absent). Unlike `optional_string_arg`,
/// it rejects a plain string: a caller wanting this shape is decoding a Scheme
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
/// values via a `(spelling, value)` table: the "must be 'a, 'b, or 'c, got
/// 'x" shape shared by every Steel enum-keyword decoder. Callers
/// extract the string themselves first (`string_arg` for a string-or-symbol
/// argument, a stricter symbol-only check where the wire contract insists on
/// a bare symbol). This helper is only about the mapping and the error
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
/// `"'a"` for one): the quoted, Oxford-comma phrasing every enum-keyword
/// error message uses.
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

pub(crate) fn int_arg(val: SteelVal, ctx_name: &str) -> Result<i64, SteelErr> {
    match val {
        SteelVal::IntV(n) => Ok(n as i64),
        _ => steel::stop!(TypeMismatch => "{}: expected an integer", ctx_name),
    }
}

pub(crate) fn bool_arg(val: SteelVal, ctx_name: &str) -> Result<bool, SteelErr> {
    match val {
        SteelVal::BoolV(b) => Ok(b),
        _ => steel::stop!(TypeMismatch => "{}: expected a bool", ctx_name),
    }
}

/// A Steel callable: `Closure`, `FuncV`, or `MutFunc`, the same three
/// variants `define-command!`'s own `proc` check accepts. Kept in one place
/// so a second "is this callable" decode (e.g. `live-picker!`'s
/// `#:command`) never drifts from that list.
pub(crate) fn callable_arg(val: SteelVal, ctx_name: &str) -> Result<SteelVal, SteelErr> {
    match val {
        SteelVal::Closure(_) | SteelVal::FuncV(_) | SteelVal::MutFunc(_) => Ok(val),
        _ => steel::stop!(TypeMismatch => "{}: expected a callable, got {:?}", ctx_name, val),
    }
}

/// `(%callable? v)`: backs `live-picker!`'s `#:command` check (see
/// `bootstrap.scm`), which must validate it in Scheme before composing the
/// debounced-respawn wrapper around it: once wrapped, `%live-picker!`'s own
/// `callable_arg` check on `on_query_change` sees only the wrapper closure,
/// not the caller's value. Deliberately `callable_arg`'s `is_ok()`, not
/// Steel's own `function?`/`procedure?`, which accept
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

/// A Steel list of integers, unpacked to `Vec<i32>`: `picker-source-spawn!`'s
/// `#:ok-exit-codes`, an exit-code allowlist. Rejects (rather than silently
/// truncating) a value outside `i32`'s range. Every other decoder in this
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
/// `Vec<(String, String)>`: the wire shape for `register-lsp-server!`'s
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
/// a [`JsonHandle`](crate::json::JsonHandle) (an `lsp-request!` response
/// threaded straight back in, e.g. `codeAction/resolve`'s params, or a
/// WorkspaceEdit/Location a plugin got from one and hands to
/// `apply-workspace-edit!`/`goto-location!`) or ordinary Steel data a
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
/// (or an `#:init-options`/`#:settings` blob) expects: an owned `Value` for
/// a caller that has to build an outgoing message (the value must outlive
/// this call to reach the wire). A host-trait method that only *reads* the
/// JSON takes [`json_arg`]'s handle directly instead, borrowing rather than
/// cloning. `steel_to_json`'s own `Custom` arm already unwraps a `JsonHandle`
/// to its resolved value's clone, so an already-handle `val` costs exactly
/// that one clone (unavoidable, since the wire message needs an owned copy) with
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

/// A caught `with-handler` exception value that may be `#f` (no exception:
/// the activation succeeded). `#f` decodes as `None`; anything else decodes
/// as the `SteelErr` `with-handler` caught (`with-handler`'s lambda receives
/// the raised error's own `into_steelval()` form, so `SteelErr::from_steelval`
/// round-trips it with its span intact). Used only by
/// `%finish-lazy-activation!`/`%finish-manifest-declare!`'s `error` argument;
/// see `bootstrap.scm`'s `%activate-plugin-inline!`/`declare-plugin!`.
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

/// Unpacks `val` as a list and errors unless its length falls in `arity`:
/// the shared shape check every fixed-field entry (a position pair, a text
/// edit) opens with.
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

// ── Symbol-keyed hash entries ────────────────────────────────────────────────

/// One entry of a hash-shaped list (a decoration setter's `(hash 'line …)`),
/// its keys already checked against the caller's own key set by
/// [`hash_list`].
pub(crate) struct HashEntry<'a> {
    map: steel::rvals::SteelHashMap,
    ctx_name: &'a str,
}

impl HashEntry<'_> {
    pub(crate) fn optional(&self, key: &str) -> Option<SteelVal> {
        self.map.get(&SteelVal::SymbolV(key.into())).cloned()
    }

    pub(crate) fn required(&self, key: &str) -> Result<SteelVal, SteelErr> {
        self.optional(key)
            .ok_or_else(|| generic_err(format!("{}: missing '{key}", self.ctx_name)))
    }
}

/// Decodes a Steel list of symbol-keyed hashmaps into `Vec<T>`: the shared
/// skeleton every decoration setter opens with. Each entry must be a
/// hashmap whose keys are all symbols drawn from `keys`; an unknown or
/// non-symbol key errors, so a misspelled key is reported, not ignored. Which
/// keys are required is `row`'s call, via [`HashEntry::required`].
pub(crate) fn hash_list<T>(
    val: SteelVal,
    ctx_name: &str,
    keys: &[&str],
    mut row: impl FnMut(&HashEntry) -> Result<T, SteelErr>,
) -> Result<Vec<T>, SteelErr> {
    let expected = || {
        keys.iter()
            .map(|k| format!("'{k}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    list_items(val, ctx_name)?
        .into_iter()
        .map(|entry| {
            let SteelVal::HashMapV(map) = entry else {
                steel::stop!(TypeMismatch =>
                    "{}: each entry must be a hashmap with keys from {}", ctx_name, expected());
            };
            for (key, _) in map.iter() {
                let SteelVal::SymbolV(name) = key else {
                    steel::stop!(Generic =>
                        "{}: hashmap key must be a symbol, got {:?}", ctx_name, key);
                };
                if !keys.contains(&name.as_str()) {
                    steel::stop!(Generic =>
                        "{}: unknown key '{}, expected one of {}", ctx_name, name, expected());
                }
            }
            row(&HashEntry { map, ctx_name })
        })
        .collect()
}

// ── List encoders ─────────────────────────────────────────────────────────────

/// Builds a proper Steel list from already-encoded `SteelVal`s: the shared
/// encode counterpart to `list_items`, for a builtin returning a list rather
/// than decoding one.
pub(crate) fn list_of(items: impl IntoIterator<Item = SteelVal>) -> SteelVal {
    SteelVal::ListV(items.into_iter().collect::<Vec<_>>().into())
}

/// A Steel list of strings: `list_of`'s common case, for a builtin whose
/// result is itself a list of `String`s (a diff hunk's lines, a tokenizer's
/// words) rather than a mix of `SteelVal` shapes.
pub(crate) fn string_list(items: impl IntoIterator<Item = String>) -> SteelVal {
    list_of(items.into_iter().map(|s| SteelVal::StringV(s.into())))
}

// ── Dotted-pair decoders/encoders ────────────────────────────────────────────

/// Unpacks `val` as a dotted pair `(car . cdr)`: the shared decode for wire
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

/// A dotted-pair argument that may be `#f` (absent): `pair_fields`'s
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

/// Builds a dotted pair `(a . b)`: the shared encode counterpart to
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
// Used as typed params in builtin signatures. steel-core's
// `register_fn_with_ctx` wrapper prepends the registered builtin name to any
// `from_steelval` failure automatically, so these messages carry no
// `ctx_name` of their own.

/// A decoded [`PaneHandle`] argument, tolerant of a since-closed buffer,
/// for a builtin whose own contract is "answer `#f`/empty for a `pane`
/// this host doesn't currently show anything for," where a closed buffer is
/// just one more case of that, not a distinct error (`buffer-path`,
/// `lsp-capabilities`, …). [`LivePane`] is the counterpart for a builtin
/// that must raise on a closed buffer instead. Neither checks the pane half
/// live: a builtin that needs the pane itself (kind A/B, see
/// `hume-editor`'s `CommandPane::resolve`/`FocusedPane::resolve`) resolves and checks it through the host,
/// not here; a kind-C builtin never looks at `.pane()` at all.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ArgPane(pub(crate) PaneHandle);

impl FromSteelVal for ArgPane {
    fn from_steelval(val: &SteelVal) -> Result<Self, SteelErr> {
        super::ids::downcast_pane(val)
            .map(ArgPane)
            .ok_or_else(|| SteelErr::new(ErrorKind::TypeMismatch, "expected pane".to_string()))
    }
}

/// A [`PaneHandle`] argument checked live during decoding (see
/// [`BuiltinArg`]'s impl below): the funnel every explicit-pane builtin
/// that must raise on a closed buffer (rather than answer `#f`/empty, see
/// [`ArgPane`]) declares its `pane` parameter as, in the `builtins!` table.
/// The private field keeps `FromSteelVal` (type only) and `BuiltinArg`
/// (type + liveness) as the only two ways to produce one, so a builtin can't
/// accidentally skip the liveness half by constructing this directly.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LivePane(PaneHandle);

impl FromSteelVal for LivePane {
    fn from_steelval(val: &SteelVal) -> Result<Self, SteelErr> {
        super::ids::downcast_pane(val)
            .map(LivePane)
            .ok_or_else(|| SteelErr::new(ErrorKind::TypeMismatch, "expected pane".to_string()))
    }
}

/// One marshalling step every ctx-taking `builtins!` table entry (`cmd`,
/// `config`, `open`) runs each declared argument through, right after its
/// `FromSteelVal` decode and its eval-mode gate: the seam [`LivePane`]'s
/// liveness check hooks into without every builtin body repeating
/// `ctx.host.buffers().buffer_exists(...)` by hand. Every other declared
/// arg type is `Self`-identity here: decode is already everything they
/// need. `name` is the registered Steel name (the same `$name` the gate
/// gets), so a `LivePane` failure's message names the builtin without the
/// function body supplying it.
pub(crate) trait BuiltinArg {
    type Out;
    fn resolve(self, ctx: &mut crate::SteelCtx, name: &'static str) -> Result<Self::Out, SteelErr>;
}

/// `BuiltinArg::resolve` for a type whose decode is already everything it
/// needs: `Self::Out = Self`, `resolve` a no-op `Ok(self)`. One list
/// instead of four (`SteelVal`, `String`, `bool`, `ArgPane`) hand-copying
/// the same nine-line body.
macro_rules! identity_builtin_arg {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl BuiltinArg for $ty {
                type Out = $ty;
                fn resolve(
                    self,
                    _ctx: &mut crate::SteelCtx,
                    _name: &'static str,
                ) -> Result<Self::Out, SteelErr> {
                    Ok(self)
                }
            }
        )+
    };
}

identity_builtin_arg!(SteelVal, String, bool, ArgPane);

impl BuiltinArg for LivePane {
    type Out = PaneHandle;
    fn resolve(
        self,
        ctx: &mut crate::SteelCtx,
        name: &'static str,
    ) -> Result<PaneHandle, SteelErr> {
        if ctx.host.buffers().buffer_exists(self.0.buffer()) {
            Ok(self.0)
        } else {
            Err(not_live_err(name, self.0.buffer()))
        }
    }
}

/// [`usize_arg`]'s `FromSteelVal` counterpart, for a `LivePane`-sibling
/// argument that must decode (via steel-core's own per-argument
/// `FromSteelVal` pass, left to right, ahead of every builtin body) before
/// `LivePane`'s liveness check ever runs, so a malformed sibling argument
/// isn't masked behind a stale-buffer error. `usize_arg`'s plain-function
/// form stays the one every other (no-`LivePane`) builtin decodes its own
/// integer arguments with; this exists only where the ordering matters.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Usize(pub(crate) usize);

impl FromSteelVal for Usize {
    fn from_steelval(val: &SteelVal) -> Result<Self, SteelErr> {
        match val {
            SteelVal::IntV(n) if *n >= 0 => Ok(Usize(*n as usize)),
            _ => Err(SteelErr::new(
                ErrorKind::TypeMismatch,
                "expected a non-negative integer".to_string(),
            )),
        }
    }
}

impl BuiltinArg for Usize {
    type Out = usize;
    fn resolve(self, _ctx: &mut crate::SteelCtx, _name: &'static str) -> Result<usize, SteelErr> {
        Ok(self.0)
    }
}

/// [`Usize`], but `#f` decodes to `None`: [`optional_usize_arg`]'s
/// `FromSteelVal` counterpart.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OptUsize(pub(crate) Option<usize>);

impl FromSteelVal for OptUsize {
    fn from_steelval(val: &SteelVal) -> Result<Self, SteelErr> {
        match val {
            SteelVal::BoolV(false) => Ok(OptUsize(None)),
            other => Usize::from_steelval(other).map(|Usize(n)| OptUsize(Some(n))),
        }
    }
}

impl BuiltinArg for OptUsize {
    type Out = Option<usize>;
    fn resolve(
        self,
        _ctx: &mut crate::SteelCtx,
        _name: &'static str,
    ) -> Result<Option<usize>, SteelErr> {
        Ok(self.0)
    }
}

/// `(lsp-stop! target)` / `(lsp-restart! target)`'s `target` argument, ahead
/// of [`BuiltinArg::resolve`]'s liveness check on the `Buffer` case. A pane
/// decodes like [`LivePane`] (only its buffer is used: this is a kind-C,
/// buffer-only operation), a string or symbol names a language (required:
/// there is no "focused buffer" fallback to decode `#f` into).
#[derive(Debug)]
pub(crate) enum LspTargetArg {
    Buffer(LivePane),
    Language(String),
}

impl FromSteelVal for LspTargetArg {
    fn from_steelval(val: &SteelVal) -> Result<Self, SteelErr> {
        if let Ok(pane) = LivePane::from_steelval(val) {
            return Ok(LspTargetArg::Buffer(pane));
        }
        match val {
            SteelVal::StringV(s) => Ok(LspTargetArg::Language(s.to_string())),
            SteelVal::SymbolV(s) => Ok(LspTargetArg::Language(s.to_string())),
            _ => Err(SteelErr::new(
                ErrorKind::TypeMismatch,
                "expected a pane or a language name".to_string(),
            )),
        }
    }
}

impl BuiltinArg for LspTargetArg {
    type Out = crate::types::LspServerTarget;
    fn resolve(
        self,
        ctx: &mut crate::SteelCtx,
        name: &'static str,
    ) -> Result<crate::types::LspServerTarget, SteelErr> {
        match self {
            LspTargetArg::Buffer(pane) => Ok(crate::types::LspServerTarget::Buffer(
                pane.resolve(ctx, name)?.buffer(),
            )),
            LspTargetArg::Language(language) => {
                Ok(crate::types::LspServerTarget::Language(language))
            }
        }
    }
}

/// Decodes a wire `{"line" "character"}` hashmap. `what` names the calling
/// builtin in the error message: `wire_to_char` (the eventual conversion)
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
/// `TextEdit` (a `JsonHandle`'s own resolved value, e.g. one element of a
/// `textDocument/formatting` response handed straight to
/// `apply-text-edits!`) into a [`WireTextEdit`](crate::host::WireTextEdit).
/// `encoding` is the handle's own tag, passed in rather than re-read here so
/// this stays the plain JSON-shape decoder [`wire_position`] already is.
fn wire_text_edit_from_json(
    v: &serde_json::Value,
    encoding: hume_rope::position_encoding::PositionEncoding,
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
        encoding,
    })
}

/// Decodes one `apply-text-edits!` entry into a
/// [`WireTextEdit`](crate::host::WireTextEdit): a `JsonHandle` onto a wire
/// `TextEdit` (an unconverted element straight from a
/// `textDocument/formatting`-shaped response), never a hand-built shape: a
/// hand-built tuple would have no producing server to read an encoding off,
/// and guessing one is exactly what `JsonHandle::position_encoding` refuses
/// to do.
pub(crate) fn wire_text_edit_arg(val: SteelVal) -> Result<crate::host::WireTextEdit, SteelErr> {
    let Some(handle) = crate::json::downcast_json_handle(&val) else {
        return Err(SteelErr::new(
            ErrorKind::TypeMismatch,
            "apply-text-edits!: expected a JSON handle onto a wire TextEdit".to_string(),
        ));
    };
    let encoding = handle
        .position_encoding("apply-text-edits!")
        .map_err(generic_err)?;
    wire_text_edit_from_json(handle.value(), encoding, "apply-text-edits!")
}

#[cfg(test)]
mod tests;
