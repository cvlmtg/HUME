/// Thin wrapper around `arboard::Clipboard` for the system clipboard register.
///
/// `arboard::Clipboard` is not `Send + Sync` — this struct must stay on the
/// single-threaded `Editor`. The handle is held for the editor's lifetime;
/// initialisation failures (headless CI, SSH without X11 forwarding) result in
/// `handle = None` and every subsequent call returns an error, triggering the
/// in-memory fallback in the caller.
pub(crate) struct SystemClipboard {
    handle: Option<arboard::Clipboard>,
}

impl SystemClipboard {
    pub(crate) fn new() -> Self {
        Self {
            handle: arboard::Clipboard::new().ok(),
        }
    }

    pub(crate) fn read(&mut self) -> Result<String, arboard::Error> {
        match self.handle.as_mut() {
            Some(cb) => cb.get_text(),
            None => Err(arboard::Error::ClipboardNotSupported),
        }
    }

    pub(crate) fn write(&mut self, text: &str) -> Result<(), arboard::Error> {
        match self.handle.as_mut() {
            Some(cb) => cb.set_text(text),
            None => Err(arboard::Error::ClipboardNotSupported),
        }
    }

    /// Drop the clipboard handle, forcing all subsequent read/write calls to fail.
    /// Used in tests to exercise the in-memory fallback path without requiring
    /// a real clipboard server.
    #[cfg(test)]
    pub(crate) fn force_unavailable(&mut self) {
        self.handle = None;
    }
}
