//! Free functions for register and clipboard operations.
//!
//! Extracted from `impl Editor` so the same logic can be called by both the
//! `Editor` methods (thin delegators) and command bodies that hold disjoint
//! borrows from other `Editor` fields — avoiding the whole-struct `&mut self`
//! lock that forces callers to clone captured text.
//!
//! Each function that may emit a clipboard warning returns `Option<String>`
//! (the warning message). Callers report it via `ed.report(Severity::Warning, …)`.

use std::borrow::Cow;

use crate::editor::clipboard::SystemClipboard;
use crate::ops::register::{RegisterSet, CLIPBOARD_REGISTER};

/// Read text from an explicitly named register.
///
/// Returns `(values, warning)` where `warning` is `Some(msg)` when the OS
/// clipboard was unavailable and the in-memory `'c'` mirror was used instead.
///
/// - `'c'` → OS clipboard (in-memory fallback on failure).
/// - All others (`'0'`–`'9'`, etc.) → in-memory `RegisterSet`.
///
/// The kill-ring register (`'k'`) and black-hole register (`'b'`) are handled
/// upstream in `resolve_paste_values`; this function is not called for them.
pub(crate) fn read_register_text<'a>(
    registers: &'a RegisterSet,
    clipboard: &mut SystemClipboard,
    name: char,
) -> (Option<Cow<'a, [String]>>, Option<String>) {
    if name == CLIPBOARD_REGISTER {
        match clipboard.read() {
            Ok(text) => (Some(Cow::Owned(vec![text])), None),
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
        let v = registers.read(name).and_then(|r| r.as_text()).map(Cow::Borrowed);
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
        let blob = values.join("\n");
        let warning = clipboard.write(&blob).err().map(|e| clipboard_warn(&e));
        registers.write_text(CLIPBOARD_REGISTER, values);
        warning
    } else {
        registers.write_text(name, values);
        None
    }
}

/// Write `values` to the system clipboard only (no kill-ring push).
///
/// Returns `Some(warning)` if the clipboard write failed; the in-memory `'c'`
/// mirror is always updated.
pub(crate) fn write_clipboard(
    registers: &mut RegisterSet,
    clipboard: &mut SystemClipboard,
    values: &[String],
) -> Option<String> {
    let blob = values.join("\n");
    let warning = clipboard.write(&blob).err().map(|e| clipboard_warn(&e));
    registers.write_text(CLIPBOARD_REGISTER, values.to_vec());
    warning
}

fn clipboard_warn(err: &str) -> String {
    format!("system clipboard unavailable ({err}), using in-memory 'c'")
}
