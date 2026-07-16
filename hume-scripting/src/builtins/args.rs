//! One marshalling vocabulary for builtins: plain `SteelVal` decoders,
//! `FromSteelVal` newtypes for buffer-id / position / text-edit params, and
//! the shared list/tuple decoders every multi-field setter builds on.
//!
//! No other module hand-rolls one of these decodes — enforced by
//! `rg 'expect\("len checked"\)' hume-scripting/src` returning empty (the
//! pattern every ad-hoc decoder used before this module existed).

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
/// skeleton every decoration setter (`set-signs!`, `set-virtual-lines!`, …)
/// opens with: unpack the outer list, check each entry's arity against
/// `shape`, then hand the checked, index-safe slice to `row` for
/// field-specific decoding.
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

// ── FromSteelVal newtypes ────────────────────────────────────────────────────
//
// Used as typed params in builtin signatures — steel-core's
// `register_fn_with_ctx` wrapper prepends the registered builtin name to any
// `from_steelval` failure automatically, so these messages carry no
// `ctx_name` of their own.

/// A decoded `BufferId` argument. Replaces the inline
/// `downcast_buffer_id(...).ok_or_else(...)` pattern every buffer-touching
/// builtin used to repeat.
#[derive(Debug)]
pub(crate) struct BidArg(pub(crate) BufferId);

impl FromSteelVal for BidArg {
    fn from_steelval(val: &SteelVal) -> Result<Self, SteelErr> {
        super::ids::downcast_buffer_id(val)
            .map(BidArg)
            .ok_or_else(|| SteelErr::new(ErrorKind::TypeMismatch, "expected buffer-id".to_string()))
    }
}

/// A decoded `(line col)` wire position — a 2-element list, not a `(line
/// . col)` dotted pair (steel-core 0.8.2's `Pair`/`car`/`cdr` are
/// crate-private, unreachable from a Rust builtin).
#[derive(Debug, Clone, Copy)]
pub(crate) struct PosArg {
    pub(crate) line: usize,
    pub(crate) col: usize,
}

impl FromSteelVal for PosArg {
    fn from_steelval(val: &SteelVal) -> Result<Self, SteelErr> {
        let fields = checked_fields(val.clone(), "position", 2..=2, "(line col)")?;
        Ok(PosArg {
            line: usize_arg(fields[0].clone(), "position")?,
            col: usize_arg(fields[1].clone(), "position")?,
        })
    }
}

