# The Scheme prelude

`prelude.scm` is evaluated at startup, right after `bootstrap.scm` and before `languages.scm`
(see this directory's `README.md` for the full load order). It defines the ergonomic forms
(`syntax-rules` macros and plain keyword-argument functions alike) that wrap the raw
Rust-registered builtins; every form it defines is visible in `init.scm` (evaluated globally)
and inside plugin modules loaded via `(require)`. It also defines one command,
`insert-register`, and its default keys.

## The `%` convention

Identifiers prefixed `%` are internal Rust forms. User code should always call the unprefixed
macro or function the prelude wraps instead: `define-language!` over `%define-language!`,
`configure-language!` over `%configure-language!`, `register-grammar!` over
`%register-grammar!`.

## What is deliberately not here

`(call! name args…)` is not defined in the prelude. It's a core dispatch primitive defined in
`hume-scripting/src/builtins/bootstrap.scm` (embedded via `include_str!` in `builtins/mod.rs`)
so it is unconditionally available, including in test engines that never load the prelude. The
prelude itself is optional: a missing `runtime/` directory makes it a silent no-op (see the
README's load-order note). `call!` must not share that fate, so it lives where nothing can skip
it.

## `insert-register`

`Ctrl-r` in Insert mode and on the command line reads one more key with `on-next-char!` and
inserts with `paste-text!`: `f` gives `(buffer-display-path pane #:relative (cwd))`, `p` that
path's `parent-name` joined with `""` for the trailing separator (`.` when the file sits in the
working directory), and a key for which `register-name?` holds `read-register`'s whole list,
which `paste-text!` places as `p` places a register. Any other key raises `insert-register`'s
own error, so the message names the command the user pressed. A cancelled read hands back `#f`
and inserts nothing. The command is `#:repeatable`, so `.` in Insert mode reuses the key read.

It lives here, not in Rust, so it stays plain Scheme a user can read and copy. Two consequences:
it is defined at startup like an `init.scm` command, so `command-plugin` reports it as `"user"`
and an `init.scm` `define-command!` of the same name raises; and without a `runtime/`
directory neither it nor its keys exist. The bindings are ordinary `bind-key!` calls, so a
later binding of `ctrl-r` in `init.scm` replaces them.

## What it defines

Seven forms: `bind-keys!`, `bind-keys-extend!`, `unbind-keys!`, `define-language!`,
`configure-language!`, `register-grammar!`, `configure-statusline!`. For signatures and usage examples, see the user manual:
- [Configuration — Key bindings](https://cvlmtg.github.io/HUME/configuration.html#key-bindings)
- [Settings — Statusline](https://cvlmtg.github.io/HUME/settings.html#statusline)
- [Syntax Highlighting — Teach HUME a new language](https://cvlmtg.github.io/HUME/syntax-highlighting.html#teach-hume-a-new-language)
