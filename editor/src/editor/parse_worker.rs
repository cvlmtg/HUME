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

// ── Coalescing helper ─────────────────────────────────────────────────────────

/// Insert `req` into `batch`, keeping only the highest-`text_gen` entry per `bid`.
/// Older or equal-gen duplicates are dropped without cloning.
fn coalesce_one(batch: &mut HashMap<BufferId, ParseRequest>, req: ParseRequest) {
    use std::collections::hash_map::Entry;
    match batch.entry(req.bid) {
        Entry::Vacant(v) => { v.insert(req); }
        Entry::Occupied(mut o) => {
            if req.text_gen > o.get().text_gen {
                o.insert(req);
            }
            // else: drop req — older or equal-gen duplicate.
        }
    }
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
            // highest-text_gen request per BufferId.  Superseded requests are
            // already obsolete and would produce trees the main thread discards.
            let mut batch: HashMap<BufferId, ParseRequest> = HashMap::new();
            batch.insert(first.bid, first);
            let disconnected = 'drain: loop {
                match self.rx.try_recv() {
                    Ok(r) => coalesce_one(&mut batch, r),
                    Err(mpsc::TryRecvError::Empty) => break 'drain false,
                    Err(mpsc::TryRecvError::Disconnected) => break 'drain true,
                }
            };

            for (_, req) in batch {
                if self.parse_one(req) == Flow::Exit {
                    return;
                }
            }

            if disconnected {
                return;
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
            let in_flight = InFlight { text_gen: req.text_gen };
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use slotmap::SlotMap;

    use engine::grammar::LoadedGrammar;
    use engine::pipeline::BufferId;

    use super::{LanguageConfig, ParseRequest, ParseWorker, coalesce_one};
    use crate::editor::syntax::GrammarBundle;

    fn grammar_path(name: &str) -> PathBuf {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tests/fixtures/grammars");
        let suffix = if cfg!(target_os = "macos") { "dylib" }
                     else if cfg!(windows) { "dll" }
                     else { "so" };
        base.join(name).join(format!("parser.{suffix}"))
    }

    fn make_lang(name: &str, symbol: &str) -> Arc<LanguageConfig> {
        let path = grammar_path(name);
        if !path.exists() {
            panic!(
                "grammar fixture missing: {}\nrun scripts/fetch-test-grammars.sh from the repo root",
                path.display()
            );
        }
        let grammar = LoadedGrammar::open(&path, symbol).expect("load grammar");
        let query = Arc::new(
            tree_sitter::Query::new(grammar.language(), "").expect("empty query"),
        );
        Arc::new(LanguageConfig {
            name: name.to_owned(),
            extensions: vec![],
            globs: vec![],
            shebangs: vec![],
            grammar: Some(GrammarBundle { grammar, query }),
        })
    }

    fn fresh_bid() -> BufferId {
        let mut sm: SlotMap<BufferId, ()> = SlotMap::with_key();
        sm.insert(())
    }

    // ── coalesce_one (pure) ───────────────────────────────────────────────────

    #[test]
    fn coalesce_one_keeps_higher_gen() {
        use std::collections::HashMap;
        let bid = fresh_bid();
        let lang = make_lang("json", "tree_sitter_json");
        let mut batch: HashMap<BufferId, ParseRequest> = HashMap::new();

        // Gen 2 lands first.
        coalesce_one(&mut batch, ParseRequest {
            bid, text_gen: 2, lang: Arc::clone(&lang), source_bytes: b"22".to_vec(),
        });
        // Gen 1 must not overwrite gen 2.
        coalesce_one(&mut batch, ParseRequest {
            bid, text_gen: 1, lang: Arc::clone(&lang), source_bytes: b"1".to_vec(),
        });
        assert_eq!(batch[&bid].text_gen, 2, "lower-gen request must not overwrite");
        assert_eq!(batch[&bid].source_bytes, b"22", "source must match the winning gen");

        // Gen 3 must overwrite gen 2.
        coalesce_one(&mut batch, ParseRequest {
            bid, text_gen: 3, lang: Arc::clone(&lang), source_bytes: b"333".to_vec(),
        });
        assert_eq!(batch[&bid].text_gen, 3, "higher-gen request must win");
        assert_eq!(batch[&bid].source_bytes, b"333");

        // Equal gen must not overwrite (no-op).
        coalesce_one(&mut batch, ParseRequest {
            bid, text_gen: 3, lang: Arc::clone(&lang), source_bytes: b"REPLACED".to_vec(),
        });
        assert_eq!(batch[&bid].source_bytes, b"333", "equal-gen request must be dropped");
    }

    // ── ParseWorker shutdown ──────────────────────────────────────────────────

    #[test]
    fn worker_shutdown_joins_thread() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            drop(ParseWorker::new());
            let _ = tx.send(());
        });
        rx.recv_timeout(Duration::from_secs(1))
            .expect("ParseWorker::drop did not return within 1 s");
    }

    // ── Language switch re-sets cached parser ─────────────────────────────────

    #[test]
    fn worker_language_switch_produces_trees_for_both() {
        let json_lang = make_lang("json", "tree_sitter_json");
        let rust_lang = make_lang("rust", "tree_sitter_rust");
        let mut worker = ParseWorker::new();
        let bid = fresh_bid();

        worker.post(ParseRequest {
            bid,
            text_gen: 1,
            lang: Arc::clone(&json_lang),
            source_bytes: b"{\"x\": 1}".to_vec(),
        });
        // Ensure json parse is done before sending rust, so the worker must
        // switch language on the second request.
        let done1 = worker.rx_done.recv_timeout(Duration::from_secs(5))
            .expect("json parse timed out");
        assert!(done1.tree.is_some(), "json parse must succeed");
        assert!(Arc::ptr_eq(&done1.lang, &json_lang));

        worker.post(ParseRequest {
            bid,
            text_gen: 2,
            lang: Arc::clone(&rust_lang),
            source_bytes: b"fn main() {}".to_vec(),
        });
        let done2 = worker.rx_done.recv_timeout(Duration::from_secs(5))
            .expect("rust parse timed out");
        assert!(done2.tree.is_some(), "rust parse must succeed after language switch");
        assert!(Arc::ptr_eq(&done2.lang, &rust_lang));
    }
}
