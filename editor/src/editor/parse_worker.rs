use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use std::thread;

use engine::pipeline::BufferId;

use super::syntax::LanguageConfig;

// Compile-time Send assertions — tree_sitter::Tree is Send+Sync;
// tree_sitter::Parser is Send+!Sync (lives on the worker thread only).
const _: fn() = || {
    fn _assert_send<T: Send>() {}
    _assert_send::<ParseRequest>();
    _assert_send::<ParseDone>();
};

// ── Messages ──────────────────────────────────────────────────────────────────

pub(super) struct ParseRequest {
    pub(super) bid: BufferId,
    pub(super) text_gen: u64,
    /// Keepalive: holds the Arc so the dlopen'd grammar is not unloaded while
    /// the worker holds this request.
    pub(super) lang: Arc<LanguageConfig>,
    /// Full-text snapshot at `text_gen`.  O(n) allocation; the incremental
    /// milestone (M9.5) will replace this with `InputEdit` + delta.
    pub(super) source_bytes: Vec<u8>,
}

pub(super) struct ParseDone {
    pub(super) bid: BufferId,
    pub(super) text_gen: u64,
    /// Identity token — compared via `Arc::ptr_eq` on the main thread to detect
    /// grammar swaps that occurred while the request was in flight.
    pub(super) lang: Arc<LanguageConfig>,
    /// `None` when `Parser::set_language` rejected the grammar ABI.
    pub(super) tree: Option<tree_sitter::Tree>,
    /// Moved back from the worker so `refresh_source` can reuse the allocation
    /// without an extra copy.
    pub(super) source_bytes: Vec<u8>,
}

// ── Worker internals (stays on the worker thread) ─────────────────────────────

struct WorkerState {
    rx: mpsc::Receiver<ParseRequest>,
    tx: mpsc::Sender<ParseDone>,
    parser: tree_sitter::Parser,
    /// Arc of the language currently loaded into `parser`, used to detect
    /// language switches via `Arc::ptr_eq` without extra clones.
    current_lang: Option<Arc<LanguageConfig>>,
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
            // latest per BufferId.  Earlier requests are already superseded —
            // parsing them wastes CPU and produces trees the main thread will
            // discard on the text_gen check.
            let mut batch: HashMap<BufferId, ParseRequest> = HashMap::new();
            batch.insert(first.bid, first);
            'drain: loop {
                match self.rx.try_recv() {
                    Ok(r) => {
                        let entry = batch.entry(r.bid).or_insert_with(|| ParseRequest {
                            bid: r.bid,
                            text_gen: 0,
                            lang: Arc::clone(&r.lang),
                            source_bytes: Vec::new(),
                        });
                        if r.text_gen >= entry.text_gen {
                            *entry = r;
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break 'drain,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        for (_, req) in batch {
                            if self.parse_one(req) == Flow::Exit {
                                return;
                            }
                        }
                        return;
                    }
                }
            }

            for (_, req) in batch {
                if self.parse_one(req) == Flow::Exit {
                    return;
                }
            }
        }
    }

    /// Parse one request and send the result back.  Returns `Exit` when the
    /// result channel is closed (editor dropped).
    fn parse_one(&mut self, req: ParseRequest) -> Flow {
        let language_changed = self.current_lang.as_ref()
            .map_or(true, |cur| !Arc::ptr_eq(cur, &req.lang));

        if language_changed {
            let bundle = match req.lang.grammar.as_ref() {
                Some(b) => b,
                None => {
                    // Grammar detached between enqueue and execution.
                    self.current_lang = None;
                    return self.send_done(req, None);
                }
            };
            match self.parser.set_language(&bundle.grammar.language()) {
                Ok(()) => self.current_lang = Some(Arc::clone(&req.lang)),
                Err(_) => {
                    // ABI mismatch — should not happen after a successful
                    // attach, but handle defensively.
                    self.current_lang = None;
                    return self.send_done(req, None);
                }
            }
        }

        let tree = self.parser.parse(&req.source_bytes, None);
        self.send_done(req, tree)
    }

    fn send_done(&self, req: ParseRequest, tree: Option<tree_sitter::Tree>) -> Flow {
        match self.tx.send(ParseDone {
            bid: req.bid,
            text_gen: req.text_gen,
            lang: req.lang,
            tree,
            source_bytes: req.source_bytes,
        }) {
            Ok(()) => Flow::Continue,
            Err(_) => Flow::Exit,
        }
    }
}

#[derive(PartialEq, Eq)]
enum Flow {
    Continue,
    Exit,
}

// ── ParseWorker handle (lives on the main thread) ─────────────────────────────

/// Records a pending parse request so `reparse_stale_buffers` can avoid
/// re-submitting for a buffer whose request is already in flight.
pub(super) struct InFlight {
    /// Keepalive + identity token for the submitted request.
    pub(super) lang: Arc<LanguageConfig>,
    /// `text_gen` of the submitted request.
    pub(super) text_gen: u64,
}

pub(super) struct ParseWorker {
    /// `None` after `Drop` closes the channel to signal the worker.
    tx_req: Option<mpsc::Sender<ParseRequest>>,
    /// Incoming parse results.  Drained by `reparse_stale_buffers` and by
    /// `Editor::join_pending_parses`.
    pub(super) rx_done: mpsc::Receiver<ParseDone>,
    /// Per-buffer record of the most recently submitted (but not yet installed)
    /// request.  Keyed by `BufferId`; at most one entry per buffer.
    pub(super) in_flight: HashMap<BufferId, InFlight>,
    /// Set when `tx_req.send` returns `Err` (worker exited unexpectedly).
    /// Once set, suppresses all further sends; parsing is suspended for the
    /// lifetime of this editor session.
    pub(super) disconnected: bool,
    /// Set after the disconnect has been surfaced to the message log so the
    /// warning fires exactly once.
    pub(super) disconnect_logged: bool,
    thread: Option<thread::JoinHandle<()>>,
}

impl ParseWorker {
    pub(super) fn new() -> Self {
        let (tx_req, rx_req) = mpsc::channel::<ParseRequest>();
        let (tx_done, rx_done) = mpsc::channel::<ParseDone>();

        let state = WorkerState {
            rx: rx_req,
            tx: tx_done,
            parser: tree_sitter::Parser::new(),
            current_lang: None,
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
            disconnect_logged: false,
            thread: Some(thread),
        }
    }

    /// Submit a parse request and record it in `in_flight`.
    ///
    /// No-op (and `disconnected` is set on first failure) if the worker has exited.
    pub(super) fn post(&mut self, req: ParseRequest) {
        if self.disconnected {
            return;
        }
        if let Some(ref tx) = self.tx_req {
            let bid = req.bid;
            let in_flight = InFlight { lang: Arc::clone(&req.lang), text_gen: req.text_gen };
            match tx.send(req) {
                Ok(()) => { self.in_flight.insert(bid, in_flight); }
                Err(_) => {
                    self.disconnected = true;
                    self.in_flight.clear();
                }
            }
        }
    }
}

impl Drop for ParseWorker {
    fn drop(&mut self) {
        // Closing tx_req causes the worker's rx.recv() to return Err, exiting the loop.
        self.tx_req = None;
        if let Some(jh) = self.thread.take() {
            let _ = jh.join();
        }
    }
}
