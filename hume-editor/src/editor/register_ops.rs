//! Free functions for register and clipboard operations.
//!
//! Free functions (not `impl Editor` methods) so the same logic can be
//! called by both the `Editor` methods (thin delegators) and command bodies
//! that hold disjoint borrows from other `Editor` fields — avoiding the
//! whole-struct `&mut self` lock that forces callers to clone captured text.
//!
//! Each function that may emit a clipboard warning returns `Option<String>`
//! (the warning message). Callers report it via `ed.report(Severity::Warning, …)`.

use std::borrow::Cow;

use crate::editor::clipboard::SystemClipboard;
use hume_editing::text::normalize_line_endings;
use hume_ops::register::{CLIPBOARD_REGISTER, RegisterSet, is_register_linewise};

/// Pending state for the two-keystroke `"<reg>` register-prefix sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegisterPrefix {
    /// `"` pressed — waiting for the register-name character.
    Awaiting,
    /// Register name received; armed for the next yank/delete/change/paste.
    Selected(char),
}

/// Read text from an explicitly named register.
///
/// Returns `(values, warning)` where `warning` is `Some(msg)` when the OS
/// clipboard was unavailable and the in-memory `'c'` mirror was used instead.
///
/// - `'c'` → OS clipboard (in-memory fallback on failure).
/// - All others (`'0'`–`'9'`, etc.) → in-memory `RegisterSet`.
///
/// The kill-ring register (`'k'`) and black-hole register (`'b'`) are handled
/// upstream in `resolve_explicit_register`; this function is not called for them.
pub(crate) fn read_register_text<'a>(
    registers: &'a RegisterSet,
    clipboard: &mut SystemClipboard,
    name: char,
) -> (Option<Cow<'a, [String]>>, Option<String>) {
    if name == CLIPBOARD_REGISTER {
        match clipboard.read() {
            Ok(text) => {
                // When the OS clipboard matches what we last wrote, the in-memory
                // 'c' register is in sync — prefer its structured Vec<String>,
                // which preserves multi-selection boundaries.  When they differ,
                // the clipboard was externally modified; use its content directly.
                if registers.clipboard_blob() == Some(&text)
                    && let Some(mem) = registers.read(CLIPBOARD_REGISTER).and_then(|r| r.as_text())
                {
                    return (Some(Cow::Borrowed(mem)), None);
                }
                // Only past the blob-equality check above, which compares
                // against the raw bytes HUME itself last wrote to the OS —
                // normalizing before that check would break self-round-trip
                // detection for a CRLF-bearing clipboard entry.
                let text = normalize_line_endings(&text).into_owned();
                (Some(Cow::Owned(vec![text])), None)
            }
            Err(e) => {
                let warning = clipboard_warn(&e);
                let fallback = registers
                    .read(CLIPBOARD_REGISTER)
                    .and_then(|r| r.as_text())
                    .map(Cow::Borrowed);
                (fallback, Some(warning))
            }
        }
    } else {
        let v = registers
            .read(name)
            .and_then(|r| r.as_text())
            .map(Cow::Borrowed);
        (v, None)
    }
}

/// Write `values` into named register `name`, routing `'c'` through the OS clipboard.
///
/// Returns `Some(warning)` if the clipboard write failed; the in-memory mirror
/// is always updated regardless.
pub(crate) fn write_register(
    registers: &mut RegisterSet,
    clipboard: &mut SystemClipboard,
    name: char,
    values: Vec<String>,
) -> Option<String> {
    if name == CLIPBOARD_REGISTER {
        // Build the OS clipboard blob per-element: insert a '\n' separator
        // only when the previous value does not already end in one (linewise).
        // This matches how paste consumes each value independently and correctly
        // handles mixed selections (e.g. ["line\n", "word"] → "line\nword",
        // not "line\n\nword").
        let mut blob = String::new();
        for (i, v) in values.iter().enumerate() {
            if i > 0 && !is_register_linewise(&values[i - 1]) {
                blob.push('\n');
            }
            blob.push_str(v);
        }
        let warning = clipboard.write(&blob).err().map(|e| clipboard_warn(&e));
        registers.write_text(CLIPBOARD_REGISTER, values);
        registers.set_clipboard_blob(blob);
        warning
    } else {
        registers.write_text(name, values);
        None
    }
}

fn clipboard_warn(err: &str) -> String {
    format!("system clipboard unavailable ({err}), using in-memory 'c'")
}

#[cfg(test)]
mod tests;
