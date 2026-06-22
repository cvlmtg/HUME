/// Thin wrapper around `arboard::Clipboard` for the system clipboard register.
///
/// `arboard::Clipboard` is not `Send + Sync` — must stay on the single-threaded
/// `Editor`. Initialisation failures (headless CI, SSH without X11 forwarding)
/// yield `handle = None`; subsequent calls return `Err(String)`, triggering the
/// in-memory fallback in the caller. CRLF normalisation is applied on read.
///
/// In test builds, `mock_content` can be set to control what `read()` returns
/// and capture what `write()` receives — no real clipboard is touched.
pub(crate) struct SystemClipboard {
    handle: Option<arboard::Clipboard>,
    #[cfg(test)]
    mock_content: Option<String>,
}

impl SystemClipboard {
    pub(crate) fn new() -> Self {
        Self {
            handle: arboard::Clipboard::new().ok(),
            #[cfg(test)]
            mock_content: None,
        }
    }

    pub(crate) fn read(&mut self) -> Result<String, String> {
        #[cfg(test)]
        if let Some(content) = &self.mock_content {
            return Ok(content.clone());
        }
        match self.handle.as_mut() {
            Some(cb) => cb
                .get_text()
                .map(|t| t.replace("\r\n", "\n"))
                .map_err(|e| e.to_string()),
            None => Err(arboard::Error::ClipboardNotSupported.to_string()),
        }
    }

    pub(crate) fn write(&mut self, text: &str) -> Result<(), String> {
        #[cfg(test)]
        {
            self.mock_content = Some(text.to_string());
        }
        match self.handle.as_mut() {
            Some(cb) => cb.set_text(text).map_err(|e| e.to_string()),
            None => Err(arboard::Error::ClipboardNotSupported.to_string()),
        }
    }

    /// Create a clipboard instance whose handle is already dropped.
    ///
    /// All real read/write calls return `Err`, hitting the in-memory fallback.
    /// Used by `Editor::for_testing` so proptest never reaches the real
    /// NSPasteboard (which throws uncatchable ObjC exceptions in test threads).
    #[cfg(test)]
    pub(crate) fn new_unavailable() -> Self {
        Self {
            handle: None,
            mock_content: None,
        }
    }

    /// Drop the clipboard handle, forcing all subsequent real read/write calls
    /// to fail.  Used in tests to exercise the in-memory fallback path without
    /// requiring a real clipboard server.
    #[cfg(test)]
    pub(crate) fn force_unavailable(&mut self) {
        self.handle = None;
        self.mock_content = None;
    }

    /// Set the mock clipboard content.
    ///
    /// Subsequent `read()` calls return this text and `write()` calls overwrite
    /// it.  No real OS clipboard is touched.
    #[cfg(test)]
    pub(crate) fn set_mock_content(&mut self, text: &str) {
        self.mock_content = Some(text.to_string());
    }
}
