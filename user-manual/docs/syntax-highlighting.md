# Syntax Highlighting

HUME colors your code with **tree-sitter**: accurate highlighting that stays correct as you type and handles partial or malformed code without choking.

Highlighting is **opt-in per language**: HUME knows how to recognize many languages out of the box, but it doesn't ship pre-compiled parsers. For a language to light up, you install a **grammar**, a small package tree-sitter uses to parse that language. [PLUM](core-plugins.md#core-plum) handles this for you.

PLUM is a plugin like any other, and it doesn't load by itself. Everything on this page
(installing, listing, and cleaning up grammars) needs it loaded in your
[`init.scm`](configuration.md) first, along with `core:stdlib`, which its install and
cleanup commands depend on:

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:plum")
```

Once a grammar is installed, though, PLUM is no longer in the picture: HUME registers every
already-compiled grammar itself at startup, so highlighting keeps working even if you later
remove PLUM from `init.scm`. Each grammar loads the first time a file or an embedded block
needs it, so installed grammars you don't use cost nothing at startup.

## Prerequisites

Installing a grammar runs a few external tools. Most are already on your system; if one is missing, the install fails with an error naming it. You need:

- `git`
- `curl`
- A C and C++ compiler (`cc` and `c++`, from `gcc` or `clang`), pre-installed on macOS and most Linux distributions. Some grammars are partly written in C++. Set `CC` or `CXX` to use a specific one.

How you install these depends on your operating system:

- **macOS**: [Homebrew](https://brew.sh) covers `git` and `curl`: `brew install git curl`. The compilers come with Xcode's Command Line Tools (`xcode-select --install`).
- **Linux**: use your distribution's package manager. `git` and `curl` are usually preinstalled; `gcc` and `g++`, or `clang`, come from your distro's base-devel/build-essential group.
- **Windows**: [winget](https://learn.microsoft.com/windows/package-manager/winget/) or [Scoop](https://scoop.sh) can install `git` and `curl`; for the compilers, install the [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) (select the "Desktop development with C++" workload), or run HUME under WSL and follow the Linux instructions above. If you'd rather skip the Build Tools install, `clang`, `gcc`, or [zig](https://ziglang.org/download/) (e.g. `winget install zig.zig`) work too: without the Build Tools, HUME uses whichever of them it finds on `PATH`.

## Install a grammar

Open a file in the language you want highlighted, then run:

```
:plum-install-grammar
```

When it finishes, the current buffer is highlighted immediately, and any other open buffers in the same language pick it up on the next frame.

You don't need to open a file in that language first. Name the language directly; Tab completes the languages that have a grammar:

```
:plum-install-grammar python
```

This is the easiest way to install a grammar for a language you only use inside a fenced code block (see [Embedded languages](#embedded-languages) below), or any time switching buffers just to install a grammar is inconvenient.

`:plum-install-grammar` always re-downloads and recompiles from scratch, purging any old source first, so running it again is also how you recover from a broken compile or refresh a grammar after updating HUME.

To install several grammars at once (skipping any already compiled), call it from your `init.scm` so it runs at startup:

```scheme
(call! "plum/ensure-grammars" '("rust" "toml" "python"))
```

After the first install, there's nothing more to do: HUME loads each compiled grammar the first time a file or an embedded block needs it.

## Embedded languages

Some languages embed others: a fenced code block in Markdown, or Markdown's own bold, italic, and inline-code spans. HUME resolves these automatically once the grammars involved are installed, with no extra configuration.

For Markdown, `:plum-install-grammar` installs Markdown itself and the grammar its emphasis and inline-code spans need. A fenced ` ```rust ` block then highlights as Rust as soon as the Rust grammar is installed too. Run `:plum-install-grammar rust` for whichever languages you paste into fences, no need to open a file in that language first.

## When detection gets it wrong

If HUME can't guess correctly the buffer language, you can override it manually:

```
:set buffer language=python
```

Use the exact language name. Press `Tab` after `language=` to complete from the languages HUME recognizes. (`:plum-install-grammar` completes only the languages HUME can install a grammar for, a slightly smaller set.) The override lasts for that buffer only.

## Teach HUME a new language

Add it to your `init.scm`:

```scheme
(define-language! "my-lang" '("myl") '("*.my") '("myinterpreter"))
```

The arguments, in order, are:
- the language name
- a list of file extensions, written **without** a leading dot (`"myl"`, not `".myl"`). An extension can have several parts, such as `"js.map"`; when several match, the longest wins. A leading or trailing dot, or a `/`, raises an error
- a list of glob patterns, matched against the end of the file's path: `"Makefile"` matches a file with that name in any directory, and `".github/workflows/*.yaml"` matches a file in such a directory. A pattern that starts with `/` matches the whole path. A file opened through a symbolic link is matched by the path you opened, then by the path the link leads to
- a list of shebang lines.

Trailing arguments you don't need can be dropped: `(define-language! "my-lang" '("myl"))` is fine.

Settings beyond detection go through `configure-language!`, which works on any language, built-in or your own. Call it after the `define-language!` that defines the language.

If the language server you'll connect expects a different identifier than `"my-lang"` (for example, TypeScript's language servers expect `"typescriptreact"` for `.tsx` files), set `#:language-id`:

```scheme
(configure-language! "my-lang" #:language-id "my-language-server-id")
```

