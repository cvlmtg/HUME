//! BufferText-edit application and cursor navigation primitives fed by LSP
//! responses (code actions, rename, go-to-definition); `insert-key!` too,
//! which isn't LSP-driven but shares `EditHost`'s capability gate (see that
//! trait's own doc).

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::types::PaneHandle;

use super::SteelResult;
use super::args::{
    CHAR_OFFSET, hash_entry, list_items, optional_usize_arg, single_key_arg, string_arg,
    strings_arg, tagged_json_arg, wire_text_edit_arg,
};
use super::errors::{generic_err, require_cap};

/// `(%apply-text-edits! pane edits expect-gen)`. `edits` is a list of
/// `JsonHandle`s onto wire `TextEdit`s (unconverted response elements, e.g.
/// from `textDocument/formatting`); see `wire_text_edit_arg`.
///
/// `edits` decodes manually via `wire_text_edit_arg` per entry rather than a
/// typed `Vec<WireTextEdit>` param: steel-core's blanket
/// `FromSteelVal for Vec<T>` impl discards the inner per-element error on
/// failure, replacing it with a generic message; decoding manually keeps
/// `wire_text_edit_arg`'s specific shape-error text.
pub(crate) fn apply_text_edits(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    edits: SteelVal,
    expect_gen: SteelVal,
) -> SteelResult {
    let expect_gen =
        optional_usize_arg(expect_gen, "apply-text-edits! expect-gen")?.map(|n| n as u64);
    let parsed = list_items(edits, "apply-text-edits! edits")?
        .into_iter()
        .map(wire_text_edit_arg)
        .collect::<Result<Vec<_>, SteelErr>>()?;
    require_cap(ctx.host.edits(), "apply-text-edits!")?
        .apply_text_edits(pane, parsed, expect_gen)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(%apply-workspace-edit! pane wsedit expect-gen)`. `wsedit` is a JSON
/// handle onto a `WorkspaceEdit` from a server response. Its positions
/// decode using the handle's own tagged encoding (the server that produced
/// it; see `JsonHandle::position_encoding`), so unlike `goto-location!`'s
/// char-indexed shape a hand-built value raises: there is no server to have
/// negotiated an encoding with. Returns the number of buffers modified; the
/// `apply-workspace-edit!` Scheme wrapper reports that count.
pub(crate) fn apply_workspace_edit(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    wsedit: SteelVal,
    expect_gen: SteelVal,
) -> SteelResult {
    let (handle, encoding) = tagged_json_arg(&wsedit, "apply-workspace-edit!")?;
    let expect_gen =
        optional_usize_arg(expect_gen, "apply-workspace-edit! expect-gen")?.map(|n| n as u64);
    let count = require_cap(ctx.host.edits(), "apply-workspace-edit!")?
        .apply_workspace_edit(pane, handle.value(), encoding, expect_gen)
        .map_err(generic_err)?;
    Ok(SteelVal::IntV(count as isize))
}

/// `(goto-location! pane loc)` → `#t` after a jump, `#f` when the place `loc`
/// names no longer exists. `loc` is one of three shapes, dispatched here (not
/// in Scheme):
///
/// - a JSON handle onto a raw `Location`/`LocationLink` from a server
///   response: wire position, decoded and converted using the handle's own
///   tagged encoding. The server that produced the response negotiated it
///   for the request that's being answered, regardless of which file the
///   location points into (an LSP round-trip is async; the user is free to
///   switch panes while a request is in flight, which is exactly why the
///   jump lands in `pane`, not necessarily the focused one). Errors on a
///   hand-built value, same as `apply-workspace-edit!`; `#f` when the
///   location's line is past its file's last line.
/// - `(hash 'target path 'line l 'char-col c)`, already char-indexed: `path`
///   is a path string, never a URI, and the file is opened if it is not
///   already. `#f` when `l` is past the file's last line.
/// - `(hash 'target pane 'char-offset o)`, a char offset in `pane`'s buffer.
///   Raises when `o` is past the end of the buffer.
///
/// A hashmap is one of the char-indexed shapes and a handle is the wire one.
/// Keys that belong to the other char-indexed shape raise.
pub(crate) fn goto_location(ctx: &mut SteelCtx, pane: PaneHandle, loc: SteelVal) -> SteelResult {
    match &loc {
        SteelVal::HashMapV(map)
            if map.iter().any(
                |(k, _)| matches!(k, SteelVal::SymbolV(name) if name.as_str() == "target"),
            ) =>
        {
            let fields = hash_entry(
                loc.clone(),
                "goto-location!",
                &["target", "line", "char-col", CHAR_OFFSET],
            )?;
            let target = fields.required("target")?;
            if let Some(handle) = super::ids::downcast_pane(&target) {
                for key in ["line", "char-col"] {
                    if fields.optional(key).is_some() {
                        steel::stop!(Generic =>
                            "goto-location!: a pane target takes 'char-offset, not '{}", key);
                    }
                }
                let char_offset = fields.usize(CHAR_OFFSET)?;
                require_cap(ctx.host.edits(), "goto-location!")?
                    .goto_location_buffer(pane, handle.buffer(), char_offset)
                    .map(|()| SteelVal::BoolV(true))
                    .map_err(goto_err)
            } else {
                if fields.optional(CHAR_OFFSET).is_some() {
                    steel::stop!(Generic =>
                        "goto-location!: a path target takes 'line and 'char-col, not 'char-offset");
                }
                let path = string_arg(target, "goto-location! target")?;
                if has_uri_scheme(&path) {
                    steel::stop!(Generic =>
                        "goto-location!: target must be a pane or a path, not the URI {}", path);
                }
                let line = fields.usize("line")?;
                let char_col = fields.usize("char-col")?;
                require_cap(ctx.host.edits(), "goto-location!")?
                    .goto_location_path(pane, path, line, char_col)
                    .map(SteelVal::BoolV)
                    .map_err(goto_err)
            }
        }
        SteelVal::Custom(_) => {
            let (handle, encoding) = tagged_json_arg(&loc, "goto-location!")?;
            require_cap(ctx.host.edits(), "goto-location!")?
                .goto_location_value(pane, handle.value(), encoding)
                .map(SteelVal::BoolV)
                .map_err(goto_err)
        }
        _ => steel::stop!(TypeMismatch =>
            "goto-location!: expected a Location handle from a server response or (hash 'target t 'line l 'char-col c) or (hash 'target pane 'char-offset o)"),
    }
}

/// Whether `s` starts with an RFC 3986 scheme and its `:`: an ASCII letter,
/// then letters, digits, `+`, `-` or `.`. A one-letter scheme is a Windows
/// drive (`C:\…`), so it does not count. A relative path whose first segment
/// holds a colon (`foo:bar/x`) reads as a URI too; `./foo:bar/x` does not.
fn has_uri_scheme(s: &str) -> bool {
    let Some((scheme, _)) = s.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    scheme.len() >= 2
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

fn goto_err(e: String) -> SteelErr {
    generic_err(format!("goto-location!: {e}"))
}

/// `(insert-key! pane key)`: `key` is a `bind-key!`-syntax spec naming
/// exactly one chord (see [`single_key_arg`]), decoded here rather than
/// left to the host: the host trait takes an already-parsed `KeyEvent`,
/// same as every other typed `EditHost` param.
pub(crate) fn insert_key(ctx: &mut SteelCtx, pane: PaneHandle, key: SteelVal) -> SteelResult {
    let key = single_key_arg(key, "insert-key!")?;
    require_cap(ctx.host.edits(), "insert-key!")?
        .insert_key(pane, key)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(paste-text! pane text)`: paste `text`, a string or a list of strings,
/// into whatever has focus.
pub(crate) fn paste_text(ctx: &mut SteelCtx, pane: PaneHandle, text: SteelVal) -> SteelResult {
    let values = strings_arg(text, "paste-text!")?;
    require_cap(ctx.host.edits(), "paste-text!")?
        .paste_text(pane, values)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

/// `(goto-revision! pane id)`: move `pane`'s buffer to revision `id` of its
/// undo history (a number from `(buffer-undo-tree pane)`), across branches.
/// Raises when the buffer has no such revision or is read-only.
pub(crate) fn goto_revision(ctx: &mut SteelCtx, pane: PaneHandle, revision: usize) -> SteelResult {
    require_cap(ctx.host.edits(), "goto-revision!")?
        .goto_revision(pane, revision)
        .map(|()| SteelVal::Void)
        .map_err(generic_err)
}

#[cfg(test)]
mod tests;
