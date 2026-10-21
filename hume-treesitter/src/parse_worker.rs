use rustc_hash::FxHashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;

use hume_engine::pipeline::BufferId;
use hume_platform::wake::{WakeCallback, spawn_waking};

use crate::edited_bytes::EditedBytes;
use crate::injections::{
    InjectionContext, InjectionSites, PriorInjections, SitePrior, resolve_and_parse_injections,
};
use crate::registry::GrammarBundle;
use hume_editing::text::BufferText;

// Compile-time Send assertions: tree_sitter::Tree is Send+Sync;
// tree_sitter::Parser is Send+!Sync (lives on the worker thread only).
const _: fn() = || {
    fn _assert_send<T: Send>() {}
    _assert_send::<ParseRequest>();
    _assert_send::<ParseDone>();
    _assert_send::<ParseOutcome>();
};

/// Nesting cap for recursive injections (root = depth 0). Covers the deepest
/// realistic case (markdown → rust → rustdoc comment → markdown) without
/// letting a pathological grammar recurse unboundedly.
pub(crate) const MAX_INJECTION_DEPTH: u8 = 3;

// ── Messages ──────────────────────────────────────────────────────────────────

pub struct ParseRequest {
    pub bid: BufferId,
    pub generation: u64,
    /// The grammar bundle to parse with. Read on the worker thread for
    /// `set_language` and injection resolution.
    pub bundle: Arc<GrammarBundle>,
    /// O(1) rope clone (structural sharing). The parser reads it chunk by
    /// chunk, so the text is never copied into one buffer.
    pub text: BufferText,
    /// The committed layers with every pending `InputEdit` applied, which
    /// each layer and its injection sites are reused or reparsed from.
    /// `None` for a full parse (first parse, grammar swap, or broken edit
    /// chain).
    pub(crate) prior: Option<PriorLayers>,
    /// Snapshot of every grammared language, for resolving an injected
    /// language name (e.g. a fenced code block's info string) to its grammar
    /// without touching main-thread state from the worker.
    pub langs: Arc<FxHashMap<String, Arc<GrammarBundle>>>,
}

/// Committed layers edited to a request's text.
pub(crate) struct PriorLayers {
    pub(crate) root: tree_sitter::Tree,
    pub(crate) root_sites: InjectionSites,
    pub(crate) injected: PriorInjections,
    /// The bytes the edits changed.
    pub(crate) edited: EditedBytes,
}

pub enum ParseOutcome {
    /// Root parse succeeded.  Carries the root tree plus every embedded-language
    /// layer resolved from it; byte text is read from the live rope at render
    /// time via `RopeProvider` in the engine highlighter.
    Ok(ParsedLayers),
    /// `Parser::parse` returned `None`: the parse was cancelled. The
    /// committed layers stay as they were and are not current. `frame_tick`
    /// does not post this generation again; the next edit, or a
    /// synchronous `Syntax::ensure_current`, parses again.
    ParseFailed,
}

/// The root parse tree plus every injected layer resolved from it.
pub struct ParsedLayers {
    pub root: tree_sitter::Tree,
    /// The root tree's injection sites.
    pub root_sites: InjectionSites,
    pub injected: Vec<ParsedInjection>,
}

/// One resolved and parsed injection layer.
pub struct ParsedInjection {
    /// The injected layer's grammar bundle; becomes the installed
    /// `SyntaxLayer`'s own `bundle` field.
    pub bundle: Arc<GrammarBundle>,
    pub tree: tree_sitter::Tree,
    /// Absolute byte ranges this layer was parsed over, sorted by start.
    pub ranges: Vec<tree_sitter::Range>,
    pub depth: u8,
    /// The layer's own injection sites.
    pub sites: InjectionSites,
}

pub struct ParseDone {
    pub bid: BufferId,
    pub generation: u64,
    /// The grammar bundle this was parsed with. Its `config_gen` is compared
    /// on the main thread to detect grammar swaps that occurred while the
    /// request was in flight.
    pub bundle: Arc<GrammarBundle>,
    pub outcome: ParseOutcome,
}

// ── Coalescing helper ─────────────────────────────────────────────────────────

/// Insert `req` into `batch`, keeping only the highest-`generation` entry per `bid`.
/// Older or equal-gen duplicates are dropped without cloning.
fn coalesce_one(batch: &mut FxHashMap<BufferId, ParseRequest>, req: ParseRequest) {
    use std::collections::hash_map::Entry;
    match batch.entry(req.bid) {
        Entry::Vacant(v) => {
            v.insert(req);
        }
        Entry::Occupied(mut o) => {
            // Keep the latest generation; when generations are equal, a different
            // config_gen means a grammar swap on a quiescent buffer, so take the
            // new entry so the fresh grammar wins even without a text edit.
            if req.generation > o.get().generation
                || (req.generation == o.get().generation
                    && req.bundle.config_gen != o.get().bundle.config_gen)
            {
                o.insert(req);
            }
            // else: drop req: older or same-grammar equal-gen duplicate.
        }
    }
}

