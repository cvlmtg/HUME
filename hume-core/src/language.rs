//! A language's identity and configuration as scripts register them: the
//! detection data, the per-language fields, and where its grammar is. Also
//! how two writes of a buffer's language rank.

use std::path::{Path, PathBuf};

use globset::Glob;

use crate::{BlockCommentTokens, LineCommentTokens, Tie};

/// A buffer's language: a later write from one origin replaces the earlier,
/// whether the editor applies it or a script reads it back before then.
pub const BUFFER_LANGUAGE_TIE: Tie = Tie::LaterWins;

/// A glob of a language's detection data, matched against a file's whole
/// absolute path as Helix matches the `languages.toml` data it
/// comes from: a glob that starts with neither `/` nor `*/` gets `*/` in
/// front, so it matches the path's last components (`.git/config`,
/// `Makefile`), and `*` crosses `/`.
#[derive(Debug, Clone)]
pub struct FileGlob(Glob);

impl FileGlob {
    pub fn new(pattern: &str) -> Result<Self, globset::Error> {
        let glob = if pattern.starts_with('/') || pattern.starts_with("*/") {
            Glob::new(pattern)
        } else {
            Glob::new(&format!("*/{pattern}"))
        };
        glob.map(Self)
    }

    pub fn glob(&self) -> &Glob {
        &self.0
    }
}

/// A file extension of a language's detection data: one or more
/// dot-separated names (`rs`, `js.map`), each non-empty and free of path
/// separators. It matches a file name that ends with a dot and the
/// extension, the dot not being the name's first character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileExtension(String);

impl FileExtension {
    pub fn new(extension: String) -> Result<Self, InvalidExtension> {
        let names_a_file = extension
            .split('.')
            .all(|name| !name.is_empty() && !name.contains(['/', '\\']));
        if names_a_file {
            Ok(Self(extension))
        } else {
            Err(InvalidExtension(extension))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An extension [`FileExtension::new`] refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidExtension(String);

impl std::fmt::Display for InvalidExtension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid extension {:?}: expected dot-separated names, such as \"rs\" or \"js.map\", \
             with no leading or trailing dot and no path separator",
            self.0
        )
    }
}

impl std::error::Error for InvalidExtension {}

/// An interpreter name of a language's detection data (`python`, `node`),
/// matched against the program a file's `#!` line runs. Non-empty, with no
/// whitespace and no path separator, since a line's interpreter is one
/// path's last component.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ShebangInterpreter(String);

impl ShebangInterpreter {
    pub fn new(name: String) -> Result<Self, InvalidShebangInterpreter> {
        let names_a_program = !name.is_empty()
            && !name.contains(|c: char| c.is_whitespace() || c == '/' || c == '\\');
        if names_a_program {
            Ok(Self(name))
        } else {
            Err(InvalidShebangInterpreter(name))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The language registry looks an interpreter up by a `&str` read from a
/// file's first line.
impl std::borrow::Borrow<str> for ShebangInterpreter {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// An interpreter name [`ShebangInterpreter::new`] refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidShebangInterpreter(String);

impl std::fmt::Display for InvalidShebangInterpreter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid shebang {:?}: expected an interpreter name, such as \"python\" or \"node\", \
             with no space and no path separator",
            self.0
        )
    }
}

impl std::error::Error for InvalidShebangInterpreter {}

/// Identity of one language: its detection data (extensions, glob patterns,
/// shebangs).
///
/// Immutable once registered: the language registry replaces the whole
/// record on re-registration rather than mutating it in place. The name is
/// not stored here: it lives once, in the registry's interner.
#[derive(Debug, Default)]
pub struct LanguageIdentity {
    pub extensions: Vec<FileExtension>,
    /// The language registry compiles every language's globs into one
    /// matcher.
    pub globs: Vec<FileGlob>,
    /// The interpreters whose `#!` lines name this language.
    pub shebangs: Vec<ShebangInterpreter>,
}

/// The per-language values the registry stores, as one `configure-language!`
/// call or one `<runtime>/scheme/languages/<name>.scm` file gives them;
/// `None` where it gives none.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LanguageFields {
    /// Workspace root markers.
    pub roots: Option<Vec<String>>,
    /// The wire `languageId`, when it differs from the name.
    pub lsp_language_id: Option<String>,
    pub comment_tokens: Option<LineCommentTokens>,
    pub block_comment_tokens: Option<BlockCommentTokens>,
}

named_enum! {
    /// One of [`LanguageFields`]' fields, named by the key
    /// `configure-language!` and a bundled data file spell it as.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum LanguageField {
        Roots => "roots",
        LanguageId => "language-id",
        CommentTokens => "comment-tokens",
        BlockCommentTokens => "block-comment-tokens",
    }
}

named_enum! {
    /// A query file a grammar may ship beyond its required highlights, named
    /// as scripting names it.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum OptionalQuery {
        Injections => "injections",
        TextObjects => "textobjects",
        Indents => "indents",
    }
}

impl OptionalQuery {
    /// The query file's name inside a grammar's query directory.
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Injections => "injections.scm",
            Self::TextObjects => "textobjects.scm",
            Self::Indents => "indents.scm",
        }
    }
}

/// The query files a grammar's load compiles. `highlights`
/// is required; each `OptionalQuery` kind is independently optional.
#[derive(Debug, Clone)]
pub struct QueryFiles {
    highlights: PathBuf,
    injections: Option<PathBuf>,
    textobjects: Option<PathBuf>,
    indents: Option<PathBuf>,
}

impl QueryFiles {
    /// `highlights` alone, with no optional query.
    pub fn new(highlights: PathBuf) -> Self {
        Self {
            highlights,
            injections: None,
            textobjects: None,
            indents: None,
        }
    }

    /// These files plus `path` as the `kind` query.
    pub fn with(mut self, kind: OptionalQuery, path: PathBuf) -> Self {
        match kind {
            OptionalQuery::Injections => self.injections = Some(path),
            OptionalQuery::TextObjects => self.textobjects = Some(path),
            OptionalQuery::Indents => self.indents = Some(path),
        }
        self
    }

    pub fn highlights(&self) -> &Path {
        &self.highlights
    }

    pub fn get(&self, kind: OptionalQuery) -> Option<&Path> {
        match kind {
            OptionalQuery::Injections => self.injections.as_deref(),
            OptionalQuery::TextObjects => self.textobjects.as_deref(),
            OptionalQuery::Indents => self.indents.as_deref(),
        }
    }
}

/// Where a grammar's library and query files are. Registering one opens
/// nothing; `hume_treesitter`'s `LanguageRegistry::load_grammar` reads and
/// compiles them.
#[derive(Debug, Clone)]
pub struct GrammarSource {
    pub path: PathBuf,
    pub symbol: String,
    pub queries: QueryFiles,
}

#[cfg(any(test, feature = "test-util"))]
impl GrammarSource {
    /// A source for `name` whose library and queries do not exist, so its
    /// load fails.
    pub fn missing(name: &str) -> Self {
        let dir = Path::new("/nonexistent-hume-grammar");
        Self {
            path: dir.join(format!("{name}.so")),
            symbol: format!("tree_sitter_{name}"),
            queries: QueryFiles::new(dir.join("highlights.scm")),
        }
    }
}
