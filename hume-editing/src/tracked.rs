//! A value computed against one text, readable only against that text.

use crate::edit::TextChange;
use crate::text::{BufferText, TextVersion};

/// `value` together with the version of the text it was computed against.
/// Reading it against a text of another version gives `None`, so a stored
/// position cannot be used after its text changed unless it was carried
/// through the change.
#[derive(Debug, Clone)]
pub struct Tracked<T> {
    value: T,
    version: TextVersion,
}

impl<T> Tracked<T> {
    pub fn new(value: T, text: &BufferText) -> Self {
        Self {
            value,
            version: text.version(),
        }
    }

    /// The value, when `text` is the one it was computed against.
    pub fn get(&self, text: &BufferText) -> Option<&T> {
        (self.version == text.version()).then_some(&self.value)
    }

    pub fn get_mut(&mut self, text: &BufferText) -> Option<&mut T> {
        (self.version == text.version()).then_some(&mut self.value)
    }

    pub fn into_inner(self, text: &BufferText) -> Option<T> {
        (self.version == text.version()).then_some(self.value)
    }

    /// Carry the value through `change` with `f`, when it was computed
    /// against the change's old text; otherwise leave it stale.
    pub fn translate(&mut self, change: &TextChange<'_>, f: impl FnOnce(&mut T)) {
        if self.version == change.before().version() {
            f(&mut self.value);
            self.version = change.after().version();
        }
    }
}

#[cfg(test)]
mod tests;
