use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use globset::{GlobSet, GlobSetBuilder};

use hume_engine::builtins::tree_sitter_hl::TreeSitterHighlighter;
use hume_engine::grammar::LoadedGrammar;
use hume_engine::theme::ScopeRegistry;

// ── GrammarBundle ─────────────────────────────────────────────────────────────

/// Tree-sitter grammar + precompiled highlight query, shared across all buffers
/// of a given language.
pub(crate) struct GrammarBundle {
    pub(crate) grammar: LoadedGrammar,
    pub(crate) query: Arc<tree_sitter::Query>,
}

// ── LanguageConfig ────────────────────────────────────────────────────────────

/// Language identity + optional grammar. Shared via `Arc`; rebuilt (new Arc)
/// when a grammar is attached via `attach_grammar`.
pub(crate) struct LanguageConfig {
    pub name: String,
    pub extensions: Vec<String>,
    /// Raw glob patterns (e.g. `"Makefile"`, `"*.{ts,tsx}"`). Stored for
    /// round-trip / debug; the compiled matcher lives on `LanguageRegistry`.
    pub globs: Vec<String>,
    /// Shebang substrings to match (e.g. `"python"`, `"node"`).
    pub shebangs: Vec<String>,
    /// Tree-sitter grammar + highlight query, `None` until `attach_grammar` is called.
    pub grammar: Option<GrammarBundle>,
}

// ── BufferSyntax ──────────────────────────────────────────────────────────────

/// Per-buffer syntax attachment state.
///
/// The `tree_sitter::Parser` lives on the parse worker thread.  This struct
/// tracks the grammar identity (for keepalive and grammar-swap detection via
/// `Arc::ptr_eq`) and the most recently installed tree generation.
///
/// The highlighter lives here (not in `SharedBuffer`) so there is a single
/// `Option<BufferSyntax>` attachment flag: `Some` means syntax is wired up,
/// `None` means it is not.  Moving it to the engine side would require the
/// engine crate to depend on editor-domain types (`LanguageConfig`), which
/// would invert the crate layering.  The renderer receives the highlighter via
/// the `get_syntax` closure injected into `EngineView::render`, parallel to
/// `get_rope`.
pub(crate) struct BufferSyntax {
    /// Keepalive: holds the Arc so the dlopen'd grammar is not unloaded while
    /// this buffer is syntax-attached.
    pub(crate) lang: Arc<LanguageConfig>,
    /// Compiled highlight query + capture-name mapping for this buffer's language.
    ///
    /// Owned here; panes receive a reference via the `get_syntax` closure passed
    /// to `EngineView::render` — no cloning, no shared ownership needed.
    pub(crate) highlighter: TreeSitterHighlighter,
    /// `text_gen` of the most recently installed tree.  When this equals
    /// `Buffer.text_gen`, the installed tree is up to date.
    pub(crate) parsed_gen: u64,
    /// Text generation whose coordinates the committed `sbuf.tree` currently
    /// describes.  Advanced each time pending edits are baked into the committed
    /// tree in `reparse_stale_buffers`, and on each precise parse install in
    /// `apply_parse_outcome`.  Separate from `parsed_gen` because edits can
    /// outpace the worker: `tree_gen` advances every frame (on bake), while
    /// `parsed_gen` advances only when the worker delivers a result.
    pub(crate) tree_gen: u64,
    /// Edits recorded since the last bake or installed tree, in order.
    ///
    /// Each entry is `(text_gen, edit)` where `text_gen` is the generation
    /// produced by the edit.  A contiguous chain from `tree_gen + 1` to the
    /// current `Buffer.text_gen` enables in-place baking of the committed tree;
    /// a gap triggers a full reparse.  Entries are cleared on each successful
    /// bake and drained (up to the installed gen) on each `apply_parse_outcome`.
    pub(crate) pending_edits: Vec<(u64, tree_sitter::InputEdit)>,
}

impl BufferSyntax {
    pub(crate) fn new(lang: Arc<LanguageConfig>, highlighter: TreeSitterHighlighter) -> Self {
        Self {
            lang,
            highlighter,
            parsed_gen: 0,
            tree_gen: 0,
            pending_edits: Vec::new(),
        }
    }
}

// ── LanguageRegistry ──────────────────────────────────────────────────────────

