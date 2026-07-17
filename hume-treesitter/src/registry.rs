use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use globset::{GlobSet, GlobSetBuilder};

use hume_engine::builtins::tree_sitter_hl::TreeSitterHighlighter;
use hume_engine::grammar::LoadedGrammar;
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

// ── BufferSyntax ──────────────────────────────────────────────────────────────

/// Per-buffer syntax attachment state.
///
/// The `tree_sitter::Parser` lives on the parse worker thread.  This struct
/// tracks the attached grammar bundle (for grammar-swap detection via
/// `GrammarBundle::config_gen`) and the most recently installed tree
/// generation.
///
/// The parsed trees and highlighters live engine-side, in
/// `SharedBuffer.syntax: Option<SyntaxLayers>` — both are already engine
/// types, so there's no crate-layering reason to keep them here. This struct
/// is the editor-domain half: a single `Option<BufferSyntax>` attachment
/// flag (`Some` means syntax is wired up), the attached grammar bundle, and
/// generation bookkeeping for incremental parsing.
pub struct BufferSyntax {
    /// The attached root grammar bundle.  Read when reposting an incremental
    /// reparse request and when checking whether the currently attached root
    /// grammar has an injections query (`sweep_buffers_for_grammars`).
    pub bundle: Arc<GrammarBundle>,
    /// `text_gen` of the most recently installed tree.  When this equals
    /// `Buffer.text_gen`, the installed tree is up to date.
    pub parsed_gen: u64,
    /// Text generation whose coordinates the committed `sbuf.syntax` layers
    /// currently describe.  Advanced each time pending edits are baked into
    /// the committed layers in `reparse_stale_buffers`, and on each precise
    /// parse install in `apply_parse_outcome`.  Separate from `parsed_gen`
    /// because edits can outpace the worker: `tree_gen` advances every frame
    /// (on bake), while `parsed_gen` advances only when the worker delivers a
    /// result.
    pub tree_gen: u64,
    /// Edits recorded since the last bake or installed tree, in order.
    ///
    /// Each entry is `(text_gen, edit)` where `text_gen` is the generation
    /// produced by the edit.  A contiguous chain from `tree_gen + 1` to the
    /// current `Buffer.text_gen` enables in-place baking of the committed tree;
    /// a gap triggers a full reparse.  Entries are cleared on each successful
    /// bake and drained (up to the installed gen) on each `apply_parse_outcome`.
    pub pending_edits: Vec<(u64, tree_sitter::InputEdit)>,
}

impl BufferSyntax {
    pub fn new(bundle: Arc<GrammarBundle>) -> Self {
        Self {
            bundle,
            parsed_gen: 0,
            tree_gen: 0,
            pending_edits: Vec::new(),
        }
    }
}

// ── LanguageRegistry ──────────────────────────────────────────────────────────

/// Global registry of configured language identities. Lives on `Editor`.
///
/// `ids`/`names`/`identities`/`grammars` are the interner: dense, append-only,
/// all index-aligned by `LanguageId.0`. `identities.len() == grammars.len() ==
/// names.len()` always — `intern` pushes one entry to each.
pub struct LanguageRegistry {
    ids: HashMap<String, LanguageId>,
    names: Vec<String>,
    /// `None` = interned but no identity registered.
    identities: Vec<Option<LanguageIdentity>>,
    /// The `LanguageId -> GrammarBundle` map.
    grammars: Vec<Option<Arc<GrammarBundle>>>,
    /// Detection indices — never rebuilt on `attach_grammar`, only on
    /// identity (re-)registration or removal.
    by_ext: HashMap<String, LanguageId>,
    /// Compiled glob matcher, rebuilt whenever languages are added or removed.
    /// Index-aligned with `glob_lang_ids`.
    compiled_globs: GlobSet,
    /// Language id for each glob pattern at the corresponding GlobSet index.
    glob_lang_ids: Vec<LanguageId>,
    shebang_to_id: HashMap<String, LanguageId>,
    /// Registration order for glob priority: later entries win on overlap.
    lang_order: Vec<LanguageId>,
    /// Grammared languages only, rebuilt whenever the grammar table changes.
    /// Handed to the parse worker so it can resolve an injected language name
    /// (a dynamically-discovered info-string) to its grammar without
    /// touching main-thread state.
    grammar_snapshot: Arc<HashMap<String, Arc<GrammarBundle>>>,
    /// Source of `GrammarBundle::config_gen` — incremented on every grammar
    /// attach so each attached bundle gets a unique identity.
    next_config_gen: u32,
}

