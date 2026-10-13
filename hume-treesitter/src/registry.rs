use rustc_hash::FxHashMap;
use std::path::Path;
use std::sync::Arc;

use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::grammar::LoadedGrammar;
use crate::highlight::TreeSitterHighlighter;
use hume_editing::comment::{BlockCommentTokens, CommentTokens, LineCommentTokens};
use hume_engine::theme::ScopeRegistry;

use crate::injections::InjectionsQuery;
use crate::textobjects::TextObjectsQuery;

/// Interned language identity. Dense, append-only, minted by `LanguageRegistry`.
/// An id is only ever handed out by `LanguageRegistry::intern`; indexing a
/// different registry instance with it is a caller bug, not a runtime error.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LanguageId(u32);

/// Identity of one language: its detection data (extensions, glob patterns,
/// shebangs).
///
/// Immutable once registered: re-registration (`register_identity_no_rebuild`)
/// replaces the whole record rather than mutating it in place. The name is
/// not stored here: it lives once, in the registry's interner (SSOT).
#[derive(Debug, Default)]
pub struct LanguageIdentity {
    pub extensions: Vec<String>,
    /// Parsed glob patterns (e.g. `"Makefile"`, `"*.{ts,tsx}"`), validated at
    /// registration, so an uncompilable pattern can never reach here. Source
    /// text is recoverable via `Glob::glob()`. The combined matcher across all
    /// languages lives on `LanguageRegistry` (`compiled_globs`).
    pub globs: Vec<Glob>,
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
    /// language, not one per buffer (capture names are interned once at
    /// attach time).
    pub highlighter: Arc<TreeSitterHighlighter>,
    /// Compiled `injections.scm`, if the grammar has one. `None` means this
    /// language never injects embedded languages.
    pub injections: Option<InjectionsQuery>,
    /// Compiled `textobjects.scm`, if the grammar has one. `None` means this
    /// language ships no structural text objects or navigation.
    pub textobjects: Option<TextObjectsQuery>,
    /// Unique per attach, issued by `LanguageRegistry::next_gen`. Grammar-swap
    /// / staleness checks compare this instead of `Arc::ptr_eq`, a plain
    /// integer identity that survives across the worker-thread boundary.
    pub config_gen: u32,
    /// The language this grammar was attached for.
    pub language: LanguageId,
}

/// The query files `attach_grammar` may compile for a grammar. `highlights`
/// is required; `injections` and `textobjects` are each independently
/// optional. Named fields, so the two optional paths can't be swapped at a
/// call site.
#[derive(Clone, Copy)]
pub struct QueryPaths<'a> {
    pub highlights: &'a Path,
    pub injections: Option<&'a Path>,
    pub textobjects: Option<&'a Path>,
}

impl<'a> QueryPaths<'a> {
    /// `highlights` alone, no injections or textobjects query. The common
    /// shape: most grammars (and most test fixtures) have nothing else to
    /// offer.
    pub fn highlights_only(highlights: &'a Path) -> Self {
        Self {
            highlights,
            injections: None,
            textobjects: None,
        }
    }
}

/// Everything the registry holds for one interned language.
struct LanguageEntry {
    name: Arc<str>,
    /// `None` = interned but no identity registered.
    identity: Option<LanguageIdentity>,
    config: LanguageConfig,
    grammar: Option<Arc<GrammarBundle>>,
}

/// Global registry of configured language identities. Lives on `Editor`.
///
/// `ids` and `entries` are the interner: dense and append-only, with
/// `entries[id.0]` holding the language `id` names.
pub struct LanguageRegistry {
    ids: FxHashMap<Arc<str>, LanguageId>,
    entries: Vec<LanguageEntry>,
    /// Detection indices, never rebuilt on `attach_grammar`, only on
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
    /// Grammared languages only, rebuilt whenever the grammar table changes.
    /// Handed to the parse worker so it can resolve an injected language name
    /// (a dynamically-discovered info-string) to its grammar without
    /// touching main-thread state.
    grammar_snapshot: Arc<FxHashMap<String, Arc<GrammarBundle>>>,
    /// Source of `GrammarBundle::config_gen`, incremented on every grammar
    /// attach so each attached bundle gets a unique identity.
    next_config_gen: u32,
    /// Languages given a grammar whose bundled data file has not been read.
    /// A syntax layer exists only for a language with a grammar, so reading
    /// these before any parse means every layer's language has its data.
    bundled_pending: Vec<LanguageId>,
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
    /// Failed to read the injections query file.
    InjectionsRead(std::io::Error),
    /// Failed to compile the injections query.
    InjectionsQueryBuild(tree_sitter::QueryError),
    /// Failed to read the textobjects query file.
    TextObjectsRead(std::io::Error),
    /// Failed to compile the textobjects query.
    TextObjectsQueryBuild(tree_sitter::QueryError),
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
            Self::InjectionsRead(e) => write!(f, "injections.scm read failed: {e}"),
            Self::InjectionsQueryBuild(e) => write!(f, "injections query compilation failed: {e}"),
            Self::TextObjectsRead(e) => write!(f, "textobjects.scm read failed: {e}"),
            Self::TextObjectsQueryBuild(e) => {
                write!(f, "textobjects query compilation failed: {e}")
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
            grammar_snapshot: Arc::new(FxHashMap::default()),
            next_config_gen: 0,
            bundled_pending: Vec::new(),
        }
    }
}

