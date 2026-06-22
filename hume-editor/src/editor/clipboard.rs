/// Thin wrapper around `arboard::Clipboard` for the system clipboard register.
///
/// `arboard::Clipboard` is not `Send + Sync` — must stay on the single-threaded
/// `Editor`. Initialisation failures (headless CI, SSH without X11 forwarding)
/// yield `handle = None`; subsequent calls return `Err(String)`, triggering the
/// in-memory fallback in the caller. CRLF normalisation is applied on read.
///
/// In test builds, `mock_active` gates a virtual clipboard: when active,
/// `read()` returns `mock_content` (or `Err` if not yet written) and `write()`
/// stores the text in `mock_content` and returns `Ok`. When inactive, both
/// calls fall through to the real-handle path, which returns `Err` for a
/// dropped handle.
pub(crate) struct SystemClipboard {
    handle: Option<arboard::Clipboard>,
    /// Whether the in-process virtual clipboard is engaged. `false` for
    /// production instances and `new_unavailable()` (both hit the real handle).
    #[cfg(test)]
    mock_active: bool,
    /// Content of the virtual clipboard, set by `write()` or `set_mock_content()`.
    #[cfg(test)]
    mock_content: Option<String>,
}

impl SystemClipboard {
    pub(crate) fn new() -> Self {
        Self {
            handle: arboard::Clipboard::new().ok(),
            #[cfg(test)]
            mock_active: false,
            #[cfg(test)]
            mock_content: None,
        }
    }

    pub(crate) fn read(&mut self) -> Result<String, String> {
        #[cfg(test)]
        if self.mock_active {
            return self
                .mock_content
                .clone()
                .ok_or_else(|| arboard::Error::ClipboardNotSupported.to_string());
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
        if self.mock_active {
            self.mock_content = Some(text.to_string());
            return Ok(());
        }
        match self.handle.as_mut() {
            Some(cb) => cb.set_text(text).map_err(|e| e.to_string()),
            None => Err(arboard::Error::ClipboardNotSupported.to_string()),
        }
    }

    /// Create a clipboard instance whose handle is already dropped.
    ///
    /// All read/write calls return `Err`, hitting the in-memory fallback.
    /// The virtual mock is inactive — `force_unavailable()` on an already-inactive
    /// instance is also a no-op.
    /// Used by `Editor::for_testing` so proptest never reaches the real
    /// NSPasteboard (which throws uncatchable ObjC exceptions in test threads).
    #[cfg(test)]
    pub(crate) fn new_unavailable() -> Self {
        Self {
            handle: None,
            mock_active: false,
            mock_content: None,
        }
    }

    /// Create a clipboard instance backed by an in-process virtual clipboard.
    ///
    /// `read()` returns the last value passed to `write()` or `set_mock_content()`;
    /// `write()` stores text and returns `Ok`. No real OS clipboard is touched.
    /// Use when a test needs a functioning clipboard without a real server.
    #[cfg(test)]
    pub(crate) fn new_mock() -> Self {
        Self {
            handle: None,
            mock_active: true,
            mock_content: None,
        }
    }

    /// Disable the virtual clipboard mock and drop the real handle.
    ///
    /// All subsequent read/write calls return `Err`, triggering the in-memory
    /// fallback. Undoes a previous `set_mock_content()` / `new_mock()`.
    #[cfg(test)]
    pub(crate) fn force_unavailable(&mut self) {
        self.handle = None;
        self.mock_active = false;
        self.mock_content = None;
    }

    /// Seed the virtual clipboard with `text` and engage the mock.
    ///
    /// Subsequent `read()` calls return `text`; `write()` calls overwrite it.
    /// No real OS clipboard is touched.
    #[cfg(test)]
    pub(crate) fn set_mock_content(&mut self, text: &str) {
        self.mock_active = true;
        self.mock_content = Some(text.to_string());
    }
}
