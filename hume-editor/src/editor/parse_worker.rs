use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;

use hume_engine::pipeline::BufferId;

use super::injections::resolve_and_parse_injections;
use super::syntax::LanguageConfig;
use hume_editing::text::Text;

// Compile-time Send assertions — tree_sitter::Tree is Send+Sync;
// tree_sitter::Parser is Send+!Sync (lives on the worker thread only).
const _: fn() = || {
    fn _assert_send<T: Send>() {}
    _assert_send::<ParseRequest>();
    _assert_send::<ParseDone>();
    _assert_send::<ParseOutcome>();
};

/// Nesting cap for recursive injections (root = depth 0). Covers the deepest
/// realistic case — markdown → rust → rustdoc comment → markdown — without
/// letting a pathological grammar recurse unboundedly.
pub(super) const MAX_INJECTION_DEPTH: u8 = 3;

// ── Messages ──────────────────────────────────────────────────────────────────

pub(super) struct ParseRequest {
    pub(super) bid: BufferId,
    pub(super) text_gen: u64,
    /// Keepalive: holds the Arc so the dlopen'd grammar is not unloaded while
    /// the worker holds this request.
    pub(super) lang: Arc<LanguageConfig>,
    /// O(1) rope clone (structural sharing) — serialised to bytes on the worker
    /// thread only when the parse succeeds, avoiding the main-thread allocation.
    pub(super) text: Text,
    /// Previous parse tree with all pending `InputEdit`s applied, enabling
    /// incremental re-parsing.  `None` for a full reparse (first parse, grammar
    /// swap, or broken edit chain).
    pub(super) old_tree: Option<tree_sitter::Tree>,
    /// Snapshot of every grammared language, for resolving an injected
    /// language name (e.g. a fenced code block's info string) to its grammar
    /// without touching main-thread state from the worker.
    pub(super) langs: Arc<HashMap<String, Arc<LanguageConfig>>>,
}

pub(super) enum ParseOutcome {
    /// Root parse succeeded.  Carries the root tree plus every embedded-language
    /// layer resolved from it; byte text is read from the live rope at render
    /// time via `RopeProvider` in the engine highlighter.
    Ok(ParsedLayers),
    /// `Parser::parse` returned `None` (transient; currently unreachable without
    /// a cancellation / timeout).  Syntax stays attached; the next frame retries.
    ParseFailed,
}

/// The root parse tree plus every injected layer resolved from it.
pub(super) struct ParsedLayers {
    pub(super) root: tree_sitter::Tree,
    pub(super) injected: Vec<ParsedInjection>,
}

/// One resolved and parsed injection layer.
pub(super) struct ParsedInjection {
    /// Keepalive + identity for the injected layer's grammar.
    pub(super) lang: Arc<LanguageConfig>,
    pub(super) tree: tree_sitter::Tree,
    /// Absolute byte ranges this layer was parsed over, sorted by start.
    pub(super) ranges: Vec<tree_sitter::Range>,
    pub(super) depth: u8,
}

pub(super) struct ParseDone {
    pub(super) bid: BufferId,
    pub(super) text_gen: u64,
    /// Identity token — compared via `Arc::ptr_eq` on the main thread to detect
    /// grammar swaps that occurred while the request was in flight.
    pub(super) lang: Arc<LanguageConfig>,
    pub(super) outcome: ParseOutcome,
}

// ── Coalescing helper ─────────────────────────────────────────────────────────

/// Insert `req` into `batch`, keeping only the highest-`text_gen` entry per `bid`.
/// Older or equal-gen duplicates are dropped without cloning.
fn coalesce_one(batch: &mut HashMap<BufferId, ParseRequest>, req: ParseRequest) {
    use std::collections::hash_map::Entry;
    match batch.entry(req.bid) {
        Entry::Vacant(v) => {
            v.insert(req);
        }
        Entry::Occupied(mut o) => {
            // Keep the latest generation; when generations are equal, a different
            // grammar Arc means a grammar swap on a quiescent buffer — take the
            // new entry so the fresh grammar wins even without a text edit.
            if req.text_gen > o.get().text_gen
                || (req.text_gen == o.get().text_gen && !Arc::ptr_eq(&req.lang, &o.get().lang))
            {
                o.insert(req);
            }
            // else: drop req — older or same-grammar equal-gen duplicate.
        }
    }
}

// ── Shared parse logic ────────────────────────────────────────────────────────

