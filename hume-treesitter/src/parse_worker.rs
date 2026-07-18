use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;

use hume_engine::pipeline::BufferId;

use crate::injections::resolve_and_parse_injections;
use crate::registry::GrammarBundle;
use hume_editing::text::Text;

/// Called by the worker thread after posting results, so the editor's main
/// loop wakes and drains them instead of rechecking on a poll cadence.
/// Type-erased so this crate stays free of a `hume-platform` dependency —
/// production wraps `hume_platform::events::EventWaker::wake`.
pub type WakeCallback = Arc<dyn Fn() + Send + Sync>;

/// Invokes a [`WakeCallback`] on drop — fires whether the worker thread
/// exits normally or unwinds from a panic, so a dead worker still wakes the
/// main loop once (the subsequent drain observes the disconnect via the
/// existing channel and reports it through `is_disconnected`). A normal
/// exit firing one extra, spurious wake is harmless — callers already
/// tolerate spurious wakes by design.
struct WakeOnDrop(WakeCallback);

impl Drop for WakeOnDrop {
    fn drop(&mut self) {
        (self.0)();
    }
}

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
pub(crate) const MAX_INJECTION_DEPTH: u8 = 3;

// ── Messages ──────────────────────────────────────────────────────────────────

pub struct ParseRequest {
    pub bid: BufferId,
    pub text_gen: u64,
    /// The grammar bundle to parse with. Read on the worker thread for
    /// `set_language` and injection resolution.
    pub bundle: Arc<GrammarBundle>,
    /// O(1) rope clone (structural sharing) — serialised to bytes on the worker
    /// thread only when the parse succeeds, avoiding the main-thread allocation.
    pub text: Text,
    /// Previous parse tree with all pending `InputEdit`s applied, enabling
    /// incremental re-parsing.  `None` for a full reparse (first parse, grammar
    /// swap, or broken edit chain).
    pub old_tree: Option<tree_sitter::Tree>,
    /// Snapshot of every grammared language, for resolving an injected
    /// language name (e.g. a fenced code block's info string) to its grammar
    /// without touching main-thread state from the worker.
    pub langs: Arc<HashMap<String, Arc<GrammarBundle>>>,
}

pub enum ParseOutcome {
    /// Root parse succeeded.  Carries the root tree plus every embedded-language
    /// layer resolved from it; byte text is read from the live rope at render
    /// time via `RopeProvider` in the engine highlighter.
    Ok(ParsedLayers),
    /// `Parser::parse` returned `None` (transient; currently unreachable without
    /// a cancellation / timeout).  Syntax stays attached; the next frame retries.
    ParseFailed,
}

/// The root parse tree plus every injected layer resolved from it.
pub struct ParsedLayers {
    pub root: tree_sitter::Tree,
    pub injected: Vec<ParsedInjection>,
}

/// One resolved and parsed injection layer.
pub struct ParsedInjection {
    /// The injected layer's grammar bundle — read for its highlighter on install.
    pub bundle: Arc<GrammarBundle>,
    pub tree: tree_sitter::Tree,
    /// Absolute byte ranges this layer was parsed over, sorted by start.
    pub ranges: Vec<tree_sitter::Range>,
    pub depth: u8,
}

pub struct ParseDone {
    pub bid: BufferId,
    pub text_gen: u64,
    /// The grammar bundle this was parsed with. Its `config_gen` is compared
    /// on the main thread to detect grammar swaps that occurred while the
    /// request was in flight.
    pub bundle: Arc<GrammarBundle>,
    pub outcome: ParseOutcome,
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
            // config_gen means a grammar swap on a quiescent buffer — take the
            // new entry so the fresh grammar wins even without a text edit.
            if req.text_gen > o.get().text_gen
                || (req.text_gen == o.get().text_gen
                    && req.bundle.config_gen != o.get().bundle.config_gen)
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
pub(crate) fn run_parse(
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
    parser
        .set_language(req.bundle.grammar.language())
        .expect("ABI verified at grammar registration time in attach_grammar");
    parser
        .set_included_ranges(&[])
        .expect("empty ranges are always valid — whole-buffer parse");

    let rope = req.text.rope();
    let outcome = match run_parse(parser, rope, req.old_tree.as_ref(), cancel) {
        Some(root) => {
            let injected = resolve_and_parse_injections(
                parser,
                &root,
                &req.bundle,
                rope,
                &req.langs,
                cancel,
                1,
            );
            ParseOutcome::Ok(ParsedLayers { root, injected })
        }
        None => ParseOutcome::ParseFailed,
    };

    ParseDone {
        bid: req.bid,
        text_gen: req.text_gen,
        bundle: req.bundle,
        outcome,
    }
}

// ── Worker internals (stays on the worker thread) ─────────────────────────────

struct WorkerState {
    rx: mpsc::Receiver<ParseRequest>,
    tx: mpsc::Sender<ParseDone>,
    parser: tree_sitter::Parser,
    cancel: Arc<AtomicBool>,
    wake: WakeCallback,
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
                (self.wake)();
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
pub trait ParseBackend {
    fn post(&mut self, req: ParseRequest);

