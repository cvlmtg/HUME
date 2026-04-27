/// Thin wrapper around `arboard::Clipboard` for the system clipboard register.
///
/// `arboard::Clipboard` is not `Send + Sync` — must stay on the single-threaded
/// `Editor`. Initialisation failures (headless CI, SSH without X11 forwarding)
/// yield `handle = None`; subsequent calls return `Err(String)`, triggering the
/// in-memory fallback in the caller. CRLF normalisation is applied on read.
pub(crate) struct SystemClipboard {
    handle: Option<arboard::Clipboard>,
}

impl SystemClipboard {
    pub(crate) fn new() -> Self {
        Self {
            handle: arboard::Clipboard::new().ok(),
        }
    }

    pub(crate) fn read(&mut self) -> Result<String, String> {
        match self.handle.as_mut() {
            Some(cb) => cb.get_text().map(|t| t.replace("\r\n", "\n")).map_err(|e| e.to_string()),
            None => Err(arboard::Error::ClipboardNotSupported.to_string()),
        }
    }

    pub(crate) fn write(&mut self, text: &str) -> Result<(), String> {
        match self.handle.as_mut() {
            Some(cb) => cb.set_text(text).map_err(|e| e.to_string()),
            None => Err(arboard::Error::ClipboardNotSupported.to_string()),
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