/// Global registry of configured language identities. Lives on `Editor`.
pub(crate) struct LanguageRegistry {
    by_name: HashMap<String, Arc<LanguageConfig>>,
    by_ext: HashMap<String, Arc<LanguageConfig>>,
    /// Compiled glob matcher, rebuilt whenever languages are added or removed.
    /// Index-aligned with `glob_lang_names`.
    compiled_globs: GlobSet,
    /// Language name for each glob pattern at the corresponding GlobSet index.
    glob_lang_names: Vec<String>,
    shebang_to_name: HashMap<String, String>,
    /// Registration order for glob priority: later entries win on overlap.
    lang_order: Vec<String>,
}

#[derive(Debug)]
pub(crate) enum RegisterError {
    /// The combined glob pattern set exceeded globset's NFA size limit.
    GlobBuild(globset::Error),
    /// Failed to open the grammar shared library.
    GrammarLoad(hume_engine::grammar::GrammarLoadError),
    /// Failed to read the highlights query file.
    HighlightsRead(std::io::Error),
    /// Failed to compile the highlights query.
    QueryBuild(tree_sitter::QueryError),
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
            by_name: HashMap::new(),
            by_ext: HashMap::new(),
            compiled_globs: GlobSet::empty(),
            glob_lang_names: Vec::new(),
            shebang_to_name: HashMap::new(),
            lang_order: Vec::new(),
        }
    }
}

