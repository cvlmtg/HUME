use rustc_hash::FxHashMap;
use std::path::Path;
use std::sync::Arc;

use globset::{GlobSet, GlobSetBuilder};

use crate::grammar::LoadedGrammar;
use crate::highlight::TreeSitterHighlighter;
use hume_engine::theme::ScopeRegistry;

use crate::injections::InjectionsQuery;

// ── LanguageId ────────────────────────────────────────────────────────────────

/// Interned language identity. Dense, append-only, minted by `LanguageRegistry`.
/// An id is only ever handed out by `LanguageRegistry::intern` — indexing a
/// different registry instance with it is a caller bug, not a runtime error.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LanguageId(u32);

// ── LanguageIdentity ────────────────────────────────────────────────────────────

/// Detection identity for one language: extensions, glob patterns, shebangs.
///
/// Immutable once registered — re-registration (`register_identity_no_rebuild`)
/// replaces the whole record rather than mutating it in place. The name is
/// not stored here: it lives once, in the registry's interner (SSOT).
#[derive(Debug, Default)]
pub struct LanguageIdentity {
    pub extensions: Vec<String>,
    /// Raw glob patterns (e.g. `"Makefile"`, `"*.{ts,tsx}"`). Stored for
    /// round-trip / debug; the compiled matcher lives on `LanguageRegistry`.
    pub globs: Vec<String>,
    /// Shebang substrings to match (e.g. `"python"`, `"node"`).
    pub shebangs: Vec<String>,
}

// ── GrammarBundle ─────────────────────────────────────────────────────────────

/// Tree-sitter grammar + precompiled highlight query, shared across all buffers
/// of a given language.
pub struct GrammarBundle {
    pub grammar: LoadedGrammar,
    /// Shared highlighter wrapping the compiled highlight query — one per
    /// language, not one per buffer (capture names are interned once at
    /// attach time).
    pub highlighter: Arc<TreeSitterHighlighter>,
    /// Compiled `injections.scm`, if the grammar has one. `None` means this
    /// language never injects embedded languages.
    pub injections: Option<InjectionsQuery>,
    /// Unique per attach, issued by `LanguageRegistry::next_gen`. Grammar-swap
    /// / staleness checks compare this instead of `Arc::ptr_eq` — a plain
    /// integer identity that survives across the worker-thread boundary.
    pub config_gen: u32,
}

// ── LanguageRegistry ──────────────────────────────────────────────────────────

