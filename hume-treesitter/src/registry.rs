use rustc_hash::FxHashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use globset::{GlobSet, GlobSetBuilder};
use hume_platform::path::AbsolutePath;
use ropey::Rope;

use crate::grammar::LoadedGrammar;
use crate::highlight::TreeSitterHighlighter;
use hume_core::{
    BlockCommentTokens, CommentTokens, ConfigOrigin, FirstWins, GrammarSource, LanguageField,
    LanguageFields, LanguageIdentity, LaterWins, LineCommentTokens, OptionalQuery, OsWait, Ranked,
    ShebangInterpreter, WriteOutcome,
};
#[cfg(any(test, feature = "test-util"))]
use hume_core::{FileExtension, FileGlob};
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

/// A per-language value keeps the first write from one origin, so a bundled
/// data file read after a plugin's configuration never replaces it.
type LanguageTie = FirstWins;

/// A grammar registration from one origin replaces the earlier one.
type GrammarTie = LaterWins;

/// Per-language values outside the identity: set by configuration, or filled
/// from the language's bundled data file the first time a buffer uses it.
/// Re-registering the identity leaves them in place.
#[derive(Debug, Default)]
struct LanguageConfig {
    /// Override for the wire `languageId` sent to language servers, when it
    /// differs from the name (e.g. `"tsx"` needs `"typescriptreact"`).
    lsp_language_id: Ranked<String, LanguageTie>,
    /// File or directory names that make a directory this language's
    /// workspace root (e.g. `"Cargo.toml"`, `".git"`).
    roots: Ranked<Vec<String>, LanguageTie>,
    line_comment_tokens: Ranked<LineCommentTokens, LanguageTie>,
    block_comment_tokens: Ranked<BlockCommentTokens, LanguageTie>,
    /// Whether the bundled data file has been read, found or not.
    bundled_loaded: bool,
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

/// The grammar ABI versions the linked tree-sitter loads, both included.
const GRAMMAR_ABI: std::ops::RangeInclusive<usize> =
    tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION;

/// Refuses grammar `name`'s `abi` unless the linked tree-sitter loads it.
fn require_abi(name: &str, abi: usize) -> Result<(), RegisterError> {
    if GRAMMAR_ABI.contains(&abi) {
        Ok(())
    } else {
        Err(RegisterError::AbiIncompatible {
            name: name.to_owned(),
            abi,
        })
    }
}

/// Opens `source`'s library through `library_wait`, checks its ABI, and
/// reads and compiles its queries. `scope_reg` is touched only once
/// everything else succeeded.
fn compile_source(
    name: &str,
    source: &GrammarSource,
    library_wait: &OsWait,
    scope_reg: &mut ScopeRegistry,
) -> Result<(LoadedGrammar, CompiledQueries), RegisterError> {
    let grammar = library_wait
        .waiting(|| LoadedGrammar::open(&source.path, &source.symbol))
        .map_err(RegisterError::GrammarLoad)?;
    require_abi(name, grammar.language().abi_version())?;
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
    /// Empty = no `define-language!` for it yet.
    identity: Ranked<LanguageIdentity, LanguageTie>,
    config: LanguageConfig,
    grammar: Ranked<GrammarSlot, GrammarTie>,
}

impl LanguageEntry {
    /// Whether the language is defined: given detection data, or a grammar
    /// (`register-grammar!` defines a language it names).
    fn is_defined(&self) -> bool {
        self.identity.get().is_some() || self.grammar.get().is_some()
    }
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
    shebang_to_id: FxHashMap<ShebangInterpreter, LanguageId>,
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
    /// What each grammar library opens through. The editor's is connected
    /// to the scripting budget, since macOS checks a library file the first
    /// time it opens, about 100 ms for a grammar.
    library_wait: OsWait,
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
    /// Grammar ABI version is outside the range the linked tree-sitter
    /// loads.  Recompile the grammar with a compatible generator.
    AbiIncompatible { name: String, abi: usize },
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
            Self::AbiIncompatible { name, abi } => write!(
                f,
                "grammar '{name}' ABI {abi} not in supported range {GRAMMAR_ABI:?}"
            ),
        }
    }
}

