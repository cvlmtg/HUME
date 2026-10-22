# Vendored crates

Third-party crates HUME builds from a patched copy instead of crates.io, wired in through the
root `Cargo.toml`'s `[patch.crates-io]`. Each directory is the crate as published, plus the
upstream license files and the changes listed here. `vendor/` is excluded from the workspace,
so the workspace's clippy, fmt and doc gates do not run over upstream code.

To drop a patch, delete its directory and its `[patch.crates-io]` line once an upstream
release carries the fix.

## steel-parser 0.8.3

Upstream: [mattwparas/steel](https://github.com/mattwparas/steel), `crates/steel-parser` at
commit `11a57adcf42c22733463c59b1f44ec49eb7043bc` (the published 0.8.3).

**`SyntaxObjectId::fresh()` (`src/parser.rs`) takes ids in per-thread blocks.** Upstream does
one `fetch_add(1)` on the global `SYNTAX_OBJECT_ID` for every syntax object created or
cloned. Engines compiling on several threads at once all write that one atomic, and the
cache-line contention made the test suite's Steel-heavy tests take several times their
single-threaded CPU. Each thread now reserves 4096 ids at a time. Ids stay unique, and
strictly increasing within a thread, which is all an engine (confined to one thread) sees.