    /// Drain all available parse results.
    fn drain_done(&mut self) -> Vec<ParseDone>;

    fn is_disconnected(&self) -> bool;
}

// ── ThreadedParseBackend (production) ─────────────────────────────────────────

pub struct ThreadedParseBackend {
    /// `None` after `Drop` closes the channel to signal the worker.
    tx_req: Option<mpsc::Sender<ParseRequest>>,
    rx_done: mpsc::Receiver<ParseDone>,
    disconnected: bool,
    cancel: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl ThreadedParseBackend {
    pub fn new() -> Self {
        Self::with_waker(Arc::new(|| {}))
    }

    /// Like [`Self::new`], but `wake` is called after every posted result
    /// (and once more, harmlessly, when the worker thread exits) so the
    /// editor's main loop wakes instead of polling for completion.
    pub fn with_waker(wake: WakeCallback) -> Self {
        let (tx_req, rx_req) = mpsc::channel::<ParseRequest>();
        let (tx_done, rx_done) = mpsc::channel::<ParseDone>();
        let cancel = Arc::new(AtomicBool::new(false));

        let state = WorkerState {
            rx: rx_req,
            tx: tx_done,
            parser: tree_sitter::Parser::new(),
            cancel: Arc::clone(&cancel),
            wake: Arc::clone(&wake),
        };

        let thread = thread::Builder::new()
            .name("hume-parse-worker".into())
            .spawn(move || {
                let _wake_on_drop = WakeOnDrop(wake);
                state.run()
            })
            .expect("failed to spawn parse worker thread");

        Self {
            tx_req: Some(tx_req),
            rx_done,
            disconnected: false,
            cancel,
            thread: Some(thread),
        }
    }
}

impl Default for ThreadedParseBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ParseBackend for ThreadedParseBackend {
    fn post(&mut self, req: ParseRequest) {
        if self.disconnected {
            return;
        }
        if let Some(ref tx) = self.tx_req
            && tx.send(req).is_err()
        {
            self.disconnected = true;
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
                    break;
                }
            }
        }
        out
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

// ── InlineParseBackend (synchronous backend for tests) ────────────────────────

/// Synchronous parse backend for tests.  `post` runs the parse immediately and
/// queues the result; `drain_done` flushes the queue.  No threads, no channels,
/// no waiting — tests call `reparse_stale_buffers` instead of blocking helpers.
///
/// `pub` (not `#[cfg(test)]`): used from hume-editor's own test suite across
/// the crate boundary, and as the default backend before a real editor swaps
/// in `ThreadedParseBackend`.
pub struct InlineParseBackend {
    parser: tree_sitter::Parser,
    done: std::collections::VecDeque<ParseDone>,
}

impl InlineParseBackend {
    pub fn new() -> Self {
        Self {
            parser: tree_sitter::Parser::new(),
            done: std::collections::VecDeque::new(),
        }
    }
}

impl Default for InlineParseBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ParseBackend for InlineParseBackend {
    fn post(&mut self, req: ParseRequest) {
        let no_cancel = AtomicBool::new(false);
        let done = do_parse(&mut self.parser, req, &no_cancel);
        self.done.push_back(done);
    }

