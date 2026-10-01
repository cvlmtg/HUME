/// The system clipboard behind the `c` register.
///
/// A [`Backend`] is chosen once at startup. `arboard::Clipboard` is not
/// `Send + Sync`, so it stays on the single-threaded `Editor`. When no
/// native handle exists (headless Linux, SSH without X11) or the session is
/// remote, the backend is OSC 52: writes are queued in `pending_osc52` for
/// the frame loop to send to the terminal, and reads report
/// [`ClipboardRead::Mirror`] because OSC 52 cannot be read back.
///
/// A failed native write also queues the text for OSC 52 and returns `Err`,
/// so the caller still warns while the text reaches the terminal's clipboard.
/// `read()` returns the OS clipboard's raw text. Nothing normalizes line
/// endings here; `register_ops::read_register_text` normalizes on the way
/// from this raw accessor into register values, so a Steel
/// `(read-register "c")` sees LF like every other register.
pub(crate) struct SystemClipboard {
    backend: Backend,
    pending_osc52: Option<String>,
}

enum Backend {
    System(arboard::Clipboard),
    Osc52,
    /// Every native call fails.
    #[cfg(test)]
    Unavailable,
    /// In-process virtual clipboard; `None` until first written or seeded.
    #[cfg(test)]
    Mock(Option<String>),
}

/// Outcome of [`SystemClipboard::read`].
pub(in crate::editor) enum ClipboardRead {
    /// The clipboard's current text.
    Text(String),
    /// The clipboard cannot be read; the in-memory `c` register is the source.
    Mirror,
    Failed(String),
}

impl SystemClipboard {
    pub(in crate::editor) fn new() -> Self {
        Self::select(ssh_session())
    }

    /// A remote session uses OSC 52 without touching arboard: a native
    /// clipboard on the remote host is not the user's clipboard.
    fn select(remote: bool) -> Self {
        if remote {
            return Self::osc52();
        }
        match arboard::Clipboard::new() {
            Ok(cb) => Self::with_backend(Backend::System(cb)),
            Err(_) => Self::osc52(),
        }
    }

    fn with_backend(backend: Backend) -> Self {
        Self {
            backend,
            pending_osc52: None,
        }
    }

    /// The OSC 52 backend, also the inert baseline in `EditorState::default()`
    /// so proptest never reaches the real NSPasteboard (which throws
    /// uncatchable ObjC exceptions in test threads); `Editor::open` overrides
    /// it via `new()`.
    pub(in crate::editor) fn osc52() -> Self {
        Self::with_backend(Backend::Osc52)
    }

    pub(in crate::editor) fn read(&mut self) -> ClipboardRead {
        match &mut self.backend {
            Backend::System(cb) => match cb.get_text() {
                Ok(text) => ClipboardRead::Text(text),
                Err(e) => ClipboardRead::Failed(e.to_string()),
            },
            Backend::Osc52 => ClipboardRead::Mirror,
            #[cfg(test)]
            Backend::Mock(Some(text)) => ClipboardRead::Text(text.clone()),
            #[cfg(test)]
            Backend::Unavailable | Backend::Mock(None) => {
                ClipboardRead::Failed(arboard::Error::ClipboardNotSupported.to_string())
            }
        }
    }

    pub(in crate::editor) fn write(&mut self, text: &str) -> Result<(), String> {
        let result = match &mut self.backend {
            Backend::System(cb) => cb.set_text(text).map_err(|e| e.to_string()),
            Backend::Osc52 => {
                self.pending_osc52 = Some(text.to_string());
                return Ok(());
            }
            #[cfg(test)]
            Backend::Unavailable => Err(arboard::Error::ClipboardNotSupported.to_string()),
            #[cfg(test)]
            Backend::Mock(content) => {
                *content = Some(text.to_string());
                return Ok(());
            }
        };
        if result.is_err() {
            self.pending_osc52 = Some(text.to_string());
        }
        result
    }

    /// The text queued for the terminal's OSC 52 clipboard, if any. The last
    /// write wins.
    pub(in crate::editor) fn take_pending_osc52(&mut self) -> Option<String> {
        self.pending_osc52.take()
    }

    /// A clipboard whose native calls all fail, so writes warn and queue
    /// OSC 52 and reads report failure.
    #[cfg(test)]
    pub(in crate::editor) fn new_unavailable() -> Self {
        Self::with_backend(Backend::Unavailable)
    }

    /// A clipboard backed by an in-process virtual clipboard. No real OS
    /// clipboard is touched.
    #[cfg(test)]
    pub(in crate::editor) fn new_mock() -> Self {
        Self::with_backend(Backend::Mock(None))
    }

    /// Replace the backend with `Unavailable`, dropping any mock content.
    #[cfg(test)]
    pub(in crate::editor) fn force_unavailable(&mut self) {
        self.backend = Backend::Unavailable;
    }

    /// Engage the virtual clipboard seeded with `text`.
    #[cfg(test)]
    pub(in crate::editor) fn set_mock_content(&mut self, text: &str) {
        self.backend = Backend::Mock(Some(text.to_string()));
    }
}

fn ssh_session() -> bool {
    ["SSH_TTY", "SSH_CONNECTION"]
        .iter()
        .any(|k| std::env::var_os(k).is_some())
}

#[cfg(test)]
mod tests {
    use super::{Backend, SystemClipboard};

    #[test]
    fn remote_session_selects_osc52() {
        assert!(matches!(
            SystemClipboard::select(true).backend,
            Backend::Osc52
        ));
    }
}
