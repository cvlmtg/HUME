use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::grammar::LoadedGrammar;
use crate::highlight::TreeSitterHighlighter;
use hume_editing::comment::{BlockCommentTokens, CommentTokens, LineCommentTokens};
use hume_engine::theme::ScopeRegistry;

use crate::indent::IndentQuery;
use crate::injections::InjectionsQuery;
use crate::textobjects::TextObjectsQuery;

/// Interned language identity. Dense, append-only, minted by `LanguageRegistry`.
/// An id is only ever handed out by `LanguageRegistry::intern`; indexing a
/// different registry instance with it is a caller bug, not a runtime error.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LanguageId(u32);

impl LanguageId {
    /// The id of the language at `index` in `LanguageRegistry::entries`.
    fn from_index(index: usize) -> Self {
        Self(index as u32)
    }
}

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
}

/// A file extension of a language's detection data: one or more
/// dot-separated names (`rs`, `js.map`), each non-empty and free of path
/// separators. It matches a file name that ends with a dot and the
/// extension, the dot not being the name's first character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileExtension(String);

impl FileExtension {
    pub fn new(extension: &str) -> Result<Self, InvalidExtension> {
        let names_a_file = extension
            .split('.')
            .all(|name| !name.is_empty() && !name.contains(['/', '\\']));
        if names_a_file {
            Ok(Self(extension.to_owned()))
        } else {
            Err(InvalidExtension(extension.to_owned()))
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

/// Identity of one language: its detection data (extensions, glob patterns,
/// shebangs).
///
/// Immutable once registered: re-registration (`register_identity_no_rebuild`)
/// replaces the whole record rather than mutating it in place. The name is
/// not stored here: it lives once, in the registry's interner (SSOT).
#[derive(Debug, Default)]
pub struct LanguageIdentity {
    pub extensions: Vec<FileExtension>,
    /// The combined matcher across all languages lives on
    /// `LanguageRegistry` (`compiled_globs`).
    pub globs: Vec<FileGlob>,
    /// Shebang substrings to match (e.g. `"python"`, `"node"`).
    pub shebangs: Vec<String>,
}

/// Who configured a per-language value. A user value (`init.scm`) beats a
/// plugin's, whichever arrives first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfiguredBy {
    Plugin,
    User,
}

/// Where a per-language value came from. A configured value always beats a
/// bundled one, whichever arrives first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingSource {
    Configured(ConfiguredBy),
    Bundled,
}

#[derive(Debug)]
struct Setting<T> {
    value: T,
    source: SettingSource,
}

/// Per-language values outside the identity: set by configuration, or filled
/// from the language's bundled data file the first time a buffer uses it.
/// Re-registering the identity leaves them in place.
#[derive(Debug, Default)]
struct LanguageConfig {
    /// Override for the wire `languageId` sent to language servers, when it
    /// differs from the name (e.g. `"tsx"` needs `"typescriptreact"`).
    lsp_language_id: Option<Setting<String>>,
    /// File or directory names that make a directory this language's
    /// workspace root (e.g. `"Cargo.toml"`, `".git"`).
    roots: Option<Setting<Vec<String>>>,
    line_comment_tokens: Option<Setting<LineCommentTokens>>,
    block_comment_tokens: Option<Setting<BlockCommentTokens>>,
    /// Whether the bundled data file has been read, found or not.
    bundled_loaded: bool,
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

/// One of [`LanguageFields`]' fields, named by its configuration key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LanguageField {
    Roots,
    LanguageId,
    CommentTokens,
    BlockCommentTokens,
}

impl LanguageField {
    pub const ALL: [Self; 4] = [
        Self::Roots,
        Self::LanguageId,
        Self::CommentTokens,
        Self::BlockCommentTokens,
    ];

    /// The key `configure-language!` and a bundled data file spell it as.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Roots => "roots",
            Self::LanguageId => "language-id",
            Self::CommentTokens => "comment-tokens",
            Self::BlockCommentTokens => "block-comment-tokens",
        }
    }
}

/// Result of a configured write.
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigureOutcome {
    Applied,
    /// The field already holds a value from the same configurer; the first
    /// one stays.
    Ignored,
    /// The field already holds a user value and this write came from a plugin.
    Shadowed,
}

/// Store `value` from `by` unless the slot holds a value configured by the
/// same or a higher tier.
fn configure<T>(slot: &mut Option<Setting<T>>, value: T, by: ConfiguredBy) -> ConfigureOutcome {
    if let Some(Setting {
        source: SettingSource::Configured(held),
        ..
    }) = slot
    {
        match (*held).cmp(&by) {
            std::cmp::Ordering::Equal => return ConfigureOutcome::Ignored,
            std::cmp::Ordering::Greater => return ConfigureOutcome::Shadowed,
            std::cmp::Ordering::Less => {}
        }
    }
    *slot = Some(Setting {
        value,
        source: SettingSource::Configured(by),
    });
    ConfigureOutcome::Applied
}