impl LanguageRegistry {
    /// An empty registry that opens each grammar library through
    /// `library_wait`.
    pub fn new(library_wait: OsWait) -> Self {
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
            library_wait,
        }
    }

    /// [`Self::new`] with a `library_wait` nothing pauses.
    #[cfg(any(test, feature = "test-util"))]
    pub fn for_testing() -> Self {
        Self::new(OsWait::default())
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
            identity: Ranked::default(),
            config: LanguageConfig::default(),
            grammar: Ranked::default(),
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
            .get()
            .map_or_else(|| self.name_of(id), String::as_str)
    }

    /// The comment tokens configured or bundled for `id`.
    pub fn comment_tokens_of(&self, id: LanguageId) -> Option<CommentTokens<'_>> {
        let config = &self.entry(id).config;
        CommentTokens::new(
            config.line_comment_tokens.get(),
            config.block_comment_tokens.get(),
        )
    }

    /// The workspace root markers configured or bundled for `id`; empty when
    /// there are none.
    pub fn roots_of(&self, id: LanguageId) -> &[String] {
        self.entry(id).config.roots.get().map_or(&[], Vec::as_slice)
    }

    // ── Per-language configuration ────────────────────────────────────────────

    /// Set each field `fields` gives a value for from configuration, with the
    /// outcome of each write.
    #[must_use]
    pub fn configure_fields(
        &mut self,
        id: LanguageId,
        fields: LanguageFields,
        by: ConfigOrigin,
    ) -> Vec<(LanguageField, WriteOutcome)> {
        let LanguageFields {
            roots,
            lsp_language_id,
            comment_tokens,
            block_comment_tokens,
        } = fields;
        let config = &mut self.entry_mut(id).config;
        let mut outcomes = Vec::new();
        if let Some(roots) = roots {
            outcomes.push((LanguageField::Roots, config.roots.write(roots, by).outcome));
        }
        if let Some(lsp_language_id) = lsp_language_id {
            outcomes.push((
                LanguageField::LanguageId,
                config.lsp_language_id.write(lsp_language_id, by).outcome,
            ));
        }
        if let Some(tokens) = comment_tokens {
            outcomes.push((
                LanguageField::CommentTokens,
                config.line_comment_tokens.write(tokens, by).outcome,
            ));
        }
        if let Some(tokens) = block_comment_tokens {
            outcomes.push((
                LanguageField::BlockCommentTokens,
                config.block_comment_tokens.write(tokens, by).outcome,
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
        self.entry_mut(id).config.bundled_loaded = true;
        // Bundled data ranks lowest and is read at first use, after any
        // value a script set, so a field that keeps its value is expected.
        let _ = self.configure_fields(id, bundled, ConfigOrigin::Bundled);
    }

    // ── Identity ──────────────────────────────────────────────────────────────

    /// Register a language identity from its detection data as text, which
    /// must be valid, as HUME's bundled data, and rebuild the glob set.
    ///
    /// Returns `Err(RegisterError::GlobBuild)` if the combined glob set would exceed
    /// globset's NFA size limit.
    #[cfg(any(test, feature = "test-util"))]
    pub fn register_identity(
        &mut self,
        name: &str,
        extensions: &[&str],
        globs: &[&str],
        shebangs: &[&str],
    ) -> Result<LanguageId, RegisterError> {
        self.register_identity_by(name, extensions, globs, shebangs, ConfigOrigin::Bundled)
            .map(|(id, _)| id)
    }

    /// [`Self::register_identity`] as made by `by`, with the outcome of its
    /// ranking.
    #[cfg(any(test, feature = "test-util"))]
    pub fn register_identity_by(
        &mut self,
        name: &str,
        extensions: &[&str],
        globs: &[&str],
        shebangs: &[&str],
        by: ConfigOrigin,
    ) -> Result<(LanguageId, WriteOutcome), RegisterError> {
        let extensions: Vec<FileExtension> = extensions
            .iter()
            .map(|&e| FileExtension::new(e.to_owned()).expect("a test extension is valid"))
            .collect();
        let globs: Vec<FileGlob> = globs
            .iter()
            .map(|g| FileGlob::new(g).expect("a test glob is valid"))
            .collect();
        let shebangs: Vec<ShebangInterpreter> = shebangs
            .iter()
            .map(|&s| ShebangInterpreter::new(s.to_owned()).expect("a test shebang is valid"))
            .collect();
        let identity = LanguageIdentity {
            extensions,
            globs,
            shebangs,
        };
        let registered = self.register_identity_no_rebuild(name, identity, by);
        self.rebuild_glob_set()?;
        Ok(registered)
    }

    /// Insert identity data without rebuilding the compiled glob set.
    ///
    /// Intended for batch registration: call this N times then call
    /// `rebuild_glob_set` once, avoiding O(N²) NFA constructions at startup.
    ///
    /// Sets `name`'s detection data to `identity` from `by`, ranked as every
    /// per-language value is: detection data from the same or a higher tier
    /// stays, and the outcome says so. A registered or loaded grammar and
    /// the per-language configuration are kept. Identity and grammar are
    /// independent facts about a language, and re-registering one must not
    /// undo the other. (Symmetric with grammar registration, which likewise
    /// preserves an existing identity.)
    pub fn register_identity_no_rebuild(
        &mut self,
        name: &str,
        identity: LanguageIdentity,
        by: ConfigOrigin,
    ) -> (LanguageId, WriteOutcome) {
        let id = self.intern(name);
        let written = self.entry_mut(id).identity.write(identity, by);
        if !written.outcome.is_applied() {
            return (id, written.outcome);
        }
        if let Some(old) = written.displaced {
            self.deindex(id, &old);
        }
        self.index(id);
        self.lang_order.retain(|&i| i != id);
        self.lang_order.push(id);
        (id, written.outcome)
    }

    /// Point the `by_ext`/`shebang_to_id` secondary indices at `id` for each
    /// extension and interpreter of its held identity.
    fn index(&mut self, id: LanguageId) {
        let Self {
            entries,
            by_ext,
            shebang_to_id,
            ..
        } = self;
        let Some(identity) = entries[id.0 as usize].identity.get() else {
            return;
        };
        for ext in &identity.extensions {
            by_ext.insert(ext.as_str().to_owned(), id);
        }
        for shebang in &identity.shebangs {
            shebang_to_id.insert(shebang.clone(), id);
        }
    }

    /// Remove `identity`'s entries from the `by_ext`/`shebang_to_id` secondary
    /// indices, but only where `id` is still the current owner: a shared
    /// extension (e.g. `.h` claimed by both `c` and `cpp`) may have been
    /// reassigned to a different language since `identity` was registered, and
    /// deindexing unconditionally would evict that newer owner's mapping.
    /// Note: this does not resurrect an older claimant if `id` was
    /// indeed still the owner; the extension simply becomes unclaimed, as
    /// between two languages the one whose definition applied last claims a
    /// shared extension.
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

    /// The language whose [`hume_core::FileExtension`] `file_name` ends with, the
    /// longest one first: `app.js.map` tries `js.map`, then `map`.
    pub fn by_file_name(&self, file_name: &str) -> Option<LanguageId> {
        file_name
            .char_indices()
            .filter(|&(i, c)| c == '.' && i > 0)
            .find_map(|(i, _)| self.by_ext.get(&file_name[i + 1..]).copied())
    }

    /// The id of `name` when it is defined (e.g. `"rust"` after its
    /// `define-language!` or `register-grammar!`), unlike [`Self::id_of`],
    /// which answers for any interned name.
    pub fn defined_id(&self, name: &str) -> Option<LanguageId> {
        let id = self.id_of(name)?;
        self.entry(id).is_defined().then_some(id)
    }

    /// Iterator over defined language names, in arbitrary order.
    pub fn iter_names(&self) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .filter(|entry| entry.is_defined())
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

    /// The language whose [`ShebangInterpreter`] is `program`, the program
    /// a `#!` line runs, then one named by `program` with less of its
    /// trailing version: each `.N` part dropped in turn, then the digits
    /// left. `python3.12` tries `python3.12`, `python3`, then `python`. The
    /// most specific name first, as [`Self::by_file_name`] tries the longest
    /// extension first.
    pub fn by_interpreter(&self, program: &str) -> Option<LanguageId> {
        let dotted = std::iter::successors(Some(program), |name| {
            name.rsplit_once('.')
                .filter(|(_, minor)| !minor.is_empty() && minor.bytes().all(|b| b.is_ascii_digit()))
                .map(|(major, _)| major)
        });
        let base = dotted.clone().fold(program, |_, name| name);
        let unversioned = base.trim_end_matches(|c: char| c.is_ascii_digit());
        dotted
            .chain((unversioned != base).then_some(unversioned))
            .find_map(|name| self.shebang_to_id.get(name).copied())
    }

    /// Remove a defined language wholesale: its identity, if it has one,
    /// and its grammar, loaded or not. Returns whether `name` was defined.
    #[cfg(any(test, feature = "test-util"))]
    pub fn remove(&mut self, name: &str) -> bool {
        let Some(id) = self.defined_id(name) else {
            return false;
        };
        if let Some(identity) = self.entry_mut(id).identity.take() {
            self.deindex(id, &identity);
        }
        self.entry_mut(id).grammar = Ranked::default();
        self.lang_order.retain(|&i| i != id);
        // Only reachable if the set already failed to compile before this remove:
        // `rebuild_glob_set` is fail-soft in production (see `apply_pending_language_regs`),
        // so registry state can sit above the NFA limit, and dropping one language need not
        // bring it back under. Test-only helper: panicking is the right signal.
        self.rebuild_glob_set()
            .expect("glob rebuild after remove: glob set was already over the NFA limit");
        self.rebuild_grammar_snapshot();
        true
    }

    // ── Grammar ───────────────────────────────────────────────────────────────

    /// Register `source` from `by` as `name`'s grammar without opening it:
    /// the library and queries load on the first [`Self::load_grammar`].
    /// Replaces any grammar `name` had, loaded or not, unless a higher
    /// origin registered it; `None` then.
    ///
    /// Defines the language if it is not yet, with no detection data.
    pub fn register_grammar(
        &mut self,
        name: &str,
        source: GrammarSource,
        by: ConfigOrigin,
    ) -> Option<LanguageId> {
        if !self.rank_grammar(name, by).is_applied() {
            return None;
        }
        let id = self.intern(name);
        self.set_grammar(id, GrammarSlot::Registered(source), by);
        Some(id)
    }

    /// Register `source` from `by` as `name`'s grammar and load it now.
    /// `Ok(None)`, opening nothing, when a higher origin registered `name`'s
    /// grammar, as [`Self::register_grammar`] ranks it.
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
        by: ConfigOrigin,
        scope_reg: &mut ScopeRegistry,
    ) -> Result<Option<Arc<GrammarBundle>>, RegisterError> {
        if !self.rank_grammar(name, by).is_applied() {
            return Ok(None);
        }
        let (grammar, queries) = compile_source(name, source, &self.library_wait, scope_reg)?;
        let id = self.intern(name);
        let bundle = new_bundle(id, grammar, queries);
        self.set_grammar(id, GrammarSlot::Loaded(Arc::clone(&bundle)), by);
        Ok(Some(bundle))
    }

    /// Opens `source`'s library and compiles its queries as a load of
    /// `name`'s grammar would, registering nothing. The library stays open,
    /// as every library this process opens does.
    pub fn check_grammar(&self, name: &str, source: &GrammarSource) -> Result<(), RegisterError> {
        compile_source(name, source, &self.library_wait, &mut ScopeRegistry::new()).map(drop)
    }

    /// What registering a grammar for `name` from `by` would do, without
    /// interning `name`.
    fn rank_grammar(&self, name: &str, by: ConfigOrigin) -> WriteOutcome {
        match self.id_of(name) {
            Some(id) => self.entry(id).grammar.rank(by),
            None => Ranked::<GrammarSlot, GrammarTie>::default().rank(by),
        }
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
        let by = entry.grammar.by()?;
        let compiled = match entry.grammar.get()? {
            GrammarSlot::Registered(source) => {
                compile_source(&entry.name, source, &self.library_wait, scope_reg)
            }
            GrammarSlot::Loaded(bundle) => return Some(Arc::clone(bundle)),
        };
        match compiled {
            Ok((grammar, queries)) => {
                let bundle = new_bundle(id, grammar, queries);
                self.set_grammar(id, GrammarSlot::Loaded(Arc::clone(&bundle)), by);
                Some(bundle)
            }
            Err(error) => {
                self.entry_mut(id).grammar = Ranked::default();
                self.rebuild_grammar_snapshot();
                self.failed_loads.push((id, error));
                None
            }
        }
    }

    /// Every grammar slot a registration or load gives a language is set
    /// here. A language with a grammar is defined and has its bundled data
    /// file read, so each grammar slot implies both.
    fn set_grammar(&mut self, id: LanguageId, slot: GrammarSlot, by: ConfigOrigin) {
        let written = self.entry_mut(id).grammar.write(slot, by);
        assert!(
            written.outcome.is_applied(),
            "a grammar slot is set only after its origin was ranked"
        );
        self.rebuild_grammar_snapshot();
        if !self.bundled_loaded(id) && !self.bundled_pending.contains(&id) {
            self.bundled_pending.push(id);
        }
    }

    /// Whether `id` has a grammar that is loaded or yet to load.
    pub fn grammar_available(&self, id: LanguageId) -> bool {
        self.entry(id).grammar.get().is_some()
    }

    /// Whether `name` has a grammar that is loaded or yet to load.
    pub fn has_grammar(&self, name: &str) -> bool {
        self.id_of(name)
            .is_some_and(|id| self.grammar_available(id))
    }

    /// `id`'s grammar bundle, if it is loaded.
    #[cfg(any(test, feature = "test-util"))]
    pub fn loaded_grammar(&self, id: LanguageId) -> Option<&Arc<GrammarBundle>> {
        match self.entry(id).grammar.get() {
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
                let grammar = match entry.grammar.get()? {
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
            if let Some(identity) = self.entry(lang_id).identity.get() {
                for glob in &identity.globs {
                    builder.add(glob.glob().clone());
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
    pub as_opened: &'a AbsolutePath,
    pub resolved: &'a AbsolutePath,
}

/// Detect the language for a buffer given its paths and its text.
///
/// Priority: the path as opened, then the resolved path, each by glob
/// (last-registered wins) and then by file extension; then the shebang on
/// the text's first line. Returns the language id, or `None` if
/// unrecognised.
pub fn detect_language(
    paths: Option<DetectPaths>,
    text: &Rope,
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
        .or_else(|| detect_shebang(&shebang_line(text)?, registry))
}

/// `path`'s language by glob, then by file extension.
fn detect_path(path: &AbsolutePath, registry: &LanguageRegistry) -> Option<LanguageId> {
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

/// The longest `#!` line Linux reads, in bytes.
const SHEBANG_LINE_MAX_BYTES: usize = 256;

/// `text`'s first line, up to its `\n` or the last whole char within
/// [`SHEBANG_LINE_MAX_BYTES`]. `None` when it is empty.
fn shebang_line(text: &Rope) -> Option<String> {
    let mut line = String::new();
    for ch in text.chars() {
        if ch == '\n' || line.len() + ch.len_utf8() > SHEBANG_LINE_MAX_BYTES {
            break;
        }
        line.push(ch);
    }
    (!line.is_empty()).then_some(line)
}

fn detect_shebang(line: &str, registry: &LanguageRegistry) -> Option<LanguageId> {
    registry.by_interpreter(shebang_program(line)?)
}

/// The env(1) options, GNU and BSD, whose operand is the next word.
const ENV_OPTIONS_WITH_OPERAND: &[&str] = &[
    "-u", "--unset", "-C", "--chdir", "-a", "--argv0", "-P", "-L", "-U",
];

/// The name of the program a `#!` line runs: the last component of its
/// interpreter's path or, for `env`, of the command `env` runs, past env's
/// options and `NAME=value` assignments. `-S` takes no operand of its own,
/// since its words are the command line.
fn shebang_program(line: &str) -> Option<&str> {
    let mut words = line.strip_prefix("#!")?.split_whitespace();
    let mut program = last_component(words.next()?);
    if program == "env" {
        let mut skip_operand = false;
        let command = words.find(|word| {
            if std::mem::take(&mut skip_operand) {
                return false;
            }
            skip_operand = ENV_OPTIONS_WITH_OPERAND.contains(word);
            !word.starts_with('-') && !word.contains('=')
        })?;
        program = last_component(command);
    }
    Some(program)
}

/// `path`'s last component, split on both separators: a `#!` line is text
/// in a file, not a path on this host.
fn last_component(path: &str) -> &str {
    path.rsplit_once(['/', '\\']).map_or(path, |(_, name)| name)
}

#[cfg(test)]
mod tests;