// ── Shared parse logic ────────────────────────────────────────────────────────

/// Parse `rope` with `parser` (already configured: language + included
/// ranges), honoring `cancel`. Feeds the rope via a chunked callback, which avoids
/// a full `Vec<u8>` allocation. `chunk_at_byte` returns a &str slice directly
/// into the rope's immutable B-tree nodes.
///
/// Shared by the root parse and every injected-layer parse. `old_tree` is
/// the layer's previous tree, edited to `rope`'s text, for an incremental
/// parse; `None` parses in full.
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
/// `pub(crate)` for `Syntax::ensure_current`, which parses on the calling
/// thread when a reader needs the tree of the current text now (and, through
/// it, `Syntax::attach_sync` for small one-shot content such as a hover
/// popup).
///
/// Always calls `set_language` and resets `included_ranges` to whole-buffer
/// before the root parse: layer parsing switches languages and ranges
/// constantly, so a "current language" cache (as a single-tree parser had)
/// would just thrash on every request.
pub(crate) fn do_parse(
    parser: &mut tree_sitter::Parser,
    req: ParseRequest,
    cancel: &AtomicBool,
) -> ParseDone {
    parser.set_language(req.bundle.grammar.language()).expect(
        "ABI verified at grammar registration time in attach_grammar; the only \
             other failure mode, a language recovered from a tree in a different \
             WebAssembly instance, cannot arise here: grammars are dlopen'd \
             natively, never wasm-recovered",
    );
    parser
        .set_included_ranges(&[])
        .expect("empty ranges are always valid: whole-buffer parse");

    let rope = req.text.rope();
    let prior = req.prior.as_ref();
    let outcome = match run_parse(parser, rope, prior.map(|prior| &prior.root), cancel) {
        Some(root) => {
            let cx = InjectionContext {
                rope,
                langs: &req.langs,
                prior: prior.map(|prior| (&prior.injected, &prior.edited)),
                cancel,
            };
            let root_prior = prior.map(|prior| SitePrior {
                old_tree: &prior.root,
                sites: &prior.root_sites,
                hull: 0..=rope.len_bytes(),
                reparsed: true,
                edited: &prior.edited,
            });
            let (root_sites, injected) =
                resolve_and_parse_injections(parser, &cx, &root, &req.bundle, 1, root_prior);
            ParseOutcome::Ok(ParsedLayers {
                root,
                root_sites,
                injected,
            })
        }
        None => ParseOutcome::ParseFailed,
    };

    ParseDone {
        bid: req.bid,
        generation: req.generation,
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
}

impl WorkerState {
    fn run(mut self, wake: &WakeCallback) {
        loop {
            // Block until at least one request arrives; exit when the main
            // thread closes the channel (editor shutting down).
            let first = match self.rx.recv() {
                Ok(r) => r,
                Err(_) => return,
            };

            // Coalesce: drain any additional queued requests, keeping only the
            // highest-generation request per BufferId.  Superseded requests are
            // already obsolete and would produce trees the main thread discards.
            let mut batch: FxHashMap<BufferId, ParseRequest> = FxHashMap::default();
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
                wake();
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
    /// `wake` is called after every posted result (and once more, harmlessly,
    /// when the worker thread exits) so the editor's main loop wakes instead
    /// of polling for completion.
    pub fn with_waker(wake: WakeCallback) -> Self {
        let (tx_req, rx_req) = mpsc::channel::<ParseRequest>();
        let (tx_done, rx_done) = mpsc::channel::<ParseDone>();
        let cancel = Arc::new(AtomicBool::new(false));

        let state = WorkerState {
            rx: rx_req,
            tx: tx_done,
            parser: tree_sitter::Parser::new(),
            cancel: Arc::clone(&cancel),
        };

        let thread = spawn_waking("hume-parse-worker", wake, move |wake| state.run(wake))
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
/// no waiting; tests call `reparse_stale_buffers` instead of blocking helpers.
///
/// Gated behind `test-util` (not just `#[cfg(test)]`): `hume-editor`'s own test
/// suite constructs this across the crate boundary, so crate-local `cfg(test)`
/// alone isn't enough, same reasoning as `LanguageRegistry::register_identity`
/// in `registry.rs`. Never constructed in production: `Editor::open` always
/// builds `ThreadedParseBackend` directly.
#[cfg(any(test, feature = "test-util"))]
pub struct InlineParseBackend {
    parser: tree_sitter::Parser,
    done: std::collections::VecDeque<ParseDone>,
}

#[cfg(any(test, feature = "test-util"))]
impl InlineParseBackend {
    pub fn new() -> Self {
        Self {
            parser: tree_sitter::Parser::new(),
            done: std::collections::VecDeque::new(),
        }
    }
}

#[cfg(any(test, feature = "test-util"))]
impl Default for InlineParseBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-util"))]
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

#[cfg(test)]
mod tests;