#[derive(Debug)]
pub enum RegisterError {
    /// The combined glob pattern set exceeded globset's NFA size limit.
    GlobBuild(globset::Error),
    /// Failed to open the grammar shared library.
    GrammarLoad(hume_engine::grammar::GrammarLoadError),
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
            ids: HashMap::new(),
            names: Vec::new(),
            identities: Vec::new(),
            grammars: Vec::new(),
            by_ext: HashMap::new(),
            compiled_globs: GlobSet::empty(),
            glob_lang_ids: Vec::new(),
            shebang_to_id: HashMap::new(),
            lang_order: Vec::new(),
            grammar_snapshot: Arc::new(HashMap::new()),
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
        self.names.push(name.to_owned());
        self.identities.push(None);
        self.grammars.push(None);
        self.ids.insert(name.to_owned(), id);
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
    /// Re-registering an already-grammared name drops its grammar (matching
    /// `by_name`/`has_grammar` seeing no identity change without a fresh
    /// `attach_grammar`) and rebuilds the snapshot so no stale entry survives.
    pub fn register_identity_no_rebuild(
        &mut self,
        name: &str,
        extensions: &[&str],
        globs: &[&str],
        shebangs: &[&str],
    ) -> LanguageId {
        let id = self.intern(name);
        if let Some(old) = self.identities[id.0 as usize].take() {
            self.deindex(&old);
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
        if self.grammars[id.0 as usize].take().is_some() {
            self.rebuild_grammar_snapshot();
        }
        id
    }

    /// Remove `identity`'s entries from the `by_ext`/`shebang_to_id` secondary
    /// indices. Shared by `register_identity_no_rebuild`'s replace-existing
    /// branch and `remove`.
    fn deindex(&mut self, identity: &LanguageIdentity) {
        for ext in &identity.extensions {
            self.by_ext.remove(ext.as_str());
        }
        for shebang in &identity.shebangs {
            self.shebang_to_id.remove(shebang.as_str());
        }
    }

    /// Rebuild the compiled glob set from current registry state.
    ///
    /// Returns `Err` if the NFA size limit is exceeded; on error the prior
    /// compiled set is preserved.
    pub fn rebuild_glob_set(&mut self) -> Result<(), RegisterError> {
        let (compiled, ids) =
            Self::build_globs(&self.identities, &self.lang_order).map_err(RegisterError::GlobBuild)?;
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
            .filter_map(|(i, ident)| ident.as_ref().map(|_| self.names[i].as_str()))
    }

    /// The compiled glob matcher for path-based detection. Index-aligned with
    /// `glob_lang_name`: `glob_lang_name(i)` is the language for match index `i`.
    pub fn compiled_globs(&self) -> &GlobSet {
        &self.compiled_globs
    }

    /// Language name for glob match index `i` (from `GlobSet::matches`).
    pub fn glob_lang_name(&self, i: usize) -> Option<&str> {
        self.glob_lang_ids.get(i).map(|&id| self.name_of(id))
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
        self.deindex(&identity);
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

    /// The grammar bundle attached to `name`, if any.
    pub fn grammar_by_name(&self, name: &str) -> Option<&Arc<GrammarBundle>> {
        let id = self.id_of(name)?;
        self.grammar(id)
    }

    /// Snapshot of grammared languages, keyed by name. Handed to the parse
    /// worker so it can resolve an injection language name (a
    /// dynamically-discovered info string) to its grammar without touching
    /// main-thread state.
    pub fn grammar_snapshot(&self) -> Arc<HashMap<String, Arc<GrammarBundle>>> {
        Arc::clone(&self.grammar_snapshot)
    }

    fn rebuild_grammar_snapshot(&mut self) {
        self.grammar_snapshot = Arc::new(
            self.grammars
                .iter()
                .enumerate()
                .filter_map(|(i, g)| g.as_ref().map(|b| (self.names[i].clone(), Arc::clone(b))))
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
                    if let Ok(glob) = globset::Glob::new(pattern) {
                        builder.add(glob);
                        ids.push(lang_id);
                    }
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
/// shebang. Returns the language name string, or `None` if unrecognised.
pub fn detect_language(
    path: Option<&std::path::Path>,
    first_line: Option<&str>,
    registry: &LanguageRegistry,
) -> Option<String> {
    if let Some(path) = path {
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(std::path::Path::new);
        if let Some(name_path) = file_name {
            let matches = registry.compiled_globs().matches(name_path);
            if let Some(&last_idx) = matches.last()
                && let Some(name) = registry.glob_lang_name(last_idx)
            {
                return Some(name.to_owned());
            }
        }

        if let Some(ext) = path.extension().and_then(|e| e.to_str())
            && let Some(id) = registry.by_extension(ext)
        {
            return Some(registry.name_of(id).to_owned());
        }
    }

    if let Some(line) = first_line
        && let Some(name) = detect_shebang(line, registry)
    {
        return Some(name);
    }

    None
}

fn detect_shebang(line: &str, registry: &LanguageRegistry) -> Option<String> {
    let after_bang = line.strip_prefix("#!")?;
    let mut tokens = after_bang.split_whitespace();
    let interpreter_path = tokens.next()?;

    let interpreter = if interpreter_path.ends_with("/env")
        || interpreter_path == "env"
        || interpreter_path == "/usr/bin/env"
    {
        tokens.find(|t| !t.starts_with('-'))?
    } else {
        std::path::Path::new(interpreter_path)
            .file_name()?
            .to_str()?
    };

    registry
        .by_shebang(interpreter)
        .map(|id| registry.name_of(id).to_owned())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{LanguageRegistry, detect_language};
    use crate::test_support::{grammar_parser_path, grammar_query_path};
    use hume_engine::theme::ScopeRegistry;

    /// Write `src` to a temp file and return its path (kept alive via the
    /// returned `TempDir`).
    fn write_temp_scm(src: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("injections.scm");
        std::fs::write(&path, src).unwrap();
        (dir, path)
    }

    #[test]
    fn attach_grammar_with_valid_injections_populates_bundle() {
        let parser_path = grammar_parser_path("rust");
        if !parser_path.exists() {
            return; // fixture not fetched — scripts/fetch-test-grammars.sh
        }
        let hl_path = grammar_query_path("rust");
        let (_dir, inj_path) =
            write_temp_scm(r#"((_) @injection.content (#set! injection.language "markdown"))"#);

        let mut reg = LanguageRegistry::new();
        let mut scope_reg = ScopeRegistry::new();
        let bundle = reg
            .attach_grammar(
                "rust",
                &parser_path,
                "tree_sitter_rust",
                &hl_path,
                Some(&inj_path),
                &mut scope_reg,
            )
            .expect("attach with valid injections must succeed");

        let injections = bundle
            .injections
            .as_ref()
            .expect("injections query must be populated");
        assert!(
            injections.content_capture.is_some(),
            "injection.content capture must be found"
        );
        assert_eq!(injections.patterns.len(), 1);
        assert_eq!(
            injections.patterns[0].language.as_deref(),
            Some("markdown"),
            "static #set! injection.language must be captured"
        );
    }

    #[test]
    fn attach_grammar_without_injections_path_leaves_injections_none() {
        let parser_path = grammar_parser_path("rust");
        if !parser_path.exists() {
            return;
        }
        let hl_path = grammar_query_path("rust");

        let mut reg = LanguageRegistry::new();
        let mut scope_reg = ScopeRegistry::new();
        let bundle = reg
            .attach_grammar(
                "rust",
                &parser_path,
                "tree_sitter_rust",
                &hl_path,
                None,
                &mut scope_reg,
            )
            .expect("attach without injections must succeed");

        assert!(
            bundle.injections.is_none(),
            "no injections_path given → injections must stay None"
        );
    }

    /// A broken injections.scm hard-fails the whole attach — same as a broken
    /// highlights.scm — rather than degrading to a warning. Both files come
    /// from the same trusted pinned source, so there is no separate soft-fail
    /// path.
    #[test]
    fn attach_grammar_with_broken_injections_fails_whole_attach() {
        let parser_path = grammar_parser_path("rust");
        if !parser_path.exists() {
            return;
        }
        let hl_path = grammar_query_path("rust");
        let (_dir, inj_path) = write_temp_scm("(this is not valid tree-sitter query syntax");

        let mut reg = LanguageRegistry::new();
        let mut scope_reg = ScopeRegistry::new();
        let result = reg.attach_grammar(
            "rust",
            &parser_path,
            "tree_sitter_rust",
            &hl_path,
            Some(&inj_path),
            &mut scope_reg,
        );
        let Err(err) = result else {
            panic!("broken injections.scm must fail the attach");
        };
        assert!(
            matches!(err, super::RegisterError::InjectionsQueryBuild(_)),
            "broken injections.scm must surface as InjectionsQueryBuild, got: {err:?}"
        );
        assert!(
            reg.by_name("rust").is_none(),
            "a failed attach must not leave a partial identity behind"
        );
    }

    #[test]
    fn language_registry_by_ext_lookup_empty() {
        let reg = LanguageRegistry::new();
        assert!(reg.by_extension("rs").is_none());
    }

    #[test]
    fn language_registry_remove_is_idempotent() {
        let mut reg = LanguageRegistry::new();
        assert!(reg.remove("rust").is_none());
    }

    #[test]
    fn register_identity_then_by_name_returns_entry() {
        let mut reg = LanguageRegistry::new();
        reg.register_identity("toml", &["toml"], &[], &[]).unwrap();
        assert!(
            reg.by_name("toml").is_some(),
            "identity should be registered"
        );
        let id = reg.id_of("toml").expect("toml must be interned");
        assert_eq!(reg.name_of(id), "toml");
        assert!(
            reg.by_extension("toml").is_some(),
            "extension lookup must work after identity reg"
        );
        // Flip: unknown ext should not match.
        assert!(reg.by_extension("yaml").is_none());
    }

    #[test]
    fn register_identity_with_globs_lookup() {
        let mut reg = LanguageRegistry::new();
        reg.register_identity("makefile", &[], &["Makefile", "GNUmakefile"], &[])
            .unwrap();
        let matches = reg.compiled_globs().matches(Path::new("Makefile"));
        assert!(!matches.is_empty(), "Makefile should match registered glob");
        assert_eq!(reg.glob_lang_name(matches[0]), Some("makefile"));
        // Flip: non-matching path must produce empty match.
        assert!(
            reg.compiled_globs()
                .matches(Path::new("Cargo.toml"))
                .is_empty()
        );
    }

    #[test]
    fn remove_clears_glob_and_shebang_entries() {
        let mut reg = LanguageRegistry::new();
        reg.register_identity("python", &["py"], &["*.py"], &["python"])
            .unwrap();
        assert!(reg.by_extension("py").is_some());
        assert!(reg.by_shebang("python").is_some());
        assert!(!reg.compiled_globs().matches(Path::new("foo.py")).is_empty());

        reg.remove("python");

        assert!(reg.by_extension("py").is_none());
        assert!(reg.by_shebang("python").is_none());
        // Flip expectation: after remove, matches must be empty.
        assert!(reg.compiled_globs().matches(Path::new("foo.py")).is_empty());
    }

    #[test]
    fn detect_language_by_extension() {
        let mut reg = LanguageRegistry::new();
        reg.register_identity("rust", &["rs"], &[], &[]).unwrap();
        let name = detect_language(Some(Path::new("foo.rs")), None, &reg);
        assert_eq!(name.as_deref(), Some("rust"));
        // Flip: wrong extension must not detect.
        let no_match = detect_language(Some(Path::new("foo.py")), None, &reg);
        assert!(no_match.is_none());
    }

    #[test]
    fn detect_language_by_glob() {
        let mut reg = LanguageRegistry::new();
        reg.register_identity("makefile", &[], &["Makefile", "GNUmakefile"], &[])
            .unwrap();
        let name = detect_language(Some(Path::new("/project/Makefile")), None, &reg);
        assert_eq!(name.as_deref(), Some("makefile"));
        let no_match = detect_language(Some(Path::new("/project/other")), None, &reg);
        assert!(no_match.is_none());
    }

    #[test]
    fn detect_language_glob_beats_extension() {
        let mut reg = LanguageRegistry::new();
        reg.register_identity("typescript", &["ts"], &[], &[])
            .unwrap();
        reg.register_identity("tsconfig", &[], &["tsconfig.json", "*.config.json"], &[])
            .unwrap();
        reg.register_identity("json", &["json"], &[], &[]).unwrap();
        let name = detect_language(Some(Path::new("tsconfig.json")), None, &reg);
        assert_eq!(name.as_deref(), Some("tsconfig"));
        // Flip: without the glob match, a plain .json should detect as json.
        let plain = detect_language(Some(Path::new("other.json")), None, &reg);
        assert_eq!(plain.as_deref(), Some("json"));
    }

    #[test]
    fn detect_language_glob_tiebreak_last_registered_wins() {
        let mut reg = LanguageRegistry::new();
        reg.register_identity("generic-json", &[], &["*.json"], &[])
            .unwrap();
        reg.register_identity("strict-json", &[], &["*.json"], &[])
            .unwrap();
        assert_eq!(
            detect_language(Some(Path::new("config.json")), None, &reg).as_deref(),
            Some("strict-json"),
        );

        let mut reg2 = LanguageRegistry::new();
        reg2.register_identity("strict-json", &[], &["*.json"], &[])
            .unwrap();
        reg2.register_identity("generic-json", &[], &["*.json"], &[])
            .unwrap();
        assert_eq!(
            detect_language(Some(Path::new("config.json")), None, &reg2).as_deref(),
            Some("generic-json"),
        );
    }

    #[test]
    fn detect_language_by_shebang() {
        let mut reg = LanguageRegistry::new();
        reg.register_identity("python", &["py"], &[], &["python3", "python"])
            .unwrap();
        let name = detect_language(
            Some(Path::new("script")),
            Some("#!/usr/bin/env python3"),
            &reg,
        );
        assert_eq!(name.as_deref(), Some("python"));
        // Flip: wrong shebang must not match.
        let no_match = detect_language(Some(Path::new("script")), Some("#!/bin/bash"), &reg);
        assert!(no_match.is_none());
    }

    #[test]
    fn detect_language_shebang_direct_path() {
        let mut reg = LanguageRegistry::new();
        reg.register_identity("bash", &["sh"], &[], &["bash"])
            .unwrap();
        // Extension wins over shebang.
        let name = detect_language(Some(Path::new("run.sh")), Some("#!/bin/bash"), &reg);
        assert_eq!(name.as_deref(), Some("bash"));
        // Without extension, shebang is used.
        let name2 = detect_language(Some(Path::new("run")), Some("#!/bin/bash"), &reg);
        assert_eq!(name2.as_deref(), Some("bash"));
    }

    #[test]
    fn detect_language_no_match() {
        let reg = LanguageRegistry::new();
        assert!(detect_language(Some(Path::new("foo.xyz")), None, &reg).is_none());
        assert!(detect_language(None, None, &reg).is_none());
    }

    /// Extensions are matched case-sensitively, so `"c"` and `"C"` map to
    /// distinct languages — `foo.c` detects as `c`, `foo.C` detects as `cpp`.
    ///
    /// Flip: if extensions were folded to lowercase both would map to the
    /// later-registered language (cpp wins, .c misdetects as cpp).
    #[test]
    fn extension_matching_is_case_sensitive() {
        let mut reg = LanguageRegistry::new();
        reg.register_identity("c", &["c"], &[], &[]).unwrap();
        reg.register_identity("cpp", &["C"], &[], &[]).unwrap();
        assert_eq!(
            detect_language(Some(Path::new("foo.c")), None, &reg).as_deref(),
            Some("c"),
        );
        assert_eq!(
            detect_language(Some(Path::new("foo.C")), None, &reg).as_deref(),
            Some("cpp"),
        );
        // Sanity: unrelated extension is still None.
        assert!(detect_language(Some(Path::new("foo.rs")), None, &reg).is_none());
    }
}
