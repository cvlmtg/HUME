//! Register content reads/writes.

/// Register content reads/writes — accessed through [`EditorHost::registers`](super::EditorHost::registers).
///
/// A register holds one string per selection captured at yank time. Macro
/// registers (recorded key sequences, not text) are out of scope: there is no
/// wire format yet for handing a `Vec<KeyEvent>` to Scheme, so [`Self::read_register`]
/// answers `None` for one, exactly as it would for an empty register.
pub trait RegisterHost {
    /// Contents of `name` as one string per selection, or `None` when the
    /// register is empty, is the black hole, or holds a macro.
    ///
    /// `&mut self` because the clipboard register (`'c'`) may need to read
    /// the live OS clipboard.
    fn read_register(&mut self, name: char) -> Option<Vec<String>>;

    /// Store `values` in register `name`. Routes exactly like a keyboard
    /// write: `'k'` pushes onto the kill ring (stamped for smart-paste, same
    /// as `"ky`), `'c'` mirrors to the OS clipboard, `'b'` discards silently.
    fn write_register(&mut self, name: char, values: Vec<String>);
}