/// Global registry of configured language identities. Lives on `Editor`.
///
/// `ids`/`names`/`identities`/`grammars` are the interner: dense, append-only,
/// all index-aligned by `LanguageId.0`. `identities.len() == grammars.len() ==
/// names.len()` always — `intern` pushes one entry to each.
pub struct LanguageRegistry {
    ids: FxHashMap<Arc<str>, LanguageId>,
    names: Vec<Arc<str>>,
    /// `None` = interned but no identity registered.
    identities: Vec<Option<LanguageIdentity>>,
    /// The `LanguageId -> GrammarBundle` map.
    grammars: Vec<Option<Arc<GrammarBundle>>>,
    /// Detection indices — never rebuilt on `attach_grammar`, only on
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
    /// Source of `GrammarBundle::config_gen` — incremented on every grammar
    /// attach so each attached bundle gets a unique identity.
    next_config_gen: u32,
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
            Self::GrammarLoad(e) => write!(f, "grammar load failed: {e:?}"),
            Self::HighlightsRead(e) => write!(f, "highlights.scm read failed: {e}"),
            Self::QueryBuild(e) => write!(f, "highlight query compilation failed: {e}"),
            Self::InjectionsRead(e) => write!(f, "injections.scm read failed: {e}"),
            Self::InjectionsQueryBuild(e) => write!(f, "injections query compilation failed: {e}"),
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
            names: Vec::new(),
            identities: Vec::new(),
            grammars: Vec::new(),
            by_ext: FxHashMap::default(),
            compiled_globs: GlobSet::empty(),
            glob_lang_ids: Vec::new(),
            shebang_to_id: FxHashMap::default(),
            lang_order: Vec::new(),
            grammar_snapshot: Arc::new(FxHashMap::default()),
            next_config_gen: 0,
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
        let id = LanguageId(self.names.len() as u32);
        // One allocation, shared via Arc — `names` and `ids` both need their
        // own owned key, but they can point at the same heap string instead
        // of each holding an independent copy.
        let owned: Arc<str> = Arc::from(name);
        self.names.push(Arc::clone(&owned));
        self.identities.push(None);
        self.grammars.push(None);
        self.ids.insert(owned, id);
        id
    }

    /// The id for `name`, if it has been interned.
    pub fn id_of(&self, name: &str) -> Option<LanguageId> {
        self.ids.get(name).copied()
    }

    /// The name `id` was interned with. `id` must have been minted by this
    /// registry — an out-of-range id is a caller bug.
    pub fn name_of(&self, id: LanguageId) -> &str {
        &self.names[id.0 as usize]
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
        globs: &[&str],
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
    /// grammar is kept — identity and grammar are independent facts about a
    /// language, and re-registering one must not silently undo the other.
    /// (Symmetric with `attach_grammar`, which likewise preserves an existing
    /// identity.) A grammar only ever changes via `attach_grammar`.
    pub fn register_identity_no_rebuild(
        &mut self,
        name: &str,
        extensions: &[&str],
        globs: &[&str],
        shebangs: &[&str],
    ) -> LanguageId {
        let id = self.intern(name);
        if let Some(old) = self.identities[id.0 as usize].take() {
            self.deindex(id, &old);
        }
        let new_identity = LanguageIdentity {
            extensions: extensions.iter().map(|s| s.to_string()).collect(),
            globs: globs.iter().map(|s| s.to_string()).collect(),
            shebangs: shebangs.iter().map(|s| s.to_string()).collect(),
        };
        for ext in &new_identity.extensions {
            self.by_ext.insert(ext.clone(), id);
        }
        for shebang in &new_identity.shebangs {
            self.shebang_to_id.insert(shebang.clone(), id);
        }
        self.identities[id.0 as usize] = Some(new_identity);
        self.lang_order.retain(|&i| i != id);
        self.lang_order.push(id);
        id
    }

    /// Remove `identity`'s entries from the `by_ext`/`shebang_to_id` secondary
    /// indices, but only where `id` is still the current owner — a shared
    /// extension (e.g. `.h` claimed by both `c` and `cpp`) may have been
    /// reassigned to a different language since `identity` was registered, and
    /// deindexing unconditionally would evict that newer owner's mapping.
    /// Shared by `register_identity_no_rebuild`'s replace-existing branch and
    /// `remove`. Note: this does not resurrect an older claimant if `id` was
    /// indeed still the owner — the extension simply becomes unclaimed, which
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
        let (compiled, ids) = Self::build_globs(&self.identities, &self.lang_order)
            .map_err(RegisterError::GlobBuild)?;
        self.compiled_globs = compiled;
        self.glob_lang_ids = ids;
        Ok(())
    }

    /// Look up a language by file extension (e.g. `"rs"`, `"C"`).
    pub fn by_extension(&self, ext: &str) -> Option<LanguageId> {
        self.by_ext.get(ext).copied()
    }

    /// Look up a language's identity by name (e.g. `"rust"`).
    pub fn by_name(&self, name: &str) -> Option<&LanguageIdentity> {
        let id = self.id_of(name)?;
        self.identities[id.0 as usize].as_ref()
    }

    /// Iterator over registered language names (those with an identity), in
    /// arbitrary order. Used by `:set buffer language=` completion.
    pub fn iter_names(&self) -> impl Iterator<Item = &str> {
        self.identities
            .iter()
            .enumerate()
            .filter_map(|(i, ident)| ident.as_ref().map(|_| &*self.names[i]))
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
    /// present. Also clears any attached grammar for the same id — a removed
    /// language has no detection and no grammar, matching `remove` deleting
    /// the language wholesale.
    #[cfg(any(test, feature = "test-util"))]
    pub fn remove(&mut self, name: &str) -> Option<LanguageIdentity> {
        let id = self.id_of(name)?;
        let identity = self.identities[id.0 as usize].take()?;
        self.deindex(id, &identity);
        self.grammars[id.0 as usize] = None;
        self.lang_order.retain(|&i| i != id);
        let (compiled, ids) =
            Self::build_globs(&self.identities, &self.lang_order).unwrap_or_else(|e| {
                eprintln!("LanguageRegistry::remove: glob rebuild failed: {e}");
                (GlobSet::empty(), Vec::new())
            });
        self.compiled_globs = compiled;
        self.glob_lang_ids = ids;
        self.rebuild_grammar_snapshot();
        Some(identity)
    }

    // ── Grammar ───────────────────────────────────────────────────────────────

    /// Attach a tree-sitter grammar to a language.
    ///
    /// Reads the highlights query file, compiles it, builds the shared
    /// highlighter (interning its capture names into `scope_reg`), optionally
    /// reads and compiles `injections_path` if given, then installs the
    /// resulting `GrammarBundle` for `name`'s id — detection indices
    /// (`by_ext`/globs/shebangs) are never touched.
    ///
    /// A broken `injections.scm` fails the whole attach, same as a broken
    /// `highlights.scm` — both come from the same trusted pinned source, so
    /// there is no separate soft-degrade path. All fallible work happens
    /// before any registry mutation, so a failed attach leaves no partial state.
    ///
    /// Auto-registers a bare identity (no extensions/globs/shebangs) if the
    /// language name has none yet.
    pub fn attach_grammar(
        &mut self,
        name: &str,
        grammar_path: &Path,
        symbol: &str,
        highlights_path: &Path,
        injections_path: Option<&Path>,
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
            std::fs::read_to_string(highlights_path).map_err(RegisterError::HighlightsRead)?;
        let query = Arc::new(
            tree_sitter::Query::new(grammar.language(), &highlights_src)
                .map_err(RegisterError::QueryBuild)?,
        );
        let highlighter = Arc::new(TreeSitterHighlighter::from_shared_query(query, scope_reg));
        let injections = injections_path
            .map(|path| {
                let src = std::fs::read_to_string(path).map_err(RegisterError::InjectionsRead)?;
                let query = Arc::new(
                    tree_sitter::Query::new(grammar.language(), &src)
                        .map_err(RegisterError::InjectionsQueryBuild)?,
                );
                Ok::<_, RegisterError>(InjectionsQuery::new(query))
            })
            .transpose()?;

        let id = self.intern(name);
        if self.identities[id.0 as usize].is_none() {
            self.identities[id.0 as usize] = Some(LanguageIdentity::default());
        }
        let config_gen = self.next_gen();
        let bundle = Arc::new(GrammarBundle {
            grammar,
            highlighter,
            injections,
            config_gen,
        });
        self.grammars[id.0 as usize] = Some(Arc::clone(&bundle));
        self.rebuild_grammar_snapshot();
        Ok(bundle)
    }

    /// Returns `true` if `name` has an attached tree-sitter grammar.
    pub fn has_grammar(&self, name: &str) -> bool {
        self.id_of(name)
            .is_some_and(|id| self.grammars[id.0 as usize].is_some())
    }

    /// The grammar bundle attached to `id`, if any.
    pub fn grammar(&self, id: LanguageId) -> Option<&Arc<GrammarBundle>> {
        self.grammars.get(id.0 as usize)?.as_ref()
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
            self.grammars
                .iter()
                .enumerate()
                .filter_map(|(i, g)| {
                    g.as_ref()
                        .map(|b| (self.names[i].to_string(), Arc::clone(b)))
                })
                .collect(),
        );
    }

    fn build_globs(
        identities: &[Option<LanguageIdentity>],
        lang_order: &[LanguageId],
    ) -> Result<(GlobSet, Vec<LanguageId>), globset::Error> {
        let mut builder = GlobSetBuilder::new();
        let mut ids = Vec::new();
        for &lang_id in lang_order {
            if let Some(Some(identity)) = identities.get(lang_id.0 as usize) {
                for pattern in &identity.globs {
                    let glob = globset::Glob::new(pattern)?;
                    builder.add(glob);
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