/// Store a bundled `value` only into an empty slot.
fn fill<T>(slot: &mut Option<Setting<T>>, value: T) {
    if slot.is_none() {
        *slot = Some(Setting {
            value,
            source: SettingSource::Bundled,
        });
    }
}

/// Tree-sitter grammar + precompiled highlight query, shared across all buffers
/// of a given language.
pub struct GrammarBundle {
    pub grammar: LoadedGrammar,
    /// Shared highlighter wrapping the compiled highlight query: one per
    /// language, not one per buffer (capture names are interned once, when
    /// the grammar loads).
    pub highlighter: Arc<TreeSitterHighlighter>,
    /// Compiled `injections.scm`, if the grammar has one. `None` means this
    /// language never injects embedded languages.
    pub injections: Option<InjectionsQuery>,
    /// Compiled `textobjects.scm`, if the grammar has one. `None` means this
    /// language ships no structural text objects or navigation.
    pub textobjects: Option<TextObjectsQuery>,
    /// Compiled `indents.scm`, if the grammar has one. `None` means new
    /// lines in this language are never indented from its syntax tree.
    pub indents: Option<IndentQuery>,
    /// Unique per bundle, issued by `next_config_gen`. Grammar-swap
    /// / staleness checks compare this instead of `Arc::ptr_eq`, a plain
    /// integer identity that survives across the worker-thread boundary.
    pub config_gen: u32,
    /// The language this grammar was loaded for.
    pub language: LanguageId,
}

/// A query file a grammar may ship beyond its required highlights.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionalQuery {
    Injections,
    TextObjects,
    Indents,
}

impl OptionalQuery {
    pub const ALL: [Self; 3] = [Self::Injections, Self::TextObjects, Self::Indents];

    /// The name scripting uses for this kind.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Injections => "injections",
            Self::TextObjects => "textobjects",
            Self::Indents => "indents",
        }
    }

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
/// nothing; [`LanguageRegistry::load_grammar`] reads and compiles them.
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

/// A language's grammar, from registration through its first load.
enum GrammarSlot {
    Registered(GrammarSource),
    Loaded(Arc<GrammarBundle>),
}

/// A fresh `GrammarBundle::config_gen`. Process-wide, so bundles of two
/// registries never share one: a config reload replaces the registry while
/// the parse worker may still hold a request built against the outgoing one.
pub(crate) fn next_config_gen() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A language's grammar as an injection resolves it.
pub(crate) enum SnapshotGrammar {
    Loaded(Arc<GrammarBundle>),
    /// Registered but not loaded: only the main thread can load it, so a
    /// parse reports the id for it to load.
    Unloaded(LanguageId),
}

/// Every language whose grammar is loaded or yet to load, by name, as of one
/// registry state. Handed to the parse worker, which resolves an injected
/// language name (a dynamically discovered info string) against it without
/// touching main-thread state. A grammar whose load failed is absent.
#[derive(Default)]
pub struct GrammarSnapshot {
    /// Process-wide and increasing: a later registry state, even of another
    /// registry, has a higher one. The empty default has 0, below all of them.
    generation: u64,
    grammars: FxHashMap<Arc<str>, SnapshotGrammar>,
}

impl GrammarSnapshot {
    fn new(grammars: FxHashMap<Arc<str>, SnapshotGrammar>) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            generation: NEXT.fetch_add(1, Ordering::Relaxed),
            grammars,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn get(&self, name: &str) -> Option<&SnapshotGrammar> {
        self.grammars.get(name)
    }

    /// This snapshot plus `name` as registered but not loaded.
    #[cfg(any(test, feature = "test-util"))]
    pub fn with_unloaded(self, name: &str, id: LanguageId) -> Self {
        let mut grammars = self.grammars;
        grammars.insert(Arc::from(name), SnapshotGrammar::Unloaded(id));
        Self::new(grammars)
    }
}

/// A snapshot of loaded grammars, for tests that build bundles by hand.
#[cfg(any(test, feature = "test-util"))]
impl FromIterator<(String, Arc<GrammarBundle>)> for GrammarSnapshot {
    fn from_iter<I: IntoIterator<Item = (String, Arc<GrammarBundle>)>>(iter: I) -> Self {
        Self::new(
            iter.into_iter()
                .map(|(name, bundle)| (Arc::from(name), SnapshotGrammar::Loaded(bundle)))
                .collect(),
        )
    }
}

