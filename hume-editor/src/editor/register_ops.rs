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
use crate::ops::register::{CLIPBOARD_REGISTER, RegisterSet, is_linewise};

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
        // not "line\n\nword" which the old all-or-nothing join produced).
        let mut blob = String::new();
        for (i, v) in values.iter().enumerate() {
            if i > 0 && !is_linewise(&values[i - 1]) {
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
mod tests {
    use super::read_register_text;
    use crate::editor::clipboard::SystemClipboard;
    use crate::ops::register::{CLIPBOARD_REGISTER, RegisterSet};

    fn seeded_registers(values: &[&str]) -> RegisterSet {
        let mut regs = RegisterSet::new();
        regs.write_text(
            CLIPBOARD_REGISTER,
            values.iter().map(|s| s.to_string()).collect(),
        );
        regs
    }

    fn mock_clipboard(content: &str) -> SystemClipboard {
        let mut cb = SystemClipboard::new_unavailable();
        cb.set_mock_content(content);
        cb
    }

    #[test]
    fn clipboard_in_sync_prefers_structured() {
        let mut regs = seeded_registers(&["a", "b", "c"]);
        regs.set_clipboard_blob("a\nb\nc".to_string());
        let mut cb = mock_clipboard("a\nb\nc");

        let (values, _warn) = read_register_text(&regs, &mut cb, CLIPBOARD_REGISTER);
        let values = values.unwrap();
        assert_eq!(values.len(), 3);
        assert_eq!(values[0], "a");
        assert_eq!(values[1], "b");
        assert_eq!(values[2], "c");
    }

    #[test]
    fn clipboard_externally_modified_uses_clipboard() {
        let mut regs = seeded_registers(&["a", "b", "c"]);
        regs.set_clipboard_blob("a\nb\nc".to_string());
        let mut cb = mock_clipboard("xyz");

        let (values, _warn) = read_register_text(&regs, &mut cb, CLIPBOARD_REGISTER);
        let values = values.unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0], "xyz");
    }

    #[test]
    fn clipboard_no_blob_fresh_session_uses_clipboard() {
        let regs = RegisterSet::new();
        let mut cb = mock_clipboard("xyz");

        let (values, _warn) = read_register_text(&regs, &mut cb, CLIPBOARD_REGISTER);
        let values = values.unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0], "xyz");
    }

    #[test]
    fn clipboard_in_sync_in_memory_missing_falls_through() {
        let mut regs = RegisterSet::new();
        regs.set_clipboard_blob("xyz".to_string());
        let mut cb = mock_clipboard("xyz");

        let (values, _warn) = read_register_text(&regs, &mut cb, CLIPBOARD_REGISTER);
        let values = values.unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0], "xyz");
    }

    // ── write_register blob computation ──────────────────────────────────────

    fn write_cb<'a>(
        regs: &'a mut RegisterSet,
        cb: &mut SystemClipboard,
        values: Vec<String>,
    ) -> &'a RegisterSet {
        super::write_register(regs, cb, CLIPBOARD_REGISTER, values);
        regs
    }

    #[test]
    fn write_register_linewise_concats_without_extra_newline() {
        // Two linewise values (each ends with '\n'): blob must be their concat,
        // NOT join("\n") which inserts a spurious blank line between them.
        // Expected value is hand-computed — independent of the impl's concat/join.
        let mut regs = RegisterSet::new();
        let mut cb = SystemClipboard::new_unavailable();
        write_cb(&mut regs, &mut cb, vec!["foo\n".into(), "bar\n".into()]);
        assert_eq!(regs.clipboard_blob(), Some("foo\nbar\n"));
    }

    #[test]
    fn write_register_charwise_joins_with_newline() {
        // Charwise values (no trailing '\n'): blob is join("\n").
        let mut regs = RegisterSet::new();
        let mut cb = SystemClipboard::new_unavailable();
        write_cb(&mut regs, &mut cb, vec!["a".into(), "b".into(), "c".into()]);
        assert_eq!(regs.clipboard_blob(), Some("a\nb\nc"));
    }

    #[test]
    fn write_register_read_round_trip_preserves_structure() {
        // End-to-end: write 3 charwise values, then read back — must return
        // the 3-element Vec, not a flat 1-element blob.
        // Uses the real write_register→blob-stash→read_register_text→blob-compare
        // path; no hand-seeding of clipboard_blob.
        // new_mock(): write() must succeed (Ok) so no warning is emitted and
        // read() returns the stored blob — new_unavailable() would make write()
        // return Err and read() fall through to the register fallback instead.
        let mut regs = RegisterSet::new();
        let mut cb = SystemClipboard::new_mock();
        write_cb(&mut regs, &mut cb, vec!["a".into(), "b".into(), "c".into()]);

        let (values, warn) = read_register_text(&regs, &mut cb, CLIPBOARD_REGISTER);
        assert!(warn.is_none());
        let values = values.unwrap();
        assert_eq!(
            values.len(),
            3,
            "round-trip must return 3 structured values"
        );
        assert_eq!(&values[..], &["a", "b", "c"]);
    }

    #[test]
    fn write_register_mixed_linewise_then_charwise_no_double_newline() {
        // Linewise first, charwise second: the separator must NOT be added
        // because the linewise value already ends in '\n'. Expected blob is
        // hand-computed: "foo\n" + "bar" = "foo\nbar" (not "foo\n\nbar").
        let mut regs = RegisterSet::new();
        let mut cb = SystemClipboard::new_unavailable();
        write_cb(&mut regs, &mut cb, vec!["foo\n".into(), "bar".into()]);
        assert_eq!(regs.clipboard_blob(), Some("foo\nbar"));
    }

    #[test]
    fn write_register_mixed_charwise_then_linewise_inserts_one_newline() {
        // Charwise first, linewise second: separator IS added because the
        // charwise value does not end in '\n'. Expected blob is hand-computed:
        // "bar" + '\n' + "foo\n" = "bar\nfoo\n" (not "bar\nfoo\n" via join either,
        // but the distinction matters for the linewise→charwise order above).
        let mut regs = RegisterSet::new();
        let mut cb = SystemClipboard::new_unavailable();
        write_cb(&mut regs, &mut cb, vec!["bar".into(), "foo\n".into()]);
        assert_eq!(regs.clipboard_blob(), Some("bar\nfoo\n"));
    }
}