impl LanguageRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Issue a fresh `config_gen` for a newly attached `GrammarBundle`.
    fn next_gen(&mut self) -> u32 {
        let next = self.next_config_gen;
        self.next_config_gen += 1;
        next
    }

    // ── Interner ──────────────────────────────────────────────────────────────

    /// Mint-or-get: returns the existing id for `name`, or interns a fresh one
    /// (identity and grammar slots start `None`).
    pub fn intern(&mut self, name: &str) -> LanguageId {
        if let Some(&id) = self.ids.get(name) {
            return id;
        }
        let id = LanguageId(self.entries.len() as u32);
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
    /// unless an override was configured or bundled. HUME's names mirror
    /// Helix's, which diverge from the LSP spec's well-known identifiers for a
    /// handful of languages (`tsx` → `typescriptreact`).
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
    #[cfg(any(test, feature = "test-util"))]
    pub fn register_identity(
        &mut self,
        name: &str,
        extensions: &[&str],
        globs: &[Glob],
        shebangs: &[&str],
    ) -> Result<LanguageId, RegisterError> {
        let id = self.register_identity_no_rebuild(name, extensions, globs, shebangs);
        self.rebuild_glob_set()?;
        Ok(id)
    }

    /// Insert identity data without rebuilding the compiled glob set.
    ///
    /// Intended for batch registration: call this N times then call
    /// `rebuild_glob_set` once, avoiding O(N²) NFA constructions at startup.
    ///
    /// Replaces extensions/globs/shebangs for `name`; an already-attached
    /// grammar and the per-language configuration are kept. Identity and grammar are independent
    /// facts about a language, and re-registering one must not silently undo
    /// the other. (Symmetric with `attach_grammar`, which likewise preserves an
    /// existing identity.) A grammar only ever changes via `attach_grammar`.
    pub fn register_identity_no_rebuild(
        &mut self,
        name: &str,
        extensions: &[&str],
        globs: &[Glob],
        shebangs: &[&str],
    ) -> LanguageId {
        let id = self.intern(name);
        if let Some(old) = self.entry_mut(id).identity.take() {
            self.deindex(id, &old);
        }
        let new_identity = LanguageIdentity {
            extensions: extensions.iter().map(|s| s.to_string()).collect(),
            globs: globs.to_vec(),
            shebangs: shebangs.iter().map(|s| s.to_string()).collect(),
        };
        for ext in &new_identity.extensions {
            self.by_ext.insert(ext.clone(), id);
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

    /// Look up a language by file extension (e.g. `"rs"`, `"C"`).
    pub fn by_extension(&self, ext: &str) -> Option<LanguageId> {
        self.by_ext.get(ext).copied()
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
    /// present. Also clears any attached grammar for the same id, since a removed
    /// language has no detection and no grammar, matching `remove` deleting
    /// the language wholesale.
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

    /// Attach a tree-sitter grammar to a language.
    ///
    /// Reads the highlights query file, compiles it, builds the shared
    /// highlighter (interning its capture names into `scope_reg`), optionally
    /// reads and compiles `queries.injections` / `queries.textobjects` if
    /// given, then installs the resulting `GrammarBundle` for `name`'s id.
    /// Detection indices (`by_ext`/globs/shebangs) are never touched.
    ///
    /// A broken `injections.scm` or `textobjects.scm` fails the whole attach,
    /// same as a broken `highlights.scm`: all three come from the same
    /// trusted pinned source, so there is no separate soft-degrade path. All
    /// fallible work happens before any registry mutation, so a failed
    /// attach leaves no partial state.
    ///
    /// Auto-registers a bare identity (no extensions/globs/shebangs) if the
    /// language name has none yet.
    pub fn attach_grammar(
        &mut self,
        name: &str,
        grammar_path: &Path,
        symbol: &str,
        queries: QueryPaths<'_>,
        scope_reg: &mut ScopeRegistry,
    ) -> Result<Arc<GrammarBundle>, RegisterError> {
        let grammar =
            LoadedGrammar::open(grammar_path, symbol).map_err(RegisterError::GrammarLoad)?;
        let abi = grammar.language().abi_version();
        let supported =
            tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION;
        if !supported.contains(&abi) {
            return Err(RegisterError::AbiIncompatible {
                name: name.to_owned(),
                abi,
                supported,
            });
        }
        let highlights_src =
            std::fs::read_to_string(queries.highlights).map_err(RegisterError::HighlightsRead)?;
        let query = Arc::new(
            tree_sitter::Query::new(grammar.language(), &highlights_src)
                .map_err(RegisterError::QueryBuild)?,
        );
        let highlighter = Arc::new(TreeSitterHighlighter::from_shared_query(query, scope_reg));
        let injections = queries
            .injections
            .map(|path| {
                let src = std::fs::read_to_string(path).map_err(RegisterError::InjectionsRead)?;
                let query = Arc::new(
                    tree_sitter::Query::new(grammar.language(), &src)
                        .map_err(RegisterError::InjectionsQueryBuild)?,
                );
                Ok::<_, RegisterError>(InjectionsQuery::new(query))
            })
            .transpose()?;
        let textobjects = queries
            .textobjects
            .map(|path| {
                let src = std::fs::read_to_string(path).map_err(RegisterError::TextObjectsRead)?;
                let query = tree_sitter::Query::new(grammar.language(), &src)
                    .map_err(RegisterError::TextObjectsQueryBuild)?;
                Ok::<_, RegisterError>(TextObjectsQuery::new(query))
            })
            .transpose()?;

        let id = self.intern(name);
        self.entry_mut(id)
            .identity
            .get_or_insert_with(LanguageIdentity::default);
        let config_gen = self.next_gen();
        let bundle = Arc::new(GrammarBundle {
            grammar,
            highlighter,
            injections,
            textobjects,
            config_gen,
            language: id,
        });
        self.entry_mut(id).grammar = Some(Arc::clone(&bundle));
        self.rebuild_grammar_snapshot();
        if !self.bundled_loaded(id) && !self.bundled_pending.contains(&id) {
            self.bundled_pending.push(id);
        }
        Ok(bundle)
    }

    /// Returns `true` if `name` has an attached tree-sitter grammar.
    pub fn has_grammar(&self, name: &str) -> bool {
        self.id_of(name)
            .is_some_and(|id| self.entry(id).grammar.is_some())
    }

    /// The grammar bundle attached to `id`, if any.
    pub fn grammar(&self, id: LanguageId) -> Option<&Arc<GrammarBundle>> {
        self.entry(id).grammar.as_ref()
    }

    /// Snapshot of grammared languages, keyed by name. Handed to the parse
    /// worker so it can resolve an injection language name (a
    /// dynamically-discovered info string) to its grammar without touching
    /// main-thread state.
    pub fn grammar_snapshot(&self) -> Arc<FxHashMap<String, Arc<GrammarBundle>>> {
        Arc::clone(&self.grammar_snapshot)
    }

    fn rebuild_grammar_snapshot(&mut self) {
        self.grammar_snapshot = Arc::new(
            self.entries
                .iter()
                .filter_map(|entry| {
                    entry
                        .grammar
                        .as_ref()
                        .map(|b| (entry.name.to_string(), Arc::clone(b)))
                })
                .collect(),
        );
    }

    fn build_globs(&self) -> Result<(GlobSet, Vec<LanguageId>), globset::Error> {
        let mut builder = GlobSetBuilder::new();
        let mut ids = Vec::new();
        for &lang_id in &self.lang_order {
            if let Some(identity) = &self.entry(lang_id).identity {
                for glob in &identity.globs {
                    builder.add(glob.clone());
                    ids.push(lang_id);
                }
            }
        }
        Ok((builder.build()?, ids))
    }
}

// ── Language detection ────────────────────────────────────────────────────────

/// Detect the language for a buffer given its path and first line.
///
/// Priority: glob match (most-specific / last-registered) → file extension →
/// shebang. Returns the language id, or `None` if unrecognised.
pub fn detect_language(
    path: Option<&std::path::Path>,
    first_line: Option<&str>,
    registry: &LanguageRegistry,
) -> Option<LanguageId> {
    if let Some(path) = path {
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(std::path::Path::new);
        if let Some(name_path) = file_name {
            let matches = registry.compiled_globs().matches(name_path);
            if let Some(&last_idx) = matches.last()
                && let Some(id) = registry.glob_lang_id(last_idx)
            {
                return Some(id);
            }
        }

        if let Some(ext) = path.extension().and_then(|e| e.to_str())
            && let Some(id) = registry.by_extension(ext)
        {
            return Some(id);
        }
    }

    if let Some(line) = first_line
        && let Some(id) = detect_shebang(line, registry)
    {
        return Some(id);
    }

    None
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
