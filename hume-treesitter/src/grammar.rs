use std::path::Path;

use tree_sitter::Language;
use tree_sitter_language::LanguageFn;

// ---------------------------------------------------------------------------
// LoadedGrammar
// ---------------------------------------------------------------------------

/// A tree-sitter grammar loaded from a dynamic library at runtime.
///
/// `language` is a thin pointer into the `.so`'s read-only data. The
/// dynamic library backing it is intentionally leaked at load time (see
/// `open`) so it stays mapped for the process lifetime — grammars are never
/// unloaded, so `Language` values derived from this struct are valid
/// unconditionally, with no lifetime tied to `LoadedGrammar` or any wrapper.
pub struct LoadedGrammar {
    language: Language,
}

#[derive(Debug)]
pub enum GrammarLoadError {
    Dlopen(libloading::Error),
    MissingSymbol(libloading::Error),
}

impl std::fmt::Display for GrammarLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dlopen(e) => write!(f, "failed to open grammar library: {e}"),
            Self::MissingSymbol(e) => write!(f, "grammar symbol not found: {e}"),
        }
    }
}

impl LoadedGrammar {
    /// Load a tree-sitter grammar from `path`.
    ///
    /// `symbol` is the exported C function name (e.g. `"tree_sitter_rust"`).
    /// The function must have the ABI `unsafe extern "C" fn() -> *const ()`,
    /// which is the standard signature produced by `tree-sitter generate`.
    ///
    /// # Errors
    ///
    /// Returns [`GrammarLoadError::Dlopen`] if the library cannot be opened,
    /// or [`GrammarLoadError::MissingSymbol`] if the named symbol is absent.
    ///
    /// This is genuine FFI — the same character as terminal probing in
    /// `editor/src/os/`. We dlopen a tree-sitter grammar built to the
    /// well-known extern "C" ABI (`tree_sitter_<lang>() -> *const TSLanguage`).
    /// The library is leaked (`mem::forget`) rather than owned, so it stays
    /// mapped for the process lifetime and the derived `Language` pointer
    /// never dangles.
    pub fn open(path: &Path, symbol: &str) -> Result<Self, GrammarLoadError> {
        // SAFETY: see doc comment above. The symbol is a well-known
        // tree-sitter grammar entry point.
        unsafe {
            let lib = libloading::Library::new(path).map_err(GrammarLoadError::Dlopen)?;
            let sym: libloading::Symbol<unsafe extern "C" fn() -> *const ()> = lib
                .get(symbol.as_bytes())
                .map_err(GrammarLoadError::MissingSymbol)?;
            // Copy the raw fn pointer out of `sym` before `sym` (which borrows
            // `lib`) is dropped.
            let fn_ptr: unsafe extern "C" fn() -> *const () = *sym;
            let lang = Language::from(LanguageFn::from_raw(fn_ptr));
            // Grammars are never unloaded (Helix model) — leak the library so
            // the .so stays mapped for the process lifetime instead of tying
            // its lifetime to this struct or its wrappers.
            std::mem::forget(lib);
            Ok(LoadedGrammar { language: lang })
        }
    }

    pub fn language(&self) -> &Language {
        &self.language
    }
}
