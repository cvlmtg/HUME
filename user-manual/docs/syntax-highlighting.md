# Syntax Highlighting

HUME colors your code with **tree-sitter** — accurate highlighting that stays correct as you type and handles partial or malformed code without choking.

Highlighting is **opt-in per language**: HUME knows how to recognize many languages out of the box, but it doesn't ship pre-compiled parsers. For a language to light up, you install a **grammar** — a small package tree-sitter uses to parse that language. PLUM handles this for you.

## Prerequisites

Installing a grammar runs a few external tools. Most are already on your system; if one is missing the install will tell you. You need:

- `git`
- `curl`
- `tree-sitter` (the tree-sitter CLI)
- A C compiler (`cc`, `gcc`, or `clang`) — pre-installed on macOS and most Linux distributions.

How you install these depends on your operating system:

- **macOS**: [Homebrew](https://brew.sh) covers all four — `brew install git curl tree-sitter`. A C compiler comes with Xcode's Command Line Tools (`xcode-select --install`); most machines already have it.
- **Linux**: use your distribution's package manager. `git` and `curl` are usually preinstalled; `gcc` or `clang` come from your distro's base-devel/build-essential group. The `tree-sitter` CLI isn't in every distro's repos — if yours doesn't have it, install it via `cargo install tree-sitter-cli` (needs a [Rust toolchain](https://rustup.rs)) or `npm install -g tree-sitter-cli`.
- **Windows**: [winget](https://learn.microsoft.com/windows/package-manager/winget/) or [Scoop](https://scoop.sh) can install `git`, `curl`, and `tree-sitter-cli`; for a C compiler, install the [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) (select the "Desktop development with C++" workload), or run HUME under WSL and follow the Linux instructions above.

## Install a grammar

Open a file in the language you want highlighted, then run:

```
:plum-install-grammar
```

When it finishes, the current buffer is highlighted immediately, and any other open buffers in the same language pick it up on the next frame.

To install several grammars at once — skipping any already compiled — call it from your `init.scm` so it runs at startup:

```scheme
(call! "plum-ensure-grammars" '("rust" "toml" "python"))
```

After the first install, launching HUME just loads the compiled grammars silently; there's nothing more to do.

### Re-install

```
:plum-update-grammar
```

Re-downloads the grammar source and recompiles it. Use it after updating HUME, or to recover from a broken compile. The old source is purged first.

## Embedded languages

Some languages embed others — a fenced code block in Markdown, or Markdown's own bold, italic, and inline-code spans. HUME resolves these automatically once the grammars involved are installed, with no extra configuration.

For Markdown, `:plum-install-grammar` installs Markdown itself and the grammar its emphasis and inline-code spans need. A fenced ` ```rust ` block then highlights as Rust as soon as the Rust grammar is installed too — install it for whichever languages you paste into fences.

## When detection gets it wrong

If HUME can't guess correctly the buffer language, you can override it manually:

```
:set buffer language=python
```

Use the exact language name — `:plum-list-grammars` shows every name HUME recognizes. The override lasts for that buffer only.

## Teach HUME a new language

Add it to your `init.scm`:

```scheme
(define-language! "my-lang" '(".myl") '("*.my") '("myinterpreter"))
```

The arguments, in order, are:
- the language name
- a list of file extensions
- a list of glob patterns
- a list of shebang lines.

Trailing arguments you don't need can be dropped — `(define-language! "my-lang" '(".myl"))` is fine.

Now `my-lang` is detected like any built-in. For a grammar that isn't in the catalog — a private or experimental tree-sitter grammar — point HUME at the compiled library and a highlight query file by hand:

```scheme
(register-grammar! "my-lang"
  "/path/to/my_grammar.so"
  "tree_sitter_my_lang"
  "/path/to/highlights.scm")
```

The fields are, in order:
- language name (define it with `define-language!` first)
- path to the compiled library
- the C symbol that library exposes (each grammar's repo documents this)
- a highlight query file.

If `my-lang` embeds other languages (like Markdown's fenced code blocks), add a fifth argument pointing at its injections query:

```scheme
(register-grammar! "my-lang"
  "/path/to/my_grammar.so"
  "tree_sitter_my_lang"
  "/path/to/highlights.scm"
  "/path/to/injections.scm")
```

Omit it for a language with nothing embedded.

## Manage installed grammars

```
:plum-list-grammars
```

Logs the names HUME knows, which are compiled on disk, which are missing, and which are orphans (compiled but no longer in the catalog).

```
:plum-cleanup-grammars
```

Drops the compiled files for orphan grammars. Run it after a HUME update to reclaim space and avoid stale libraries.

## Large files

Buffers above a size threshold skip highlighting to stay responsive. The default is 1 MiB:

```scheme
(set-option! "syntax-highlight-max-bytes" 5242880)   ; raise to 5 MiB
```

See [Configuration](configuration.md) for the full settings reference.

## Troubleshooting

**The file opens with no colors.** No compiled grammar for the language. Run `:plum-list-grammars` and look at the **missing** line. If the language is missing, run `:plum-install-grammar`. If the language isn't *declared* at all, HUME doesn't recognize the file — set it with `:set buffer language=<name>` or define it in your `init.scm`.

**`:plum-install-grammar` fails.** The message names the missing piece. Usually a missing tool on your `PATH` — check the [prerequisites](#prerequisites). A bad source tree recovers with `:plum-update-grammar`.

**Detection picks the wrong language.** Override with `:set buffer language=<name>`, or add the file pattern to your `init.scm` with `define-language!`.

**Colors look wrong after a HUME update.** The mirrored catalog may have moved. Run `:plum-cleanup-grammars`, then `:plum-install-grammar` again.

**A fenced code block doesn't highlight, but Markdown emphasis does.** The fenced language's own grammar isn't installed — Markdown's bold/italic/inline-code always come with the Markdown grammar itself, but a fence's language (` ```rust `, ` ```python `, …) is a separate grammar. Open a scratch file in that language (e.g. a `.rs` file for a ` ```rust ` fence) and run `:plum-install-grammar` there too.
