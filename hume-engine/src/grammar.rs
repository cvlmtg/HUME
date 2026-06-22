use std::path::Path;

use tree_sitter::Language;
use tree_sitter_language::LanguageFn;

// ---------------------------------------------------------------------------
// LoadedGrammar
// ---------------------------------------------------------------------------

/// A tree-sitter grammar loaded from a dynamic library at runtime.
///
/// Wraps a [`libloading::Library`] alongside the derived [`Language`]. The
/// library **must** outlive any `Language`, `Parser`, or `Tree` produced from
/// it — because `Language` is a thin pointer into the `.so`'s read-only data.
/// Field declaration order is intentional: Rust drops fields top-to-bottom, so
/// `language` is dropped (the pointer becomes dangling) before `_library` drops
/// the `.so`. That drop order is safe because `Language` is `Copy` and holds no
/// owned resources — the `.so` unload only becomes UB if the language is
/// *used* after it is freed. The `Arc<LanguageConfig>` that wraps us ensures
/// the library outlives all parsers and trees.
pub struct LoadedGrammar {
    pub(crate) language: Language,
    // Declared SECOND — drops after `language`. Keeps the .so loaded.
    _library: libloading::Library,
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
    /// # Safety
    ///
    /// This is genuine FFI — the same character as terminal probing in
    /// `editor/src/os/`. We dlopen a tree-sitter grammar built to the
    /// well-known extern "C" ABI (`tree_sitter_<lang>() -> *const TSLanguage`).
    /// The `Library` is kept alive inside this struct and drops AFTER the
    /// `Language` pointer is no longer reachable. Callers must not call
    /// tree-sitter API on a `Language` after its originating `LoadedGrammar`
    /// (or the last `Arc<LanguageConfig>` wrapping it) has been dropped.
    pub fn open(path: &Path, symbol: &str) -> Result<Self, GrammarLoadError> {
        // SAFETY: see doc comment above. The Library is stored in this struct
        // (field `_library`, second position) and is therefore dropped AFTER
        // `language`. The symbol is a well-known tree-sitter grammar entry point.
        unsafe {
            let lib = libloading::Library::new(path).map_err(GrammarLoadError::Dlopen)?;
            let sym: libloading::Symbol<unsafe extern "C" fn() -> *const ()> = lib
                .get(symbol.as_bytes())
                .map_err(GrammarLoadError::MissingSymbol)?;
            // Copy the raw fn pointer out of `sym` before `sym` (which borrows
            // `lib`) is dropped.
            let fn_ptr: unsafe extern "C" fn() -> *const () = *sym;
            let lang = Language::from(LanguageFn::from_raw(fn_ptr));
            Ok(LoadedGrammar {
                language: lang,
                _library: lib,
            })
        }
    }

    pub fn language(&self) -> &Language {
        &self.language
    }
}