/// Parse `rope` with `parser` (already configured: language + included
/// ranges), honoring `cancel`. Feeds the rope via a chunked callback — avoids
/// a full `Vec<u8>` allocation. `chunk_at_byte` returns a &str slice directly
/// into the rope's immutable B-tree nodes.
///
/// Shared by the root parse and every injected-layer parse — injected layers
/// always pass `old_tree: None` (Phase 3 decision: only the root is
/// incremental; injected regions are typically small enough that a full
/// parse is cheap and avoids tracking per-layer identity across edits).
pub(super) fn run_parse(
    parser: &mut tree_sitter::Parser,
    rope: &ropey::Rope,
    old_tree: Option<&tree_sitter::Tree>,
    cancel: &AtomicBool,
) -> Option<tree_sitter::Tree> {
    use std::ops::ControlFlow;
    let mut progress = |_state: &tree_sitter::ParseState| -> ControlFlow<()> {
        if cancel.load(Ordering::Relaxed) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let options = tree_sitter::ParseOptions::new().progress_callback(&mut progress);
    let len_bytes = rope.len_bytes();
    parser.parse_with_options(
        &mut |byte_offset, _| {
            if byte_offset >= len_bytes {
                return b"" as &[u8];
            }
            let (chunk, chunk_byte_start, _, _) = rope.chunk_at_byte(byte_offset);
            &chunk.as_bytes()[byte_offset - chunk_byte_start..]
        },
        old_tree,
        Some(options),
    )
}

/// Execute a parse request synchronously, returning the finished `ParseDone`.
/// Used by both `WorkerState` and `InlineParseBackend` (the latter in tests).
///
/// Always calls `set_language` and resets `included_ranges` to whole-buffer
/// before the root parse — layer parsing switches languages and ranges
/// constantly, so a "current language" cache (as a single-tree parser had)
/// would just thrash on every request.
fn do_parse(parser: &mut tree_sitter::Parser, req: ParseRequest, cancel: &AtomicBool) -> ParseDone {
    let bundle = req.lang.grammar.as_ref().expect(
        "grammar must be Some — setup_buffer_syntax verifies grammar.is_some() before posting",
    );
    parser
        .set_language(bundle.grammar.language())
        .expect("ABI verified at grammar registration time in attach_grammar");
    parser
        .set_included_ranges(&[])
        .expect("empty ranges are always valid — whole-buffer parse");

    let rope = req.text.rope();
    let outcome = match run_parse(parser, rope, req.old_tree.as_ref(), cancel) {
        Some(root) => {
            let injected =
                resolve_and_parse_injections(parser, &root, &req.lang, rope, &req.langs, cancel, 1);
            ParseOutcome::Ok(ParsedLayers { root, injected })
        }
        None => ParseOutcome::ParseFailed,
    };

    ParseDone {
        bid: req.bid,
        text_gen: req.text_gen,
        lang: req.lang,
        outcome,
    }
}

// ── Worker internals (stays on the worker thread) ─────────────────────────────

struct WorkerState {
    rx: mpsc::Receiver<ParseRequest>,
    tx: mpsc::Sender<ParseDone>,
    parser: tree_sitter::Parser,
    cancel: Arc<AtomicBool>,
}

impl WorkerState {
    fn run(mut self) {
        loop {
            // Block until at least one request arrives; exit when the main
            // thread closes the channel (editor shutting down).
            let first = match self.rx.recv() {
                Ok(r) => r,
                Err(_) => return,
            };

            // Coalesce: drain any additional queued requests, keeping only the
            // highest-text_gen request per BufferId.  Superseded requests are
            // already obsolete and would produce trees the main thread discards.
            let mut batch: HashMap<BufferId, ParseRequest> = HashMap::new();
            coalesce_one(&mut batch, first);
            let disconnected = 'drain: loop {
                match self.rx.try_recv() {
                    Ok(r) => coalesce_one(&mut batch, r),
                    Err(mpsc::TryRecvError::Empty) => break 'drain false,
                    Err(mpsc::TryRecvError::Disconnected) => break 'drain true,
                }
            };

            for (_, req) in batch {
                let done = do_parse(&mut self.parser, req, &self.cancel);
                if self.tx.send(done).is_err() {
                    return;
                }
            }

            if disconnected {
                return;
            }
        }
    }
}

// ── ParseBackend trait ────────────────────────────────────────────────────────

/// Abstraction over the parse backend so tests can inject a synchronous
/// implementation that avoids threads and blocking.
pub(super) trait ParseBackend {
    fn post(&mut self, req: ParseRequest);

    /// Drain all available parse results.  Must be called before
    /// `is_in_flight` or other state queries on the same frame.
    fn drain_done(&mut self) -> Vec<ParseDone>;

