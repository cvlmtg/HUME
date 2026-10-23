use std::sync::Arc;
use std::time::Duration;

use rustc_hash::FxHashMap;

use hume_engine::pipeline::BufferId;
use hume_platform::wake::{counting_wake, no_op_wake};

use super::ParseBackend as _;
use super::{BufferText, ParseOutcome, ParseRequest, ThreadedParseBackend, coalesce_one};
use crate::registry::{GrammarBundle, GrammarSnapshot};
use crate::test_support::{empty_langs, fresh_bid};
use test_fixtures::require_grammars;

fn make_bundle(name: &str, symbol: &str) -> Arc<GrammarBundle> {
    crate::test_support::make_bundle(name, symbol, "", &[])
}

// ── coalesce_one (pure) ───────────────────────────────────────────────────

#[test]
fn coalesce_one_keeps_higher_gen() {
    require_grammars(&["json"]);
    let bid = fresh_bid();
    let bundle = make_bundle("json", "tree_sitter_json");
    let mut batch: FxHashMap<BufferId, ParseRequest> = FxHashMap::default();

    // Gen 2 lands first.
    coalesce_one(
        &mut batch,
        ParseRequest {
            bid,
            generation: 2,
            bundle: Arc::clone(&bundle),
            text: BufferText::from("bb\n"),
            prior: None,
            langs: empty_langs(),
        },
    );
    // Gen 1 must not overwrite gen 2.
    coalesce_one(
        &mut batch,
        ParseRequest {
            bid,
            generation: 1,
            bundle: Arc::clone(&bundle),
            text: BufferText::from("a\n"),
            prior: None,
            langs: empty_langs(),
        },
    );
    assert_eq!(
        batch[&bid].generation, 2,
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
            generation: 3,
            bundle: Arc::clone(&bundle),
            text: BufferText::from("ccc\n"),
            prior: None,
            langs: empty_langs(),
        },
    );
    assert_eq!(batch[&bid].generation, 3, "higher-gen request must win");
    assert_eq!(batch[&bid].text.len_chars(), 4);

    // Equal gen must not overwrite (no-op).
    coalesce_one(
        &mut batch,
        ParseRequest {
            bid,
            generation: 3,
            bundle: Arc::clone(&bundle),
            text: BufferText::from("REPLACED\n"),
            prior: None,
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
    require_grammars(&["json", "rust"]);
    let bid = fresh_bid();
    let bundle_a = make_bundle("json", "tree_sitter_json");
    let bundle_b = make_bundle("rust", "tree_sitter_rust");
    let mut batch: FxHashMap<BufferId, ParseRequest> = FxHashMap::default();

    // bundle_a arrives first at gen 5.
    coalesce_one(
        &mut batch,
        ParseRequest {
            bid,
            generation: 5,
            bundle: Arc::clone(&bundle_a),
            text: BufferText::from("{}\n"),
            prior: None,
            langs: empty_langs(),
        },
    );
    // bundle_b at the same gen: grammar swap on a quiescent buffer.
    coalesce_one(
        &mut batch,
        ParseRequest {
            bid,
            generation: 5,
            bundle: Arc::clone(&bundle_b),
            text: BufferText::from("fn f(){}\n"),
            prior: None,
            langs: empty_langs(),
        },
    );
    // The new bundle must win.
    assert_eq!(
        batch[&bid].bundle.config_gen, bundle_b.config_gen,
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
            generation: 5,
            bundle: Arc::clone(&bundle_b),
            text: BufferText::from("REPLACED\n"),
            prior: None,
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
        drop(ThreadedParseBackend::with_waker(no_op_wake()));
        let _ = tx.send(());
    });
    rx.recv_timeout(Duration::from_secs(1))
        .expect("ThreadedParseBackend::drop did not return within 1 s");
}

// ── Language switch re-sets cached parser ─────────────────────────────────

#[test]
fn worker_language_switch_produces_trees_for_both() {
    require_grammars(&["json", "rust"]);
    let json_bundle = make_bundle("json", "tree_sitter_json");
    let rust_bundle = make_bundle("rust", "tree_sitter_rust");
    let mut worker = ThreadedParseBackend::with_waker(no_op_wake());
    let bid = fresh_bid();

    worker.post(ParseRequest {
        bid,
        generation: 1,
        bundle: Arc::clone(&json_bundle),
        text: BufferText::from("{\"x\": 1}\n"),
        prior: None,
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
        generation: 2,
        bundle: Arc::clone(&rust_bundle),
        text: BufferText::from("fn main() {}\n"),
        prior: None,
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
    require_grammars(&["json"]);
    let (wake, rx_wake) = counting_wake();
    let mut worker = ThreadedParseBackend::with_waker(wake);
    let bundle = make_bundle("json", "tree_sitter_json");

    worker.post(ParseRequest {
        bid: fresh_bid(),
        generation: 1,
        bundle,
        text: BufferText::from("{}\n"),
        prior: None,
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
fn coalesce_one_same_gen_newer_snapshot_replaces() {
    require_grammars(&["json"]);
    let bid = fresh_bid();
    let bundle = make_bundle("json", "tree_sitter_json");
    let request = |langs: Arc<GrammarSnapshot>| ParseRequest {
        bid,
        generation: 5,
        bundle: Arc::clone(&bundle),
        text: BufferText::from("{}\n"),
        prior: None,
        langs,
    };
    let snapshot = || Arc::new(GrammarSnapshot::default().with_unloaded("rust", bundle.language));
    let older = snapshot();
    let newer = snapshot();
    let mut batch: FxHashMap<BufferId, ParseRequest> = FxHashMap::default();

    coalesce_one(&mut batch, request(Arc::clone(&older)));
    coalesce_one(&mut batch, request(Arc::clone(&newer)));
    assert_eq!(batch[&bid].langs.generation(), newer.generation());

    coalesce_one(&mut batch, request(older));
    assert_eq!(
        batch[&bid].langs.generation(),
        newer.generation(),
        "an older snapshot does not replace a newer one"
    );
}
