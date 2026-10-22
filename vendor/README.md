# Vendored crates

Third-party crates HUME builds from a patched copy instead of crates.io, wired in through the
root `Cargo.toml`'s `[patch.crates-io]`. Each directory is the crate as published, plus the
upstream license files and the changes listed here. `vendor/` is excluded from the workspace,
so the workspace's clippy, fmt and doc gates do not run over upstream code.

To drop a patch, delete its directory and its `[patch.crates-io]` line once an upstream
release carries the fix.

## steel-core 0.8.3

Upstream: [mattwparas/steel](https://github.com/mattwparas/steel), `crates/steel-core` at
commit `11a57adcf42c22733463c59b1f44ec49eb7043bc` (the published 0.8.3).

**`Cargo.toml` allows `dead_code`, `deprecated` and `unused_imports`.** Cargo caps a registry
crate's warnings but shows a path dependency's, and upstream's code raises those three.

**`with-handler` (`src/scheme/stdlib.scm`) reinstates its own meta-continuation before
running the handler.** `with-handler` wraps its body in a `reset`, which installs a
meta-continuation in the `*meta-continuation*` thread-local and restores the previous one only
when the body returns. A raise that escapes an inner `with-handler`, such as its handler
re-raising the error, abandons the inner `reset` with its meta-continuation still installed.
Upstream, the outer handler's `shift` then calls that stale one and jumps into a continuation
that has left the stack, panicking with "Failed to find an open continuation on the stack".
Each `with-handler` here saves the meta-continuation its `reset` installed and puts it back
first thing in the handler.

**The compiler prints nothing to stdout.** Upstream's `println!` in
`LowerRestArguments::handle_plist_get_positional_arg_list` (`src/compiler/passes/analysis.rs`)
fires on every compile of a lambda with an optional or keyword argument before a rest
argument, and `CodeGenerator::visit_let` (`src/compiler/code_gen.rs`) prints each let-bound
variable that is both captured and mutated. Both are deleted: stdout is the editor's terminal
display.

**The analysis pass marks a tail call's last-usage arguments by name lookup.** Upstream's
`AnalysisPass::visit_list` (`src/compiler/passes/analysis.rs`) walks every name in the scope
map at each tail call to find the variables an argument used last, and that map keeps every
name bound anywhere in the program being compiled. The cost grows with tail calls times
distinct names, and it was the compiler's largest hotspot at startup. `visit_atom` only sets
`last_used` on the binding of the atom's own name, so looking up each atom argument's name
finds the same bindings. With the walk replaced by that lookup, a release startup with core
plugins loaded takes about 35 ms less.

## steel-parser 0.8.3

Upstream: [mattwparas/steel](https://github.com/mattwparas/steel), `crates/steel-parser` at
commit `11a57adcf42c22733463c59b1f44ec49eb7043bc` (the published 0.8.3).

**`SyntaxObjectId::fresh()` (`src/parser.rs`) takes ids in per-thread blocks.** Upstream does
one `fetch_add(1)` on the global `SYNTAX_OBJECT_ID` for every syntax object created or
cloned. Engines compiling on several threads at once all write that one atomic, and the
cache-line contention made the test suite's Steel-heavy tests take several times their
single-threaded CPU. Each thread now reserves 4096 ids at a time. Ids stay unique, and
strictly increasing within a thread, which is all an engine (confined to one thread) sees.