/// Opens `source`'s library, checks its ABI, and reads and compiles its
/// queries. `scope_reg` is touched only once everything else succeeded.
fn compile_source(
    name: &str,
    source: &GrammarSource,
    scope_reg: &mut ScopeRegistry,
) -> Result<(LoadedGrammar, CompiledQueries), RegisterError> {
    let grammar =
        LoadedGrammar::open(&source.path, &source.symbol).map_err(RegisterError::GrammarLoad)?;
    let abi = grammar.language().abi_version();
    let supported = tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION;
    if !supported.contains(&abi) {
        return Err(RegisterError::AbiIncompatible {
            name: name.to_owned(),
            abi,
            supported,
        });
    }
    let queries = &source.queries;
    let highlights_src =
        std::fs::read_to_string(queries.highlights()).map_err(RegisterError::HighlightsRead)?;
    let mut optional_src = Vec::new();
    for kind in OptionalQuery::ALL {
        if let Some(path) = queries.get(kind) {
            let src = std::fs::read_to_string(path)
                .map_err(|error| RegisterError::OptionalQueryRead { kind, error })?;
            optional_src.push((kind, src));
        }
    }
    let optional: Vec<(OptionalQuery, &str)> = optional_src
        .iter()
        .map(|(kind, src)| (*kind, src.as_str()))
        .collect();
    let compiled = compile_queries(grammar.language(), &highlights_src, &optional, scope_reg)?;
    Ok((grammar, compiled))
}

fn new_bundle(
    language: LanguageId,
    grammar: LoadedGrammar,
    compiled: CompiledQueries,
) -> Arc<GrammarBundle> {
    let CompiledQueries {
        highlighter,
        injections,
        textobjects,
        indents,
    } = compiled;
    Arc::new(GrammarBundle {
        grammar,
        highlighter,
        injections,
        textobjects,
        indents,
        config_gen: next_config_gen(),
        language,
    })
}

/// The compiled queries of one grammar, ready to become a `GrammarBundle`.
pub(crate) struct CompiledQueries {
    pub(crate) highlighter: Arc<TreeSitterHighlighter>,
    pub(crate) injections: Option<InjectionsQuery>,
    pub(crate) textobjects: Option<TextObjectsQuery>,
    pub(crate) indents: Option<IndentQuery>,
}

/// Compiles a grammar's queries. This is the one place query sources become
/// compiled queries. Every compile happens before `scope_reg` is touched, so
/// a failure leaves it unchanged.
pub(crate) fn compile_queries(
    language: &tree_sitter::Language,
    highlights: &str,
    optional: &[(OptionalQuery, &str)],
    scope_reg: &mut ScopeRegistry,
) -> Result<CompiledQueries, RegisterError> {
    let highlights =
        Arc::new(tree_sitter::Query::new(language, highlights).map_err(RegisterError::QueryBuild)?);
    let mut injections = None;
    let mut textobjects = None;
    let mut indents = None;
    for &(kind, src) in optional {
        let query = tree_sitter::Query::new(language, src)
            .map_err(|error| RegisterError::OptionalQueryBuild { kind, error })?;
        let duplicate = match kind {
            OptionalQuery::Injections => injections
                .replace(InjectionsQuery::new(Arc::new(query)))
                .is_some(),
            OptionalQuery::TextObjects => {
                textobjects.replace(TextObjectsQuery::new(query)).is_some()
            }
            OptionalQuery::Indents => indents
                .replace(
                    IndentQuery::new(query, src, language)
                        .map_err(|error| RegisterError::OptionalQueryBuild { kind, error })?,
                )
                .is_some(),
        };
        assert!(!duplicate, "{} given twice", kind.file_name());
    }
    Ok(CompiledQueries {
        highlighter: Arc::new(TreeSitterHighlighter::from_shared_query(
            highlights, scope_reg,
        )),
        injections,
        textobjects,
        indents,
    })
}

/// Everything the registry holds for one interned language.
struct LanguageEntry {
    name: Arc<str>,
    /// `None` = interned but no identity registered.
    identity: Option<LanguageIdentity>,
    config: LanguageConfig,
    grammar: Option<GrammarSlot>,
}

