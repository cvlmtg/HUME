# Syntax Highlighting

HUME colors your code with **tree-sitter** — accurate highlighting that stays correct as you type and handles partial or malformed code without choking.

Highlighting is **opt-in per language**: HUME knows how to recognize many languages out of the box, but it doesn't ship pre-compiled parsers. For a language to light up, you install a **grammar** — a small package tree-sitter uses to parse that language. PLUM handles this for you.

## Prerequisites

Installing a grammar runs a few external tools. Most are already on your system; if one is missing the install will tell you. You need:

- `git`
- `curl`
- `tree-sitter` (the tree-sitter CLI)
- A C compiler (`cc`, `gcc`, or `clang`) — pre-installed on macOS and most Linux distributions.

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