impl LanguageRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Register a language identity: name, extensions, glob patterns, shebangs.
    ///
    /// Returns `Err(RegisterError::GlobBuild)` if the combined glob set would exceed
    /// globset's NFA size limit.
    ///
    /// For batch registration use `register_identity_no_rebuild` + `rebuild_glob_set`.
    #[cfg(test)]
    pub(crate) fn register_identity(
        &mut self,
        name: &str,
        extensions: &[&str],
        globs: &[&str],
        shebangs: &[&str],
    ) -> Result<Arc<LanguageConfig>, RegisterError> {
        let config = self.register_identity_no_rebuild(name, extensions, globs, shebangs);
        self.rebuild_glob_set()?;
        Ok(config)
    }

    /// Insert identity data without rebuilding the compiled glob set.
    ///
    /// Intended for batch registration: call this N times then call
    /// `rebuild_glob_set` once, avoiding O(N²) NFA constructions at startup.
    pub(crate) fn register_identity_no_rebuild(
        &mut self,
        name: &str,
        extensions: &[&str],
        globs: &[&str],
        shebangs: &[&str],
    ) -> Arc<LanguageConfig> {
        let config = Arc::new(LanguageConfig {
            name: name.to_owned(),
            extensions: extensions.iter().map(|s| s.to_string()).collect(),
            globs: globs.iter().map(|s| s.to_string()).collect(),
            shebangs: shebangs.iter().map(|s| s.to_string()).collect(),
            grammar: None,
        });
        self.lang_order.retain(|n| n.as_str() != name);
        self.lang_order.push(name.to_owned());
        if let Some(old) = self.by_name.remove(name) {
            for ext in &old.extensions {
                self.by_ext.remove(ext.as_str());
            }
            for shebang in &old.shebangs {
                self.shebang_to_name.remove(shebang.as_str());
            }
        }
        self.by_name.insert(name.to_owned(), Arc::clone(&config));
        for ext in &config.extensions {
            self.by_ext.insert(ext.clone(), Arc::clone(&config));
        }
        for shebang in &config.shebangs {
            self.shebang_to_name
                .insert(shebang.clone(), name.to_owned());
        }
        config
    }

    /// Rebuild the compiled glob set from current registry state.
    ///
    /// Returns `Err` if the NFA size limit is exceeded; on error the prior
    /// compiled set is preserved.
    pub(crate) fn rebuild_glob_set(&mut self) -> Result<(), RegisterError> {
        let (compiled, names) =
            Self::build_globs(&self.by_name, &self.lang_order).map_err(RegisterError::GlobBuild)?;
        self.compiled_globs = compiled;
        self.glob_lang_names = names;
        Ok(())
    }

    /// Look up a language by file extension (e.g. `"rs"`, `"C"`).
    pub(crate) fn by_extension(&self, ext: &str) -> Option<&Arc<LanguageConfig>> {
        self.by_ext.get(ext)
    }

    /// Look up a language by name (e.g. `"rust"`).
    pub(crate) fn by_name(&self, name: &str) -> Option<&Arc<LanguageConfig>> {
        self.by_name.get(name)
    }

    /// Iterator over registered language names, in arbitrary (HashMap) order.
    /// Used by `:set buffer language=` completion.
    pub(crate) fn iter_names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }

    /// The compiled glob matcher for path-based detection. Index-aligned with
    /// `glob_lang_name`: `glob_lang_name(i)` is the language for match index `i`.
    pub(crate) fn compiled_globs(&self) -> &GlobSet {
        &self.compiled_globs
    }

    /// Language name for glob match index `i` (from `GlobSet::matches`).
    pub(crate) fn glob_lang_name(&self, i: usize) -> Option<&str> {
        self.glob_lang_names.get(i).map(String::as_str)
    }

    /// Look up a language by shebang substring (e.g. `"python"`).
    pub(crate) fn by_shebang(&self, token: &str) -> Option<&Arc<LanguageConfig>> {
        self.shebang_to_name
            .get(token)
            .and_then(|name| self.by_name.get(name))
    }

    /// Remove a registered language by name, returning it if present.
    #[cfg(test)]
    pub(crate) fn remove(&mut self, name: &str) -> Option<Arc<LanguageConfig>> {
        let config = self.by_name.remove(name)?;
        for ext in &config.extensions {
            self.by_ext.remove(ext.as_str());
        }
        for shebang in &config.shebangs {
            self.shebang_to_name.remove(shebang.as_str());
        }
        let new_order: Vec<String> = self
            .lang_order
            .iter()
            .filter(|n| n.as_str() != name)
            .cloned()
            .collect();
        let (compiled, names) = Self::build_globs(&self.by_name, &new_order).unwrap_or_else(|e| {
            eprintln!("LanguageRegistry::remove: glob rebuild failed: {e}");
            (GlobSet::empty(), Vec::new())
        });
        self.lang_order = new_order;
        self.compiled_globs = compiled;
        self.glob_lang_names = names;
        Some(config)
    }

    /// Attach a tree-sitter grammar to a language.
    ///
    /// Reads the highlights query file, compiles it, interns all capture names
    /// into `scope_reg`, then replaces the `Arc<LanguageConfig>` in the registry
    /// so all subsequent `by_name`/`by_extension` lookups see the grammar.
    ///
    /// Auto-registers the identity (no extensions/globs/shebangs) if the language
    /// name is not already known.
    pub(crate) fn attach_grammar(
        &mut self,
        name: &str,
        grammar_path: &Path,
        symbol: &str,
        highlights_path: &Path,
        scope_reg: &mut ScopeRegistry,
    ) -> Result<Arc<LanguageConfig>, RegisterError> {
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
        for name_str in query.capture_names() {
            scope_reg.intern_runtime(name_str);
        }
        let existing = self.by_name.get(name);
        let new_config = Arc::new(LanguageConfig {
            name: name.to_owned(),
            extensions: existing.map_or_else(Vec::new, |c| c.extensions.clone()),
            globs: existing.map_or_else(Vec::new, |c| c.globs.clone()),
            shebangs: existing.map_or_else(Vec::new, |c| c.shebangs.clone()),
            grammar: Some(GrammarBundle { grammar, query }),
        });
        self.by_name
            .insert(name.to_owned(), Arc::clone(&new_config));
        for ext in &new_config.extensions {
            self.by_ext.insert(ext.clone(), Arc::clone(&new_config));
        }
        Ok(new_config)
    }

    /// Returns `true` if `name` has an attached tree-sitter grammar.
    pub(crate) fn has_grammar(&self, name: &str) -> bool {
        self.by_name.get(name).is_some_and(|c| c.grammar.is_some())
    }

    fn build_globs(
        by_name: &HashMap<String, Arc<LanguageConfig>>,
        lang_order: &[String],
    ) -> Result<(GlobSet, Vec<String>), globset::Error> {
        let mut builder = GlobSetBuilder::new();
        let mut names = Vec::new();
        for lang_name in lang_order {
            if let Some(config) = by_name.get(lang_name) {
                for pattern in &config.globs {
                    if let Ok(glob) = globset::Glob::new(pattern) {
                        builder.add(glob);
                        names.push(config.name.clone());
                    }
                }
            }
        }
        Ok((builder.build()?, names))
    }
}

// ── Language detection ────────────────────────────────────────────────────────