/// Global registry of configured language identities. Lives on `Editor`.
///
/// `ids` and `entries` are the interner: dense and append-only, with
/// `entries[id.0]` holding the language `id` names.
pub struct LanguageRegistry {
    ids: FxHashMap<Arc<str>, LanguageId>,
    entries: Vec<LanguageEntry>,
    /// Detection indices, never rebuilt by a grammar change, only on
    /// identity (re-)registration or removal.
    by_ext: FxHashMap<String, LanguageId>,
    /// Compiled glob matcher, rebuilt whenever languages are added or removed.
    /// Index-aligned with `glob_lang_ids`.
    compiled_globs: GlobSet,
    /// Language id for each glob pattern at the corresponding GlobSet index.
    glob_lang_ids: Vec<LanguageId>,
    shebang_to_id: FxHashMap<String, LanguageId>,
    /// Registration order for glob priority: later entries win on overlap.
    lang_order: Vec<LanguageId>,
    /// Rebuilt whenever a grammar changes.
    grammar_snapshot: Arc<GrammarSnapshot>,
    /// Languages given a grammar whose bundled data file has not been read.
    /// A syntax layer exists only for a language with a grammar, so reading
    /// these before any parse means every layer's language has its data.
    bundled_pending: Vec<LanguageId>,
    /// Grammars whose load failed since the last take, each with its error.
    /// A failed load clears the grammar, so each failure is queued once.
    failed_loads: Vec<(LanguageId, RegisterError)>,
}

#[derive(Debug)]
pub enum RegisterError {
    /// The combined glob pattern set exceeded globset's NFA size limit.
    GlobBuild(globset::Error),
    /// Failed to open the grammar shared library.
    GrammarLoad(crate::grammar::GrammarLoadError),
    /// Failed to read the highlights query file.
    HighlightsRead(std::io::Error),
    /// Failed to compile the highlights query.
    QueryBuild(tree_sitter::QueryError),
    /// Failed to read an optional query file.
    OptionalQueryRead {
        kind: OptionalQuery,
        error: std::io::Error,
    },
    /// Failed to compile an optional query.
    OptionalQueryBuild {
        kind: OptionalQuery,
        error: tree_sitter::QueryError,
    },
    /// Grammar ABI version is outside the range the bundled tree-sitter
    /// library supports.  Recompile the grammar with a compatible generator.
    AbiIncompatible {
        name: String,
        abi: usize,
        supported: std::ops::RangeInclusive<usize>,
    },
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GlobBuild(e) => write!(f, "glob set compilation failed: {e}"),
            Self::GrammarLoad(e) => write!(f, "grammar load failed: {e}"),
            Self::HighlightsRead(e) => write!(f, "highlights.scm read failed: {e}"),
            Self::QueryBuild(e) => write!(f, "highlight query compilation failed: {e}"),
            Self::OptionalQueryRead { kind, error } => {
                write!(f, "{} read failed: {error}", kind.file_name())
            }
            Self::OptionalQueryBuild { kind, error } => {
                write!(f, "{} compilation failed: {error}", kind.file_name())
            }
            Self::AbiIncompatible {
                name,
                abi,
                supported,
            } => write!(
                f,
                "grammar '{name}' ABI {abi} not in supported range {supported:?}"
            ),
        }
    }
}

impl Default for LanguageRegistry {
    fn default() -> Self {
        Self {
            ids: FxHashMap::default(),
            entries: Vec::new(),
            by_ext: FxHashMap::default(),
            compiled_globs: GlobSet::empty(),
            glob_lang_ids: Vec::new(),
            shebang_to_id: FxHashMap::default(),
            lang_order: Vec::new(),
            grammar_snapshot: Arc::new(GrammarSnapshot::default()),
            bundled_pending: Vec::new(),
            failed_loads: Vec::new(),
        }
    }
}