It defaults to the language name when you don't set it.

`#:roots` lists the file or directory names that mark a project's root for this language (HUME walks up from the opened file to the nearest directory holding one). Language servers use the directory it finds as their workspace:

```scheme
(configure-language! "my-lang" #:roots '("my-lang.toml" ".git"))
```

A language with no roots uses the directory HUME was started in.

`#:comment-tokens` and `#:block-comment-tokens` set what `G G` writes and recognises. The first entry of each list is the one `G G` writes; any entry counts as a comment when removing one:

```scheme
(configure-language! "my-lang"
  #:comment-tokens '("--")
  #:block-comment-tokens '(("{-" . "-}")))
```

Built-in languages already come with the roots, identifiers and comments their language servers and `G G` expect; `configure-language!` replaces them. A setting from your `init.scm` beats one from a plugin, whichever runs first. Setting the same thing twice in your `init.scm` keeps the first value and notes the second in `:messages`. Redefining a language with `define-language!` replaces only its extensions, globs and shebangs; its settings stay.

Now `my-lang` is detected like any built-in. `:plum-install-grammar` installs grammars only for the languages HUME ships with, so for your own language, or a private or experimental tree-sitter grammar, point HUME at the compiled library and a highlight query file by hand:

```scheme
(register-grammar! "my-lang"
  "/path/to/my_grammar.so"
  "tree_sitter_my_lang"
  "/path/to/highlights.scm")
```

The required fields, in order:
- language name (define it with `define-language!` first)
- path to the compiled library
- the C symbol that library exposes (each grammar's repo documents this)
- a highlight query file

Three optional keyword arguments follow, in any order, and any of them can be omitted:

If `my-lang` embeds other languages (like Markdown's fenced code blocks), pass its injections query with `#:injections`:

```scheme
(register-grammar! "my-lang"
  "/path/to/my_grammar.so"
  "tree_sitter_my_lang"
  "/path/to/highlights.scm"
  #:injections "/path/to/injections.scm")
```

If it also defines structural text objects and navigation (functions, classes, arguments, …), pass its textobjects query with `#:textobjects`, with or without `#:injections`:

```scheme
(register-grammar! "my-lang"
  "/path/to/my_grammar.so"
  "tree_sitter_my_lang"
  "/path/to/highlights.scm"
  #:textobjects "/path/to/textobjects.scm")
```

```scheme
(register-grammar! "my-lang"
  "/path/to/my_grammar.so"
  "tree_sitter_my_lang"
  "/path/to/highlights.scm"
  #:injections "/path/to/injections.scm"
  #:textobjects "/path/to/textobjects.scm")
```

If it also defines how new lines are indented, pass its indents query with `#:indents`, alone or alongside the others. It uses Helix's `indents.scm` captures; see [Indentation](editing.md#indentation) for what it drives:

```scheme
(register-grammar! "my-lang"
  "/path/to/my_grammar.so"
  "tree_sitter_my_lang"
  "/path/to/highlights.scm"
  #:indents "/path/to/indents.scm")
```

Omit all keywords for a language with nothing to offer there.

Registered from `init.scm`, a grammar loads the first time a file or an embedded block needs it. If its library or a query file can't be loaded, `:messages` says so then. Registered from a command, it loads right away, and the command fails with the reason.

## Manage installed grammars

```
:plum-list-grammars
```

Opens a read-only buffer listing the grammars compiled on disk, with the orphans (compiled under a name HUME ships no grammar for) in a section of their own.

```
:plum-cleanup-grammars
```

Drops the compiled files for orphan grammars. Run it after a HUME update to reclaim space and avoid stale libraries.

## Large files

Buffers above a size threshold skip highlighting to stay responsive. The default is 1 MiB:

```scheme
(set-option! "syntax-highlight-max-bytes" 5242880)   ; raise to 5 MiB
```

See [Settings](settings.md) for the full settings reference.

## Troubleshooting

**The file opens with no colors.** No compiled grammar for the language. Run `:plum-list-grammars`. If the language isn't under **Installed grammars**, run `:plum-install-grammar <name>` (or just `:plum-install-grammar` while that buffer is focused). If `:plum-install-grammar` doesn't offer the language when you press `Tab`, HUME has no grammar to install for it. Check the language name on the status line and fix a wrong detection with `:set buffer language=<name>`. For a language HUME ships no grammar for, register one by hand (see [Teach HUME a new language](#teach-hume-a-new-language)).

**`:plum-install-grammar` fails.** The message names the missing piece. Usually a missing tool on your `PATH`; check the [prerequisites](#prerequisites). A bad source tree recovers by running `:plum-install-grammar` again.

**Detection picks the wrong language.** Override with `:set buffer language=<name>`, or add the file pattern to your `init.scm` with `define-language!`.

**Colors look wrong after a HUME update.** The update may pin a different version of the grammar or its highlighting rules than the one you installed. Run `:plum-cleanup-grammars`, then `:plum-install-grammar` again.

**A fenced code block doesn't highlight, but Markdown emphasis does.** The fenced language's own grammar isn't installed. Markdown's bold/italic/inline-code always come with the Markdown grammar itself, but a fence's language (` ```rust `, ` ```python `, …) is a separate grammar. Install it by name, e.g. `:plum-install-grammar rust` for a ` ```rust ` fence; no need to open a file in that language first.