/// A decoded `((start-line start-col) (end-line end-col) text)` LSP text
/// edit entry, wire positions.
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
            "((start-line start-col) (end-line end-col) text)",
        )?;
        Ok(TextEditArg {
            start: PosArg::from_steelval(&fields[0])?,
            end: PosArg::from_steelval(&fields[1])?,
            text: string_arg(fields[2].clone(), "text edit")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use steel::rvals::IntoSteelVal as _;

    fn list_of(items: &[&str]) -> SteelVal {
        items
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_steelval()
            .unwrap()
    }

    // ── Plain decoders ────────────────────────────────────────────────────────

    #[test]
    fn string_arg_accepts_string_and_symbol() {
        assert_eq!(string_arg(SteelVal::StringV("x".into()), "f").unwrap(), "x");
        assert_eq!(string_arg(SteelVal::SymbolV("x".into()), "f").unwrap(), "x");
    }

    #[test]
    fn string_arg_rejects_wrong_type_naming_the_arg() {
        let err = string_arg(SteelVal::IntV(1), "buffer-name").unwrap_err();
        assert!(err.to_string().contains("buffer-name"), "got: {err}");
        assert!(err.to_string().contains("expected a string"), "got: {err}");
    }

    #[test]
    fn optional_string_arg_false_is_none() {
        assert_eq!(
            optional_string_arg(SteelVal::BoolV(false), "f").unwrap(),
            None
        );
    }

    #[test]
    fn usize_arg_rejects_negative() {
        assert!(usize_arg(SteelVal::IntV(-1), "f").is_err());
    }

    #[test]
    fn optional_usize_arg_false_is_none_some_is_some() {
        assert_eq!(
            optional_usize_arg(SteelVal::BoolV(false), "f").unwrap(),
            None
        );
        assert_eq!(optional_usize_arg(SteelVal::IntV(4), "f").unwrap(), Some(4));
    }

    #[test]
    fn list_to_strings_rejects_non_string_element() {
        let list: SteelVal = vec![SteelVal::StringV("a".into()), SteelVal::IntV(1)]
            .into_steelval()
            .unwrap();
        assert!(list_to_strings(list, "f").is_err());
    }

    #[test]
    fn chars_arg_rejects_multi_char_entry() {
        let err = chars_arg(list_of(&["ab"]), "chars").unwrap_err();
        assert!(
            err.to_string().contains("exactly one character"),
            "got: {err}"
        );
    }

    #[test]
    fn chars_arg_accepts_single_char_entries() {
        assert_eq!(
            chars_arg(list_of(&["a", "b"]), "chars").unwrap(),
            vec!['a', 'b']
        );
    }

    #[test]
    fn json_params_rejects_bool() {
        let err = json_params(SteelVal::BoolV(true), "lsp-request params").unwrap_err();
        assert!(err.to_string().contains("lsp-request params"), "got: {err}");
    }

    // ── checked_fields / tuple_list ──────────────────────────────────────────

    #[test]
    fn checked_fields_rejects_wrong_arity() {
        let err = checked_fields(list_of(&["a"]), "f", 2..=2, "(a b)").unwrap_err();
        assert!(
            err.to_string().contains("each entry must be (a b)"),
            "got: {err}"
        );
    }

    #[test]
    fn checked_fields_accepts_arity_within_range() {
        assert_eq!(
            checked_fields(list_of(&["a", "b"]), "f", 2..=3, "(a b [c])")
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            checked_fields(list_of(&["a", "b", "c"]), "f", 2..=3, "(a b [c])")
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn tuple_list_decodes_each_entry_via_row() {
        let entries: SteelVal = vec![list_of(&["a", "b"]), list_of(&["c", "d"])]
            .into_steelval()
            .unwrap();
        let out = tuple_list(entries, "f", 2..=2, "(a b)", |fields| {
            Ok((
                string_arg(fields[0].clone(), "f")?,
                string_arg(fields[1].clone(), "f")?,
            ))
        })
        .unwrap();
        assert_eq!(
            out,
            vec![
                ("a".to_string(), "b".to_string()),
                ("c".to_string(), "d".to_string())
            ]
        );
    }

    #[test]
    fn tuple_list_propagates_a_bad_entry_arity() {
        let entries: SteelVal = vec![list_of(&["a"])].into_steelval().unwrap();
        let result = tuple_list(entries, "f", 2..=2, "(a b)", |fields| {
            string_arg(fields[0].clone(), "f")
        });
        assert!(result.is_err());
    }

    // ── BidArg ────────────────────────────────────────────────────────────────

    /// `BidArg::from_steelval` rejects a non-buffer-id `SteelVal`, preserving
    /// the "expected buffer-id" substring every buffer-touching builtin's
    /// wrong-type test asserts on.
    ///
    /// Fail oracle: return `Ok` for any value → any argument would silently
    /// decode as a buffer-id.
    #[test]
    fn bid_arg_rejects_non_buffer_id() {
        let err = BidArg::from_steelval(&SteelVal::StringV("not-an-id".into())).unwrap_err();
        assert!(err.to_string().contains("expected buffer-id"), "got: {err}");
    }

    #[test]
    fn bid_arg_accepts_a_real_buffer_id() {
        use crate::builtins::ids::SteelBufferId;
        let val = SteelBufferId(BufferId::default()).into_steelval().unwrap();
        assert_eq!(BidArg::from_steelval(&val).unwrap().0, BufferId::default());
    }

    // ── PosArg ────────────────────────────────────────────────────────────────

    #[test]
    fn pos_arg_decodes_line_col() {
        let val: SteelVal = vec![SteelVal::IntV(3), SteelVal::IntV(7)]
            .into_steelval()
            .unwrap();
        let pos = PosArg::from_steelval(&val).unwrap();
        assert_eq!((pos.line, pos.col), (3, 7));
    }

    #[test]
    fn pos_arg_rejects_wrong_arity() {
        let val: SteelVal = vec![SteelVal::IntV(3)].into_steelval().unwrap();
        assert!(PosArg::from_steelval(&val).is_err());
    }

    // ── TextEditArg ───────────────────────────────────────────────────────────

    #[test]
    fn text_edit_arg_decodes_start_end_text() {
        let start: SteelVal = vec![SteelVal::IntV(0), SteelVal::IntV(0)]
            .into_steelval()
            .unwrap();
        let end: SteelVal = vec![SteelVal::IntV(0), SteelVal::IntV(3)]
            .into_steelval()
            .unwrap();
        let val: SteelVal = vec![start, end, SteelVal::StringV("abc".into())]
            .into_steelval()
            .unwrap();
        let edit = TextEditArg::from_steelval(&val).unwrap();
        assert_eq!((edit.start.line, edit.start.col), (0, 0));
        assert_eq!((edit.end.line, edit.end.col), (0, 3));
        assert_eq!(edit.text, "abc");
    }

    #[test]
    fn text_edit_arg_rejects_malformed_position() {
        let bad_start: SteelVal = vec![SteelVal::IntV(0)].into_steelval().unwrap(); // wrong arity
        let end: SteelVal = vec![SteelVal::IntV(0), SteelVal::IntV(3)]
            .into_steelval()
            .unwrap();
        let val: SteelVal = vec![bad_start, end, SteelVal::StringV("abc".into())]
            .into_steelval()
            .unwrap();
        let err = TextEditArg::from_steelval(&val).unwrap_err();
        assert!(err.to_string().contains("(line col)"), "got: {err}");
    }
}