impl LanguageRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Interner ──────────────────────────────────────────────────────────────

    /// Mint-or-get: returns the existing id for `name`, or interns a fresh one
    /// (identity and grammar slots start `None`).
    pub fn intern(&mut self, name: &str) -> LanguageId {
        if let Some(&id) = self.ids.get(name) {
            return id;
        }
        let id = LanguageId::from_index(self.entries.len());
        // One allocation, shared via Arc: the entry and `ids` both need their
        // own owned key, but they can point at the same heap string instead
        // of each holding an independent copy.
        let owned: Arc<str> = Arc::from(name);
        self.entries.push(LanguageEntry {
            name: Arc::clone(&owned),
            identity: None,
            config: LanguageConfig::default(),
            grammar: None,
        });
        self.ids.insert(owned, id);
        id
    }

    fn entry(&self, id: LanguageId) -> &LanguageEntry {
        &self.entries[id.0 as usize]
    }

    fn entry_mut(&mut self, id: LanguageId) -> &mut LanguageEntry {
        &mut self.entries[id.0 as usize]
    }

    /// The id for `name`, if it has been interned.
    pub fn id_of(&self, name: &str) -> Option<LanguageId> {
        self.ids.get(name).copied()
    }

    /// The name `id` was interned with. `id` must have been minted by this
    /// registry; an out-of-range id is a caller bug.
    pub fn name_of(&self, id: LanguageId) -> &str {
        &self.entry(id).name
    }

    /// The `languageId` LSP servers expect for `id`: the language name
    /// unless an override was configured or bundled. HUME's names diverge
    /// from the LSP spec's well-known identifiers for a handful of languages
    /// (`tsx` → `typescriptreact`).
    pub fn lsp_language_id_of(&self, id: LanguageId) -> &str {
        self.entry(id)
            .config
            .lsp_language_id
            .as_ref()
            .map_or_else(|| self.name_of(id), |s| s.value.as_str())
    }

    /// The comment tokens configured or bundled for `id`.
    pub fn comment_tokens_of(&self, id: LanguageId) -> Option<CommentTokens<'_>> {
        let config = &self.entry(id).config;
        CommentTokens::new(
            config.line_comment_tokens.as_ref().map(|s| &s.value),
            config.block_comment_tokens.as_ref().map(|s| &s.value),
        )
    }

    /// The workspace root markers configured or bundled for `id`; empty when
    /// there are none.
    pub fn roots_of(&self, id: LanguageId) -> &[String] {
        self.entry(id)
            .config
            .roots
            .as_ref()
            .map_or(&[], |s| s.value.as_slice())
    }

    // ── Per-language configuration ────────────────────────────────────────────

    /// Set each field `fields` gives a value for from configuration, with the
    /// outcome of each write.
    #[must_use]
    pub fn configure_fields(
        &mut self,
        id: LanguageId,
        fields: LanguageFields,
        by: ConfiguredBy,
    ) -> Vec<(LanguageField, ConfigureOutcome)> {
        let LanguageFields {
            roots,
            lsp_language_id,
            comment_tokens,
            block_comment_tokens,
        } = fields;
        let config = &mut self.entry_mut(id).config;
        let mut outcomes = Vec::new();
        if let Some(roots) = roots {
            outcomes.push((
                LanguageField::Roots,
                configure(&mut config.roots, roots, by),
            ));
        }
        if let Some(lsp_language_id) = lsp_language_id {
            outcomes.push((
                LanguageField::LanguageId,
                configure(&mut config.lsp_language_id, lsp_language_id, by),
            ));
        }
        if let Some(tokens) = comment_tokens {
            outcomes.push((
                LanguageField::CommentTokens,
                configure(&mut config.line_comment_tokens, tokens, by),
            ));
        }
        if let Some(tokens) = block_comment_tokens {
            outcomes.push((
                LanguageField::BlockCommentTokens,
                configure(&mut config.block_comment_tokens, tokens, by),
            ));
        }
        outcomes
    }

    /// Whether `id`'s bundled data file has been read.
    pub fn bundled_loaded(&self, id: LanguageId) -> bool {
        self.entry(id).config.bundled_loaded
    }

    /// The languages given a grammar since the last call whose bundled data
    /// file has not been read, for the caller to read.
    pub fn take_bundled_pending(&mut self) -> Vec<LanguageId> {
        std::mem::take(&mut self.bundled_pending)
    }

    /// The grammars whose load failed since the last call, for the caller to
    /// report.
    pub fn take_failed_loads(&mut self) -> Vec<(LanguageId, RegisterError)> {
        std::mem::take(&mut self.failed_loads)
    }

    /// Record that `id`'s bundled data file has been read, filling each field
    /// it gives a value for that is still unset.
    pub fn fill_bundled(&mut self, id: LanguageId, bundled: LanguageFields) {
        let LanguageFields {
            roots,
            lsp_language_id,
            comment_tokens,
            block_comment_tokens,
        } = bundled;
        let config = &mut self.entry_mut(id).config;
        config.bundled_loaded = true;
        if let Some(roots) = roots {
            fill(&mut config.roots, roots);
        }
        if let Some(lsp_language_id) = lsp_language_id {
            fill(&mut config.lsp_language_id, lsp_language_id);
        }
        if let Some(tokens) = comment_tokens {
            fill(&mut config.line_comment_tokens, tokens);
        }
        if let Some(tokens) = block_comment_tokens {
            fill(&mut config.block_comment_tokens, tokens);
        }
    }

    // ── Identity ──────────────────────────────────────────────────────────────

    /// Register a language identity: name, extensions, glob patterns, shebangs.
    ///
    /// Returns `Err(RegisterError::GlobBuild)` if the combined glob set would exceed
    /// globset's NFA size limit.
    ///
    /// For batch registration use `register_identity_no_rebuild` + `rebuild_glob_set`.
    /// Takes the detection data as text, which must be valid.
    #[cfg(any(test, feature = "test-util"))]
    pub fn register_identity(
        &mut self,
        name: &str,
        extensions: &[&str],
        globs: &[&str],
        shebangs: &[&str],
    ) -> Result<LanguageId, RegisterError> {
        let extensions: Vec<FileExtension> = extensions
            .iter()
            .map(|e| FileExtension::new(e).expect("a test extension is valid"))
            .collect();
        let globs: Vec<FileGlob> = globs
            .iter()
            .map(|g| FileGlob::new(g).expect("a test glob is valid"))
            .collect();
        let id = self.register_identity_no_rebuild(name, &extensions, &globs, shebangs);
        self.rebuild_glob_set()?;
        Ok(id)
    }

    /// Insert identity data without rebuilding the compiled glob set.
    ///
    /// Intended for batch registration: call this N times then call
    /// `rebuild_glob_set` once, avoiding O(N²) NFA constructions at startup.
    ///
    /// Replaces extensions/globs/shebangs for `name`; a registered or loaded
    /// grammar and the per-language configuration are kept. Identity and
    /// grammar are independent facts about a language, and re-registering one
    /// must not undo the other. (Symmetric with grammar registration,
    /// which likewise preserves an existing identity.)
    pub fn register_identity_no_rebuild(
        &mut self,
        name: &str,
        extensions: &[FileExtension],
        globs: &[FileGlob],
        shebangs: &[&str],
    ) -> LanguageId {
        let id = self.intern(name);
        if let Some(old) = self.entry_mut(id).identity.take() {
            self.deindex(id, &old);
        }
        let new_identity = LanguageIdentity {
            extensions: extensions.to_vec(),
            globs: globs.to_vec(),
            shebangs: shebangs.iter().map(|s| s.to_string()).collect(),
        };
        for ext in &new_identity.extensions {
            self.by_ext.insert(ext.as_str().to_owned(), id);
        }
        for shebang in &new_identity.shebangs {
            self.shebang_to_id.insert(shebang.clone(), id);
        }
        self.entry_mut(id).identity = Some(new_identity);
        self.lang_order.retain(|&i| i != id);
        self.lang_order.push(id);
        id
    }

    /// Remove `identity`'s entries from the `by_ext`/`shebang_to_id` secondary
    /// indices, but only where `id` is still the current owner: a shared
    /// extension (e.g. `.h` claimed by both `c` and `cpp`) may have been
    /// reassigned to a different language since `identity` was registered, and
    /// deindexing unconditionally would evict that newer owner's mapping.
    /// Note: this does not resurrect an older claimant if `id` was
    /// indeed still the owner; the extension simply becomes unclaimed, which
    /// matches the last-registered-wins model elsewhere in this registry.
    fn deindex(&mut self, id: LanguageId, identity: &LanguageIdentity) {
        for ext in &identity.extensions {
            if self.by_ext.get(ext.as_str()) == Some(&id) {
                self.by_ext.remove(ext.as_str());
            }
        }
        for shebang in &identity.shebangs {
            if self.shebang_to_id.get(shebang.as_str()) == Some(&id) {
                self.shebang_to_id.remove(shebang.as_str());
            }
        }
    }

    /// Rebuild the compiled glob set from current registry state.
    ///
    /// Returns `Err` if the NFA size limit is exceeded; on error the prior
    /// compiled set is preserved.
    pub fn rebuild_glob_set(&mut self) -> Result<(), RegisterError> {
        let (compiled, ids) = self.build_globs().map_err(RegisterError::GlobBuild)?;
        self.compiled_globs = compiled;
        self.glob_lang_ids = ids;
        Ok(())
    }

    /// The language whose [`FileExtension`] `file_name` ends with, the
    /// longest one first: `app.js.map` tries `js.map`, then `map`.
    pub fn by_file_name(&self, file_name: &str) -> Option<LanguageId> {
        file_name
            .char_indices()
            .filter(|&(i, c)| c == '.' && i > 0)
            .find_map(|(i, _)| self.by_ext.get(&file_name[i + 1..]).copied())
    }

    /// The id of `name` when it has a registered identity (e.g. `"rust"`
    /// after its `define-language!`), unlike [`Self::id_of`], which answers
    /// for any interned name.
    pub fn defined_id(&self, name: &str) -> Option<LanguageId> {
        let id = self.id_of(name)?;
        self.entry(id).identity.is_some().then_some(id)
    }

    /// Iterator over registered language names (those with an identity), in
    /// arbitrary order.
    pub fn iter_names(&self) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .filter(|entry| entry.identity.is_some())
            .map(|entry| &*entry.name)
    }

    /// The compiled glob matcher for path-based detection. Index-aligned with
    /// `glob_lang_id`: `glob_lang_id(i)` is the language for match index `i`.
    pub fn compiled_globs(&self) -> &GlobSet {
        &self.compiled_globs
    }

    /// Language id for glob match index `i` (from `GlobSet::matches`).
    pub fn glob_lang_id(&self, i: usize) -> Option<LanguageId> {
        self.glob_lang_ids.get(i).copied()
    }

    /// Look up a language by shebang substring (e.g. `"python"`).
    pub fn by_shebang(&self, token: &str) -> Option<LanguageId> {
        self.shebang_to_id.get(token).copied()
    }

    /// Remove a registered language's identity by name, returning it if
    /// present. Also clears any grammar of the same id, loaded or not, since
    /// a removed language has no detection and no grammar, matching `remove`
    /// deleting the language wholesale.
    #[cfg(any(test, feature = "test-util"))]
    pub fn remove(&mut self, name: &str) -> Option<LanguageIdentity> {
        let id = self.id_of(name)?;
        let identity = self.entry_mut(id).identity.take()?;
        self.deindex(id, &identity);
        self.entry_mut(id).grammar = None;
        self.lang_order.retain(|&i| i != id);
        // Only reachable if the set already failed to compile before this remove:
        // `rebuild_glob_set` is fail-soft in production (see `apply_pending_language_regs`),
        // so registry state can sit above the NFA limit, and dropping one language need not
        // bring it back under. Test-only helper: panicking is the right signal.
        self.rebuild_glob_set()
            .expect("glob rebuild after remove: glob set was already over the NFA limit");
        self.rebuild_grammar_snapshot();
        Some(identity)
    }

    // ── Grammar ───────────────────────────────────────────────────────────────

    /// Register `source` as `name`'s grammar without opening it: the library
    /// and queries load on the first [`Self::load_grammar`]. Replaces any
    /// grammar `name` had, loaded or not.
    ///
    /// Auto-registers a bare identity (no extensions/globs/shebangs) if the
    /// language name has none yet.
    pub fn register_grammar(&mut self, name: &str, source: GrammarSource) -> LanguageId {
        let id = self.intern(name);
        self.set_grammar(id, GrammarSlot::Registered(source));
        id
    }

    /// Register `source` as `name`'s grammar and load it now.
    ///
    /// A broken optional query fails the whole attach, same as a broken
    /// `highlights.scm`: they all come from the same trusted pinned source,
    /// so there is no separate soft-degrade path. All fallible work happens
    /// before any registry mutation, so a failed attach leaves the registry
    /// as it was, including any grammar `name` already had.
    pub fn attach_grammar(
        &mut self,
        name: &str,
        source: &GrammarSource,
        scope_reg: &mut ScopeRegistry,
    ) -> Result<Arc<GrammarBundle>, RegisterError> {
        let (grammar, queries) = compile_source(name, source, scope_reg)?;
        let id = self.intern(name);
        let bundle = new_bundle(id, grammar, queries);
        self.set_grammar(id, GrammarSlot::Loaded(Arc::clone(&bundle)));
        Ok(bundle)
    }

    /// `id`'s grammar bundle, loading a registered grammar first. `None` when
    /// `id` has no grammar or its load fails. A failed load leaves `id` with
    /// no grammar until it is registered again and queues its error for
    /// [`Self::take_failed_loads`].
    pub fn load_grammar(
        &mut self,
        id: LanguageId,
        scope_reg: &mut ScopeRegistry,
    ) -> Option<Arc<GrammarBundle>> {
        let entry = self.entry(id);
        let compiled = match entry.grammar.as_ref()? {
            GrammarSlot::Registered(source) => compile_source(&entry.name, source, scope_reg),
            GrammarSlot::Loaded(bundle) => return Some(Arc::clone(bundle)),
        };
        match compiled {
            Ok((grammar, queries)) => {
                let bundle = new_bundle(id, grammar, queries);
                self.set_grammar(id, GrammarSlot::Loaded(Arc::clone(&bundle)));
                Some(bundle)
            }
            Err(error) => {
                self.entry_mut(id).grammar = None;
                self.rebuild_grammar_snapshot();
                self.failed_loads.push((id, error));
                None
            }
        }
    }

    /// Every grammar slot a registration or load gives a language is set
    /// here. A language with a grammar has an identity and has its bundled
    /// data file read, so each grammar slot implies both.
    fn set_grammar(&mut self, id: LanguageId, slot: GrammarSlot) {
        let entry = self.entry_mut(id);
        entry.identity.get_or_insert_with(LanguageIdentity::default);
        entry.grammar = Some(slot);
        self.rebuild_grammar_snapshot();
        if !self.bundled_loaded(id) && !self.bundled_pending.contains(&id) {
            self.bundled_pending.push(id);
        }
    }

    /// Whether `id` has a grammar that is loaded or yet to load.
    pub fn grammar_available(&self, id: LanguageId) -> bool {
        self.entry(id).grammar.is_some()
    }

    /// Whether `name` has a grammar that is loaded or yet to load.
    pub fn has_grammar(&self, name: &str) -> bool {
        self.id_of(name)
            .is_some_and(|id| self.grammar_available(id))
    }

    /// `id`'s grammar bundle, if it is loaded.
    #[cfg(any(test, feature = "test-util"))]
    pub fn loaded_grammar(&self, id: LanguageId) -> Option<&Arc<GrammarBundle>> {
        match &self.entry(id).grammar {
            Some(GrammarSlot::Loaded(bundle)) => Some(bundle),
            _ => None,
        }
    }

    pub fn grammar_snapshot(&self) -> Arc<GrammarSnapshot> {
        Arc::clone(&self.grammar_snapshot)
    }

    fn rebuild_grammar_snapshot(&mut self) {
        let grammars = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let grammar = match entry.grammar.as_ref()? {
                    GrammarSlot::Loaded(bundle) => SnapshotGrammar::Loaded(Arc::clone(bundle)),
                    GrammarSlot::Registered(_) => {
                        SnapshotGrammar::Unloaded(LanguageId::from_index(index))
                    }
                };
                Some((Arc::clone(&entry.name), grammar))
            })
            .collect();
        self.grammar_snapshot = Arc::new(GrammarSnapshot::new(grammars));
    }

    fn build_globs(&self) -> Result<(GlobSet, Vec<LanguageId>), globset::Error> {
        let mut builder = GlobSetBuilder::new();
        let mut ids = Vec::new();
        for &lang_id in &self.lang_order {
            if let Some(identity) = &self.entry(lang_id).identity {
                for glob in &identity.globs {
                    builder.add(glob.0.clone());
                    ids.push(lang_id);
                }
            }
        }
        Ok((builder.build()?, ids))
    }
}