/// Detect the language for a buffer given its path and first line.
///
/// Priority: glob match (most-specific / last-registered) → file extension →
/// shebang. Returns the language name string, or `None` if unrecognised.
pub(crate) fn detect_language(
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
            && let Some(lang) = registry.by_extension(ext)
        {
            return Some(lang.name.clone());
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
        .map(|lang| lang.name.clone())
}

// ── Editor glue ───────────────────────────────────────────────────────────────

use hume_engine::pipeline::BufferId;
use steel::rvals::IntoSteelVal as _;

use super::Editor;
use hume_scripting::SteelBufferId;
use hume_scripting::hooks::HookId;

impl Editor {
    /// Set the language identity for buffer `bid`.
    ///
    /// No-op when the value is unchanged (avoids spurious hook fires).
    /// On change: writes `Buffer.language`, fires `OnLanguageSet` with `(bid, name-or-#f)`.
    /// All write paths (detection at open, `:set buffer language=`, Steel API) go
    /// through this function.
    pub(super) fn set_buffer_language(&mut self, bid: BufferId, new_lang: Option<String>) {
        if self.state.buffers.get(bid).language == new_lang {
            return;
        }
        let lang_val = match new_lang.as_deref() {
            Some(name) => name.into_steelval().expect("str into_steelval"),
            None => false.into_steelval().expect("bool into_steelval"),
        };
        // Clone before moving into the buffer so `activate_lazy_language_plugins`
        // can borrow the name after the write — a plugin reading buffer-language
        // during its own activation then sees the new value.
        let activate_name = new_lang.clone();
        self.state.buffers.get_mut(bid).language = new_lang;
        // Activate language-matched plugins after the write so handlers are
        // registered in time for the OnLanguageSet fire below.
        if let Some(name) = activate_name.as_deref() {
            self.activate_lazy_language_plugins(name);
        }
        let bid_val = SteelBufferId::new(bid).into_steel_val();
        self.fire_hook_silent(HookId::OnLanguageSet, &[bid_val, lang_val]);
        // Wire up (or tear down) tree-sitter highlighting for this buffer.
        self.setup_buffer_syntax(bid);
    }

    pub(super) fn detect_and_set_language(&mut self, bid: BufferId) {
        let detected = {
            let buf = self.state.buffers.get(bid);
            let path = buf.path().map(|p| p.to_path_buf());
            let first_line = buf.first_line();
            detect_language(
                path.as_deref(),
                first_line.as_deref(),
                &self.state.languages,
            )
        };
        self.set_buffer_language(bid, detected);
    }

    /// Register languages from a drained `pending_language_regs` vec.
    /// Fail-soft: glob-set build failures are logged as warnings, editor continues.
    pub(super) fn apply_pending_language_regs(
        &mut self,
        regs: Vec<hume_scripting::PendingLanguageReg>,
    ) {
        use hume_scripting::PendingLanguageReg;
        let mut any_identity = false;
        let mut grammar_sweeps: Vec<String> = Vec::new();
        for reg in regs {
            match reg {
                PendingLanguageReg::Identity {
                    name,
                    extensions,
                    globs,
                    shebangs,
                } => {
                    let exts: Vec<&str> = extensions.iter().map(String::as_str).collect();
                    let shebangs_ref: Vec<&str> = shebangs.iter().map(String::as_str).collect();
                    let mut valid_globs: Vec<&str> = Vec::with_capacity(globs.len());
                    for g in &globs {
                        match globset::Glob::new(g) {
                            Ok(_) => valid_globs.push(g.as_str()),
                            Err(e) => self.state.message_log.push(
                                super::Severity::Warning,
                                format!("define-language! '{}': invalid glob '{}': {}", name, g, e),
                            ),
                        }
                    }
                    self.state.languages.register_identity_no_rebuild(
                        &name,
                        &exts,
                        &valid_globs,
                        &shebangs_ref,
                    );
                    any_identity = true;
                }
                PendingLanguageReg::Grammar {
                    name,
                    grammar_path,
                    symbol,
                    highlights_path,
                } => {
                    match self.state.languages.attach_grammar(
                        &name,
                        &grammar_path,
                        &symbol,
                        &highlights_path,
                        &mut self.view.registry,
                    ) {
                        Ok(_) => grammar_sweeps.push(name),
                        Err(e) => self.state.message_log.push(
                            super::Severity::Warning,
                            format!("register-grammar! '{}': {}", name, e),
                        ),
                    }
                }
            }
        }
        if any_identity && let Err(e) = self.state.languages.rebuild_glob_set() {
            self.state.message_log.push(
                super::Severity::Warning,
                format!("define-language!: glob set build failed: {e}"),
            );
        }
        if !grammar_sweeps.is_empty() {
            self.sweep_buffers_for_grammars(grammar_sweeps);
        }
    }

    /// Drain `host.pending_language_regs` and apply them.
    pub(super) fn flush_pending_language_regs(&mut self, host: &mut hume_scripting::ScriptingHost) {
        let regs = host.take_pending_language_regs();
        self.apply_pending_language_regs(regs);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{LanguageRegistry, detect_language};

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
        let config = reg.by_name("toml").expect("identity should be registered");
        assert_eq!(config.name, "toml");
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
