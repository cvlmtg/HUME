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
finds the same bindings. Timings are under Benchmarks below.

**Constant evaluation stops once a pass changes nothing.** Upstream's
`apply_const_evaluation` (`src/compiler/compiler.rs`) runs `ConstantEvaluatorManager::run`
three times, and `run` (`src/steel_vm/const_evaluation.rs`) visits each top-level form at
least twice, because four rewrites never set `changed`: replacing an identifier with its
constant value, folding an `if` on a constant test, reducing a call of a no-argument lambda
with a constant body to that body, and dropping some of a `let`'s unused bindings. With those
reporting the change, `run` visits a form again only after a visit changed it, and the
manager stops after its second run when that run changed nothing; the second run always
happens, since the first binds the constants a form defined later can fold into an earlier
one. The folded output is the same as upstream's three runs for every compile in HUME's
startup (618 compiles) and the `hume-scripting` test suite (11,073): both schedules ran on the
same input and their results compared equal. Timings are under Benchmarks below.

### Benchmarks

Apple M4 Pro, macOS 26.5.2, rustc 1.99.1, release builds, measured 2026-10-23. Each figure is
the median of 30 runs.

`Engine::new()` timed in a fresh process, which includes building the kernel image. The
program does nothing else. steel-parser is the crates.io 0.8.3 throughout, and each patch is
applied alone to the crates.io steel-core 0.8.3. The variants ran interleaved.

| steel-core | `Engine::new()` | Saved |
|---|---|---|
| 0.8.3 | 139.0 ms | |
| with the analysis-pass patch | 116.6 ms | 22.4 ms (16%) |
| with the constant-evaluation patch | 119.4 ms | 19.6 ms (14%) |
| with both patches | 99.4 ms | 39.6 ms (28%) |
| with both patches and the `unsandboxed-kernel` feature | 85.8 ms | 53.2 ms (38%) |

`unsandboxed-kernel` is an existing steel-core feature, not a patch: macro expansion then
copies the kernel image `Engine::new()` already built, instead of building a sandboxed one.

HUME's startup with each patch against the commit before it: `hume --keys ':q<ret>'` opening
a file headless, then exiting. A config that loads core plugins adds their Scheme to what
Steel compiles, and opening a `.rs` file activates more of them. The two columns are measured
at different HUME versions, so their absolute times differ.

| Startup | Analysis-pass patch | Constant-evaluation patch |
|---|---|---|
| `.txt` file, no config | 415.7 → 394.9 ms | 175.0 → 151.1 ms |
| `.txt` file, core plugins | 489.5 → 458.4 ms | 239.6 → 211.7 ms |
| `.rs` file, core plugins | 580.5 → 540.1 ms | 395.0 → 357.9 ms |

In six profiled startups the analysis pass's self time fell from 269 to 72 samples (1 ms
interval). With core plugins loaded, constant evaluation went from 48 to 19 ms per startup.

## steel-parser 0.8.3

Upstream: [mattwparas/steel](https://github.com/mattwparas/steel), `crates/steel-parser` at
commit `11a57adcf42c22733463c59b1f44ec49eb7043bc` (the published 0.8.3).

**`SyntaxObjectId::fresh()` (`src/parser.rs`) takes ids in per-thread blocks.** Upstream does
one `fetch_add(1)` on the global `SYNTAX_OBJECT_ID` for every syntax object created or
cloned. Engines compiling on several threads at once all write that one atomic, and the
cache-line contention made the test suite's Steel-heavy tests take several times their
single-threaded CPU. Each thread now reserves 4096 ids at a time. Ids stay unique, and
strictly increasing within a thread, which is all an engine (confined to one thread) sees.