// ── Language detection ────────────────────────────────────────────────────────

/// The two absolute paths a file is detected by: the path as it was opened,
/// symlinks kept, and the path with symlinks resolved. They differ for a
/// file opened through a symlink, such as a config file linked from a
/// dotfiles repository.
#[derive(Clone, Copy, Debug)]
pub struct DetectPaths<'a> {
    pub as_opened: &'a Path,
    pub resolved: &'a Path,
}

/// Detect the language for a buffer given its paths and first line.
///
/// Priority: the path as opened, then the resolved path, each by glob
/// (last-registered wins) and then by file extension; then the shebang.
/// Returns the language id, or `None` if unrecognised.
pub fn detect_language(
    paths: Option<DetectPaths>,
    first_line: Option<&str>,
    registry: &LanguageRegistry,
) -> Option<LanguageId> {
    paths
        .and_then(|paths| {
            detect_path(paths.as_opened, registry).or_else(|| {
                (paths.resolved != paths.as_opened)
                    .then(|| detect_path(paths.resolved, registry))
                    .flatten()
            })
        })
        .or_else(|| first_line.and_then(|line| detect_shebang(line, registry)))
}

/// `path`'s language by glob, then by file extension.
fn detect_path(path: &Path, registry: &LanguageRegistry) -> Option<LanguageId> {
    debug_assert!(
        path.is_absolute(),
        "detection takes an absolute path, got {path:?}"
    );
    registry
        .compiled_globs()
        .matches(path)
        .last()
        .and_then(|&i| registry.glob_lang_id(i))
        .or_else(|| {
            let name = path.file_name()?.to_str()?;
            registry.by_file_name(name)
        })
}

fn detect_shebang(line: &str, registry: &LanguageRegistry) -> Option<LanguageId> {
    let after_bang = line.strip_prefix("#!")?;
    let mut tokens = after_bang.split_whitespace();
    let interpreter_path = tokens.next()?;

    let interpreter = if interpreter_path.ends_with("/env") || interpreter_path == "env" {
        tokens.find(|t| !t.starts_with('-'))?
    } else {
        std::path::Path::new(interpreter_path)
            .file_name()?
            .to_str()?
    };

    registry.by_shebang(interpreter)
}

#[cfg(test)]
mod tests;