    /// Whether `bid` has an in-flight request matching `text_gen`.
    fn is_in_flight(&self, bid: BufferId, text_gen: u64) -> bool;

    /// Unconditionally remove the in-flight record for `bid`.
    fn remove_in_flight(&mut self, bid: BufferId);

    /// Remove the in-flight record for `bid` only when both `text_gen` and
    /// grammar identity match.  Avoids clearing a newer in-flight entry when
    /// a stale `ParseDone` arrives for the same buffer.
    fn clear_in_flight_if_matches(
        &mut self,
        bid: BufferId,
        text_gen: u64,
        lang: &Arc<LanguageConfig>,
    );

    /// True if any parse request is currently in flight.
    fn has_in_flight(&self) -> bool;

    fn is_disconnected(&self) -> bool;
}

// ── InFlight record ───────────────────────────────────────────────────────────

/// Records a pending parse request so `reparse_stale_buffers` can avoid
/// re-submitting for a buffer whose request is already in flight.
struct InFlight {
    text_gen: u64,
    /// Grammar identity at submission time — used by `clear_in_flight_if_matches`
    /// to avoid evicting a newer in-flight entry when a stale result arrives.
    lang: Arc<LanguageConfig>,
}

// ── ThreadedParseBackend (production) ─────────────────────────────────────────

pub(super) struct ThreadedParseBackend {
    /// `None` after `Drop` closes the channel to signal the worker.
    tx_req: Option<mpsc::Sender<ParseRequest>>,
    rx_done: mpsc::Receiver<ParseDone>,
    in_flight: HashMap<BufferId, InFlight>,
    disconnected: bool,
    cancel: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl ThreadedParseBackend {
    pub(super) fn new() -> Self {
        let (tx_req, rx_req) = mpsc::channel::<ParseRequest>();
        let (tx_done, rx_done) = mpsc::channel::<ParseDone>();
        let cancel = Arc::new(AtomicBool::new(false));

        let state = WorkerState {
            rx: rx_req,
            tx: tx_done,
            parser: tree_sitter::Parser::new(),
            cancel: Arc::clone(&cancel),
        };

        let thread = thread::Builder::new()
            .name("hume-parse-worker".into())
            .spawn(move || state.run())
            .expect("failed to spawn parse worker thread");

        Self {
            tx_req: Some(tx_req),
            rx_done,
            in_flight: HashMap::new(),
            disconnected: false,
            cancel,
            thread: Some(thread),
        }
    }
}

impl ParseBackend for ThreadedParseBackend {
    fn post(&mut self, req: ParseRequest) {
        if self.disconnected {
            return;
        }
        if let Some(ref tx) = self.tx_req {
            let bid = req.bid;
            let inf = InFlight {
                text_gen: req.text_gen,
                lang: Arc::clone(&req.lang),
            };
            match tx.send(req) {
                Ok(()) => {
                    self.in_flight.insert(bid, inf);
                }
                Err(_) => {
                    self.disconnected = true;
                    self.in_flight.clear();
                }
            }
        }
    }

    fn drain_done(&mut self) -> Vec<ParseDone> {
        let mut out = Vec::new();
        loop {
            match self.rx_done.try_recv() {
                Ok(done) => out.push(done),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.disconnected = true;
                    self.in_flight.clear();
                    break;
                }
            }
        }
        out
    }

    fn is_in_flight(&self, bid: BufferId, text_gen: u64) -> bool {
        self.in_flight
            .get(&bid)
            .is_some_and(|inf| inf.text_gen == text_gen)
    }

    fn remove_in_flight(&mut self, bid: BufferId) {
        self.in_flight.remove(&bid);
    }

    fn clear_in_flight_if_matches(
        &mut self,
        bid: BufferId,
        text_gen: u64,
        lang: &Arc<LanguageConfig>,
    ) {
        if let Some(inf) = self.in_flight.get(&bid)
            && inf.text_gen == text_gen
            && Arc::ptr_eq(&inf.lang, lang)
        {
            self.in_flight.remove(&bid);
        }
    }

    fn has_in_flight(&self) -> bool {
        !self.in_flight.is_empty()
    }

    fn is_disconnected(&self) -> bool {
        self.disconnected
    }
}

impl Drop for ThreadedParseBackend {
    fn drop(&mut self) {
        // Signal cancellation first so an in-progress parse exits at the next
        // progress-callback tick rather than blocking shutdown.
        self.cancel.store(true, Ordering::Release);
        // Closing tx_req causes the worker's rx.recv() to return Err.
        self.tx_req = None;
        if let Some(jh) = self.thread.take() {
            let _ = jh.join();
        }
    }
}

