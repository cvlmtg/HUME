//! Free functions for register and clipboard operations.
//!
//! Free functions (not `impl Editor` methods) so the same logic can be
//! called by both the `Editor` methods (thin delegators) and command bodies
//! that hold disjoint borrows from other `Editor` fields, avoiding the
//! whole-struct `&mut self` lock that forces callers to clone captured text.
//!
//! Each function that may emit a clipboard warning returns `Option<String>`
//! (the warning message). Callers report it via `ed.report(Severity::Warning, …)`.

use std::borrow::Cow;

use crate::editor::clipboard::{ClipboardRead, SystemClipboard};
use hume_ops::register::{CLIPBOARD_REGISTER, Piece, RegisterSet};

/// Pending state for the two-keystroke `"<reg>` register-prefix sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegisterPrefix {
    /// `"` pressed, waiting for the register-name character.
    Awaiting,
    /// Register name received; armed for the next yank/delete/change/paste.
    Selected(char),
}

/// Read text from an explicitly named register.
///
/// Returns `(pieces, warning)` where `warning` is `Some(msg)` when the OS
/// clipboard failed and the in-memory `'c'` mirror was used instead.
///
/// - `'c'` → OS clipboard; the in-memory mirror on failure (with a warning)
///   and when the clipboard is write-only OSC 52 (no warning).
/// - All others (`'0'`–`'9'`, etc.) → in-memory `RegisterSet`.
///
/// The kill-ring register (`'k'`) and black-hole register (`'b'`) are handled
/// upstream in `resolve_explicit_register`; this function is not called for them.
pub(in crate::editor) fn read_register_text<'a>(
    registers: &'a RegisterSet,
    clipboard: &mut SystemClipboard,
    name: char,
) -> (Option<Cow<'a, [Piece]>>, Option<String>) {
    if name == CLIPBOARD_REGISTER {
        let mirror = || {
            registers
                .read(CLIPBOARD_REGISTER)
                .and_then(|r| r.as_pieces())
                .map(Cow::Borrowed)
        };
        match clipboard.read() {
            ClipboardRead::Text(text) => {
                // When the OS clipboard matches what we last wrote, the in-memory
                // 'c' register is in sync, so prefer its structured pieces, which
                // keep multi-selection boundaries and each piece's shape.  When they differ,
                // the clipboard was externally modified; use its content directly.
                if registers.clipboard_blob() == Some(&text)
                    && let Some(mem) = mirror()
                {
                    return (Some(mem), None);
                }
                (Some(Cow::Owned(vec![Piece::from(text)])), None)
            }
            ClipboardRead::Mirror => (mirror(), None),
            ClipboardRead::Failed(e) => (mirror(), Some(clipboard_read_warn(&e))),
        }
    } else {
        let v = registers
            .read(name)
            .and_then(|r| r.as_pieces())
            .map(Cow::Borrowed);
        (v, None)
    }
}

/// Write `values` into named register `name`, routing `'c'` through the OS clipboard.
///
/// Returns `Some(warning)` if the native clipboard write failed; the text is
/// then sent through OSC 52 instead. The in-memory mirror is always updated.
pub(in crate::editor) fn write_register(
    registers: &mut RegisterSet,
    clipboard: &mut SystemClipboard,
    name: char,
    values: Vec<Piece>,
) -> Option<String> {
    if name == CLIPBOARD_REGISTER {
        let blob = registers.write_clipboard(values);
        clipboard
            .write(blob)
            .err()
            .map(|e| clipboard_write_warn(&e))
    } else {
        registers.write_text(name, values);
        None
    }
}

fn clipboard_read_warn(err: &str) -> String {
    format!("system clipboard unavailable ({err}), using in-memory 'c'")
}

fn clipboard_write_warn(err: &str) -> String {
    format!(
        "system clipboard unavailable ({err}), sent to the terminal (OSC 52) and kept in-memory 'c'"
    )
}

#[cfg(test)]
mod tests;