    fn drain_done(&mut self) -> Vec<ParseDone> {
        self.done.drain(..).collect()
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

    use crate::highlight::TreeSitterHighlighter;
    use crate::grammar::LoadedGrammar;
    use hume_engine::pipeline::BufferId;
    use hume_engine::theme::ScopeRegistry;

    use super::ParseBackend as _;
    use super::{ParseOutcome, ParseRequest, Text, ThreadedParseBackend, coalesce_one};
    use crate::registry::GrammarBundle;
    use crate::test_support::grammar_parser_path;

    /// Distinct per call, mirroring `LanguageRegistry`'s `config_gen`
    /// invariant so tests that compare bundles by gen see real identity.
    fn next_test_config_gen() -> u32 {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT_GEN: AtomicU32 = AtomicU32::new(0);
        NEXT_GEN.fetch_add(1, Ordering::Relaxed)
    }

    fn make_bundle(name: &str, symbol: &str) -> Arc<GrammarBundle> {
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
        Arc::new(GrammarBundle {
            grammar,
            highlighter,
            injections: None,
            config_gen: next_test_config_gen(),
        })
    }

    fn fresh_bid() -> BufferId {
        let mut sm: SlotMap<BufferId, ()> = SlotMap::with_key();
        sm.insert(())
    }

    fn empty_langs() -> Arc<HashMap<String, Arc<GrammarBundle>>> {
        Arc::new(HashMap::new())
    }

    // ── coalesce_one (pure) ───────────────────────────────────────────────────

    #[test]
    fn coalesce_one_keeps_higher_gen() {
        use std::collections::HashMap;
        let bid = fresh_bid();
        let bundle = make_bundle("json", "tree_sitter_json");
        let mut batch: HashMap<BufferId, ParseRequest> = HashMap::new();

        // Gen 2 lands first.
        coalesce_one(
            &mut batch,
            ParseRequest {
                bid,
                text_gen: 2,
                bundle: Arc::clone(&bundle),
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
                bundle: Arc::clone(&bundle),
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
                bundle: Arc::clone(&bundle),
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
                bundle: Arc::clone(&bundle),
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
        let bundle_a = make_bundle("json", "tree_sitter_json");
        let bundle_b = make_bundle("rust", "tree_sitter_rust");
        let mut batch: HashMap<BufferId, ParseRequest> = HashMap::new();

        // bundle_a arrives first at gen 5.
        coalesce_one(
            &mut batch,
            ParseRequest {
                bid,
                text_gen: 5,
                bundle: Arc::clone(&bundle_a),
                text: Text::from("{}\n"),
                old_tree: None,
                langs: empty_langs(),
            },
        );
        // bundle_b at the same gen — grammar swap on a quiescent buffer.
        coalesce_one(
            &mut batch,
            ParseRequest {
                bid,
                text_gen: 5,
                bundle: Arc::clone(&bundle_b),
                text: Text::from("fn f(){}\n"),
                old_tree: None,
                langs: empty_langs(),
            },
        );
        // The new bundle must win.
        assert_eq!(
            batch[&bid].bundle.config_gen,
            bundle_b.config_gen,
            "grammar swap must replace same-gen entry"
        );
        assert_eq!(
            batch[&bid].text.len_chars(),
            9,
            "text must match the winning lang"
        );

        // A second request with the same bundle does not replace.
        coalesce_one(
            &mut batch,
            ParseRequest {
                bid,
                text_gen: 5,
                bundle: Arc::clone(&bundle_b),
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
        let json_bundle = make_bundle("json", "tree_sitter_json");
        let rust_bundle = make_bundle("rust", "tree_sitter_rust");
        let mut worker = ThreadedParseBackend::new();
        let bid = fresh_bid();

        worker.post(ParseRequest {
            bid,
            text_gen: 1,
            bundle: Arc::clone(&json_bundle),
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
        assert_eq!(done1.bundle.config_gen, json_bundle.config_gen);

        worker.post(ParseRequest {
            bid,
            text_gen: 2,
            bundle: Arc::clone(&rust_bundle),
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
        assert_eq!(done2.bundle.config_gen, rust_bundle.config_gen);
    }

    // ── Wake callback ─────────────────────────────────────────────────────────

    #[test]
    fn parse_completion_fires_waker() {
        let (tx_wake, rx_wake) = std::sync::mpsc::channel::<()>();
        let wake: super::WakeCallback = Arc::new(move || {
            let _ = tx_wake.send(());
        });
        let mut worker = ThreadedParseBackend::with_waker(wake);
        let bundle = make_bundle("json", "tree_sitter_json");

        worker.post(ParseRequest {
            bid: fresh_bid(),
            text_gen: 1,
            bundle,
            text: Text::from("{}\n"),
            old_tree: None,
            langs: empty_langs(),
        });
        worker
            .rx_done
            .recv_timeout(Duration::from_secs(5))
            .expect("parse timed out");
        rx_wake
            .recv_timeout(Duration::from_secs(1))
            .expect("waker must fire after the worker posts a result");
    }

    #[test]
    fn wake_on_drop_fires_during_unwind() {
        let (tx_wake, rx_wake) = std::sync::mpsc::channel::<()>();
        let wake: super::WakeCallback = Arc::new(move || {
            let _ = tx_wake.send(());
        });
        let handle = std::thread::spawn(move || {
            let _guard = super::WakeOnDrop(wake);
            panic!("simulated worker crash");
        });
        assert!(handle.join().is_err(), "thread should have panicked");
        rx_wake
            .recv_timeout(Duration::from_secs(1))
            .expect("WakeOnDrop must fire its callback during unwind");
    }
}