// ── InlineParseBackend (tests only) ──────────────────────────────────────────

/// Synchronous parse backend for tests.  `post` runs the parse immediately and
/// queues the result; `drain_done` flushes the queue.  No threads, no channels,
/// no waiting — tests call `reparse_stale_buffers` instead of blocking helpers.
#[cfg(test)]
pub(super) struct InlineParseBackend {
    parser: tree_sitter::Parser,
    done: std::collections::VecDeque<ParseDone>,
}

#[cfg(test)]
impl InlineParseBackend {
    pub(super) fn new() -> Self {
        Self {
            parser: tree_sitter::Parser::new(),
            done: std::collections::VecDeque::new(),
        }
    }
}

#[cfg(test)]
impl ParseBackend for InlineParseBackend {
    fn post(&mut self, req: ParseRequest) {
        let no_cancel = AtomicBool::new(false);
        let done = do_parse(&mut self.parser, req, &no_cancel);
        self.done.push_back(done);
    }

    fn drain_done(&mut self) -> Vec<ParseDone> {
        self.done.drain(..).collect()
    }

    fn is_in_flight(&self, _bid: BufferId, _text_gen: u64) -> bool {
        false
    }
    fn remove_in_flight(&mut self, _bid: BufferId) {}
    fn clear_in_flight_if_matches(
        &mut self,
        _bid: BufferId,
        _text_gen: u64,
        _lang: &Arc<LanguageConfig>,
    ) {
    }
    // Inline parses complete synchronously inside `post` — nothing is ever pending.
    fn has_in_flight(&self) -> bool {
        false
    }
    fn is_disconnected(&self) -> bool {
        false
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use slotmap::SlotMap;

    use std::collections::HashMap;

    use hume_engine::builtins::tree_sitter_hl::TreeSitterHighlighter;
    use hume_engine::grammar::LoadedGrammar;
    use hume_engine::pipeline::BufferId;
    use hume_engine::theme::ScopeRegistry;

    use super::ParseBackend as _;
    use super::{
        LanguageConfig, ParseOutcome, ParseRequest, Text, ThreadedParseBackend, coalesce_one,
    };
    use crate::editor::syntax::GrammarBundle;
    use crate::editor::tests::grammar_parser_path;

    fn make_lang(name: &str, symbol: &str) -> Arc<LanguageConfig> {
        let path = grammar_parser_path(name);
        if !path.exists() {
            panic!(
                "grammar fixture missing: {}\nrun scripts/fetch-test-grammars.sh from the repo root",
                path.display()
            );
        }
        let grammar = LoadedGrammar::open(&path, symbol).expect("load grammar");
        let query = Arc::new(tree_sitter::Query::new(grammar.language(), "").expect("empty query"));
        let mut registry = ScopeRegistry::new();
        let highlighter = Arc::new(TreeSitterHighlighter::from_shared_query(
            query,
            &mut registry,
        ));
        Arc::new(LanguageConfig {
            name: name.to_owned(),
            extensions: vec![],
            globs: vec![],
            shebangs: vec![],
            grammar: Some(GrammarBundle {
                grammar,
                highlighter,
                injections: None,
            }),
        })
    }

    fn fresh_bid() -> BufferId {
        let mut sm: SlotMap<BufferId, ()> = SlotMap::with_key();
        sm.insert(())
    }

    fn empty_langs() -> Arc<HashMap<String, Arc<LanguageConfig>>> {
        Arc::new(HashMap::new())
    }

    // ── coalesce_one (pure) ───────────────────────────────────────────────────

    #[test]
    fn coalesce_one_keeps_higher_gen() {
        use std::collections::HashMap;
        let bid = fresh_bid();
        let lang = make_lang("json", "tree_sitter_json");
        let mut batch: HashMap<BufferId, ParseRequest> = HashMap::new();

        // Gen 2 lands first.
        coalesce_one(
            &mut batch,
            ParseRequest {
                bid,
                text_gen: 2,
                lang: Arc::clone(&lang),
                text: Text::from("bb\n"),
                old_tree: None,
                langs: empty_langs(),
            },
        );
        // Gen 1 must not overwrite gen 2.
        coalesce_one(
            &mut batch,
            ParseRequest {
                bid,
                text_gen: 1,
                lang: Arc::clone(&lang),
                text: Text::from("a\n"),
                old_tree: None,
                langs: empty_langs(),
            },
        );
        assert_eq!(
            batch[&bid].text_gen, 2,
            "lower-gen request must not overwrite"
        );
        assert_eq!(
            batch[&bid].text.len_chars(),
            3,
            "text must match the winning gen"
        );

        // Gen 3 must overwrite gen 2.
        coalesce_one(
            &mut batch,
            ParseRequest {
                bid,
                text_gen: 3,
                lang: Arc::clone(&lang),
                text: Text::from("ccc\n"),
                old_tree: None,
                langs: empty_langs(),
            },
        );
        assert_eq!(batch[&bid].text_gen, 3, "higher-gen request must win");
        assert_eq!(batch[&bid].text.len_chars(), 4);

        // Equal gen must not overwrite (no-op).
        coalesce_one(
            &mut batch,
            ParseRequest {
                bid,
                text_gen: 3,
                lang: Arc::clone(&lang),
                text: Text::from("REPLACED\n"),
                old_tree: None,
                langs: empty_langs(),
            },
        );
        assert_eq!(
            batch[&bid].text.len_chars(),
            4,
            "equal-gen request must be dropped"
        );
    }

    #[test]
    fn coalesce_one_same_gen_different_lang_replaces() {
        use std::collections::HashMap;
        let bid = fresh_bid();
        let lang_a = make_lang("json", "tree_sitter_json");
        let lang_b = make_lang("rust", "tree_sitter_rust");
        let mut batch: HashMap<BufferId, ParseRequest> = HashMap::new();

        // lang_a arrives first at gen 5.
        coalesce_one(
            &mut batch,
            ParseRequest {
                bid,
                text_gen: 5,
                lang: Arc::clone(&lang_a),
                text: Text::from("{}\n"),
                old_tree: None,
                langs: empty_langs(),
            },
        );
        // lang_b at the same gen — grammar swap on a quiescent buffer.
        coalesce_one(
            &mut batch,
            ParseRequest {
                bid,
                text_gen: 5,
                lang: Arc::clone(&lang_b),
                text: Text::from("fn f(){}\n"),
                old_tree: None,
                langs: empty_langs(),
            },
        );
        // The new lang must win.
        assert!(
            Arc::ptr_eq(&batch[&bid].lang, &lang_b),
            "grammar swap must replace same-gen entry"
        );
        assert_eq!(
            batch[&bid].text.len_chars(),
            9,
            "text must match the winning lang"
        );

        // A second request with the same lang does not replace.
        coalesce_one(
            &mut batch,
            ParseRequest {
                bid,
                text_gen: 5,
                lang: Arc::clone(&lang_b),
                text: Text::from("REPLACED\n"),
                old_tree: None,
                langs: empty_langs(),
            },
        );
        assert_eq!(
            batch[&bid].text.len_chars(),
            9,
            "same-lang equal-gen request must be dropped"
        );
    }

    // ── ThreadedParseBackend shutdown ─────────────────────────────────────────

    #[test]
    fn worker_shutdown_joins_thread() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            drop(ThreadedParseBackend::new());
            let _ = tx.send(());
        });
        rx.recv_timeout(Duration::from_secs(1))
            .expect("ThreadedParseBackend::drop did not return within 1 s");
    }

    // ── Language switch re-sets cached parser ─────────────────────────────────

    #[test]
    fn worker_language_switch_produces_trees_for_both() {
        let json_lang = make_lang("json", "tree_sitter_json");
        let rust_lang = make_lang("rust", "tree_sitter_rust");
        let mut worker = ThreadedParseBackend::new();
        let bid = fresh_bid();

        worker.post(ParseRequest {
            bid,
            text_gen: 1,
            lang: Arc::clone(&json_lang),
            text: Text::from("{\"x\": 1}\n"),
            old_tree: None,
            langs: empty_langs(),
        });
        // Ensure json parse is done before sending rust, so the worker must
        // switch language on the second request.
        let done1 = worker
            .rx_done
            .recv_timeout(Duration::from_secs(5))
            .expect("json parse timed out");
        assert!(
            matches!(done1.outcome, ParseOutcome::Ok(..)),
            "json parse must succeed"
        );
        assert!(Arc::ptr_eq(&done1.lang, &json_lang));

        worker.post(ParseRequest {
            bid,
            text_gen: 2,
            lang: Arc::clone(&rust_lang),
            text: Text::from("fn main() {}\n"),
            old_tree: None,
            langs: empty_langs(),
        });
        let done2 = worker
            .rx_done
            .recv_timeout(Duration::from_secs(5))
            .expect("rust parse timed out");
        assert!(
            matches!(done2.outcome, ParseOutcome::Ok(..)),
            "rust parse must succeed after language switch"
        );
        assert!(Arc::ptr_eq(&done2.lang, &rust_lang));
    }
}
