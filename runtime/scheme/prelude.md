# The Scheme prelude

`prelude.scm` is evaluated at startup, right after `bootstrap.scm` and before `languages.scm` —
see this directory's `README.md` for the full load order. It defines `syntax-rules` macros
that improve ergonomics over the raw Rust-registered builtins; every macro it defines is
visible in `init.scm` (evaluated globally) and inside plugin modules loaded via `(require)`.

## The `%` convention

Identifiers prefixed `%` are internal Rust forms. User code should always call the unprefixed
macro or function the prelude wraps instead — `define-language!` over `%define-language!`,
`register-grammar!` over `%register-grammar!`.

## What is deliberately not here

`(call! name args…)` is not defined in the prelude — it's a core dispatch primitive defined in
`hume-scripting/src/builtins/bootstrap.scm` (embedded via `include_str!` in `builtins/mod.rs`)
so it is unconditionally available, including in test engines that never load the prelude. The
prelude itself is optional: a missing `runtime/` directory makes it a silent no-op (see the
README's load-order note). `call!` must not share that fate, so it lives where nothing can skip
it.

## What it defines

Five forms: `bind-keys!`, `bind-keys-extend!`, `unbind-keys!`, `define-language!`,
`register-grammar!`. For signatures, keyword arguments, and usage examples, see the user
manual:
- [Configuration — Key bindings](https://cvlmtg.github.io/HUME/configuration.html#key-bindings)
- [Syntax Highlighting — Teach HUME a new language](https://cvlmtg.github.io/HUME/syntax-highlighting.html#teach-hume-a-new-language)
