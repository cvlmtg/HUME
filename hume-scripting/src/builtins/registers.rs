//! `(read-register name)` / `(write-register! name values)` builtins.
//!
//! A register holds one string per selection captured at yank time — the
//! same `Vec<String>` shape [`crate::host::RegisterHost`] exposes. Reading
//! and writing speak that one shape on both ends: `write-register!` takes a
//! list, `read-register` returns one (or `#f`), so a read result feeds
//! straight back into a write.
//!
//! Macro registers (recorded key sequences) are out of scope: there is no
//! wire format yet for handing a `Vec<KeyEvent>` to Scheme. `read-register`
//! on one answers `#f`, indistinguishable from an empty register — a future
//! addition could serialize the sequence instead, but nothing calls for that
//! today.

use steel::rerrs::SteelErr;
use steel::rvals::{IntoSteelVal, SteelVal};

use super::SteelResult;
use super::args::list_to_strings;
use super::errors::{generic_err, require_cap};
use crate::SteelCtx;

/// Decode a single-character register name and validate it against the same
/// set the `"<reg>` keymap prefix accepts (`0`–`9`, `k`, `c`, `b`) — shared by
/// `read-register`/`write-register!` here and `set-register-prefix!` in
/// `commands.rs`, so all three builtins reject `q`/`s`/multi-char names with
/// one wording.
pub(crate) fn register_arg(
    ctx: &mut SteelCtx,
    name: &str,
    builtin: &str,
) -> Result<char, SteelErr> {
    let mut chars = name.chars();
    let reg = match (chars.next(), chars.next()) {
        (Some(c), None) => c,
        _ => steel::stop!(Generic =>
            "{}: expected a single-character register name, got {:?}", builtin, name),
    };
    if !ctx.host.commands().is_valid_register_name(reg) {
        steel::stop!(Generic =>
            "{}: invalid register '{}'; valid: 0-9, k, c, b", builtin, reg);
    }
    Ok(reg)
}

/// `(read-register name)` → contents of `name` as a list of strings, or `#f`
/// if it's empty, is the black hole, or holds a recorded macro.
pub(crate) fn read_register(ctx: &mut SteelCtx, name: String) -> SteelResult {
    let reg = register_arg(ctx, &name, "read-register")?;
    let registers = require_cap(ctx.host.registers(), "read-register")?;
    match registers.read_register(reg) {
        Some(values) => values.into_steelval().map_err(generic_err),
        None => Ok(SteelVal::BoolV(false)),
    }
}

/// `(write-register! name values)` → store `values` (a list of strings, one
/// per selection) in register `name`.
pub(crate) fn write_register(ctx: &mut SteelCtx, name: String, values: SteelVal) -> SteelResult {
    let reg = register_arg(ctx, &name, "write-register!")?;
    let values = list_to_strings(values, "write-register!")?;
    let registers = require_cap(ctx.host.registers(), "write-register!")?;
    registers.write_register(reg, values);
    Ok(SteelVal::Void)
}

#[cfg(test)]
mod tests;
