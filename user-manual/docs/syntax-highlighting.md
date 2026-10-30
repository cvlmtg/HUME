# Syntax Highlighting

HUME colors your code with **tree-sitter**: accurate highlighting that stays correct as you type and handles partial or malformed code without choking.

Highlighting is **opt-in per language**. HUME recognizes many languages out of the box but ships no grammars. For a language to light up, you install its **grammar**, a small prebuilt package tree-sitter uses to parse that language. [PLUM](core-plugins.md#core-plum) downloads it for you.

PLUM is a plugin like any other and doesn't load by itself. Installing, listing, and cleaning
up grammars all need it loaded in your [`init.scm`](configuration.md) first, along with
`core:stdlib`, which its install and cleanup commands depend on:

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:plum")
```

Once a grammar is installed, PLUM is out of the picture: HUME registers every installed
grammar itself at startup, so highlighting keeps working even if you later remove PLUM from
`init.scm`. Each grammar loads the first time a file or an embedded block needs it, so
installed grammars you don't use cost nothing at startup.

## Prerequisites

Installing a grammar downloads it with `curl` and unpacks it with `tar`. Both come with macOS, with Windows 10 and later, and with most Linux distributions; if one is missing, the install fails with an error naming it.

Prebuilt grammars exist for Linux on x86-64, macOS on Apple silicon, and Windows on x86-64. On any other platform, [install them from a local folder](#install-from-a-local-folder).

## Install a grammar

Open a file in the language you want highlighted, then run:

```
:plum-install-grammar
```

When it finishes, the current buffer is highlighted immediately, and other open buffers in the same language pick it up on the next frame.

You don't need to open a file in that language first: name the language directly. Tab completes the languages that have a grammar:

```
:plum-install-grammar python
```

This is the easiest way to get a grammar for a language you only use inside a fenced code block (see [Embedded languages](#embedded-languages) below), or whenever switching buffers to install one is inconvenient.

`:plum-install-grammar` always downloads the grammar again, and replaces the installed copy only once the new one is complete. Running it again is also how you repair a broken install or update a grammar after updating HUME.

To install several grammars at startup, skipping any already installed, add this to your `init.scm`:

```scheme
(call! "plum/ensure-grammars" '("rust" "toml" "python"))
```

## Install from a local folder

On a platform with no prebuilt grammars, or to install without a network connection, build the grammars yourself with `hume-grammar-build`, from [treesitter.hume](https://github.com/cvlmtg/treesitter.hume) (its README explains how to get it and what it needs):

```sh
hume-grammar-build build --out /path/to/hume-grammars rust python markdown markdown.inline
```

`--all` builds every grammar instead of the ones you name. Then tell PLUM to look in that folder first:

```scheme
(load-plugin! "core:plum" #:config (hash 'grammar-source "/path/to/hume-grammars"))
```

The path must be absolute, and `~` is not expanded. `:plum-install-grammar` and `plum/ensure-grammars` then install each grammar the folder holds from it, and download the others as usual, so the folder needs only the grammars you built yourself. A grammar from a folder built by a different version of `hume-grammar-build` than the one your HUME expects still installs, with a warning in `:messages`.

## Embedded languages

Some languages embed others: a fenced code block in Markdown, or Markdown's own bold, italic, and inline-code spans. HUME resolves these automatically once the grammars involved are installed, with no extra configuration.

For Markdown, `:plum-install-grammar` installs Markdown itself and the grammar its emphasis and inline-code spans need. A fenced ` ```rust ` block then highlights as Rust once the Rust grammar is installed too, with `:plum-install-grammar rust`. Install one for each language you paste into fences; you don't need to open a file in that language first.

## When detection gets it wrong

If HUME guesses the buffer's language wrong, or can't guess it, override it:

```
:set buffer language=python
```

Use the exact language name; press `Tab` after `language=` to complete from the languages HUME recognizes. The override lasts for that buffer only.

## Teach HUME a new language

Add it to your `init.scm`:

```scheme
(define-language! "my-lang" '("myl") '("*.my") '("myinterpreter"))
```

The arguments, in order, are:
- the language name
- a list of file extensions, written **without** a leading dot (`"myl"`, not `".myl"`). An extension can have several parts, such as `"js.map"`; when several match, the longest wins. A leading or trailing dot, or a `/`, raises an error
- a list of glob patterns, matched against the end of the file's path: `"Makefile"` matches a file with that name in any directory, and `".github/workflows/*.yaml"` matches a file in such a directory. A pattern that starts with `/` matches the whole path. A file opened through a symbolic link is matched by the path you opened, then by the path the link leads to
- a list of interpreter names, matched against the program a file's first line runs: `"python"` matches `#!/usr/bin/python3` and `#!/usr/bin/env python3.12`. A version after the name is ignored, unless a language names that version itself. A name with a space or a `/` raises an error.

You can drop trailing arguments you don't need: `(define-language! "my-lang" '("myl"))` is fine.

Settings beyond detection go through `configure-language!`, which works on any language, built-in or your own. Call it after the `define-language!` that defines the language.

If the language server you'll connect expects a different identifier than `"my-lang"` (TypeScript's language servers, for example, expect `"typescriptreact"` for `.tsx` files), set `#:language-id`. It defaults to the language name:

```scheme
(configure-language! "my-lang" #:language-id "my-language-server-id")
```

`#:roots` lists the file or directory names that mark a project's root for this language. HUME walks up from the opened file to the nearest directory holding one, and language servers use that directory as their workspace:

```scheme
(configure-language! "my-lang" #:roots '("my-lang.toml" ".git"))
```

A language with no roots uses the directory HUME was started in.

`#:comment-tokens` and `#:block-comment-tokens` set what `G G` writes and recognises. `G G` writes the first entry of each list; any entry counts as a comment when removing one:

```scheme
(configure-language! "my-lang"
  #:comment-tokens '("--")
  #:block-comment-tokens '(("{-" . "-}")))
```

Built-in languages already come with the roots, identifiers and comments their language servers and `G G` expect; `configure-language!` replaces them. A setting from your `init.scm` beats one from a plugin, whichever runs first. Setting the same thing twice in your `init.scm` keeps the first value and notes the second in `:messages`.

Redefining a language with `define-language!` replaces only its extensions, globs and shebangs; its settings stay. Definitions follow the same order as settings: yours beats a plugin's, whichever runs first, and a second one in your `init.scm` is ignored with a note in `:messages`.

Now `my-lang` is detected like any built-in. `:plum-install-grammar` installs grammars only for the languages HUME ships with. For your own language, or a private or experimental tree-sitter grammar, point HUME at the grammar's library and a highlight query file by hand. To build the library from the grammar's source, run `hume-grammar-build compile <grammar-dir> <library>` (see [Install from a local folder](#install-from-a-local-folder) for where to get it):

```scheme
(register-grammar! "my-lang"
  "/path/to/my_grammar.so"
  "tree_sitter_my_lang"
  "/path/to/highlights.scm")
```

The required fields, in order:
- language name: one you haven't defined with `define-language!` works too, but no file is detected as that language until you define it
- path to the grammar's library
- the C symbol that library exposes (each grammar's repo documents this)
- a highlight query file

Three optional keyword arguments can follow, in any order.

If `my-lang` embeds other languages (like Markdown's fenced code blocks), pass its injections query with `#:injections`:

```scheme
(register-grammar! "my-lang"
  "/path/to/my_grammar.so"
  "tree_sitter_my_lang"
  "/path/to/highlights.scm"
  #:injections "/path/to/injections.scm")
```

If it defines structural text objects and navigation (functions, classes, arguments, …), pass its textobjects query with `#:textobjects`, with or without `#:injections`:

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

If it defines how new lines are indented, pass its indents query with `#:indents`, alone or alongside the others. It uses Helix's `indents.scm` captures; see [Indentation](editing.md#indentation) for what it drives:

```scheme
(register-grammar! "my-lang"
  "/path/to/my_grammar.so"
  "tree_sitter_my_lang"
  "/path/to/highlights.scm"
  #:indents "/path/to/indents.scm")
```

Registered from `init.scm`, a grammar loads the first time a file or an embedded block needs it, and `:messages` reports then if its library or a query file can't be loaded. Registered from a command, it loads right away, and the command fails with the reason.

A grammar you register, from your `init.scm` or from a command you run, beats one a plugin registers, whichever runs first. Registering a language's grammar again from the same place replaces the first one.

## Manage installed grammars

```
:plum-list-grammars
```

Opens a read-only buffer listing the installed grammars, with everything else in the grammars folder in a section of its own: grammars installed under a name that isn't a language HUME knows, older copies of a grammar a reinstall couldn't delete yet, and files an earlier version or an interrupted install left. A third section lists what grammar installs that were stopped partway left behind: the files they were downloading or unpacking, and the copies they loaded to check a grammar.

```
:plum-cleanup-grammars
```

Removes everything in the second and third sections, keeping each language's installed grammar. On Windows, a grammar copy this session has loaded can't be deleted until HUME exits, so it is reported and removed the next time. Run it after a HUME update to reclaim space and avoid stale libraries.

## Large files

Buffers above a size threshold skip highlighting to stay responsive. The default is 1 MiB:

```scheme
(set-option! "syntax-highlight-max-bytes" 5242880)   ; raise to 5 MiB
```

See [Settings](settings.md) for the full settings reference.

## Troubleshooting

**The file opens with no colors.** The language has no installed grammar. Run `:plum-list-grammars`. If the language isn't under **Installed grammars**, run `:plum-install-grammar <name>` (or just `:plum-install-grammar` while that buffer is focused). If the install says there is no grammar for the language, HUME has none to install for it. Check the language name on the status line and fix a wrong detection with `:set buffer language=<name>`. For a language HUME ships no grammar for, register one by hand (see [Teach HUME a new language](#teach-hume-a-new-language)).

**`:plum-install-grammar` fails.** The message names the cause, usually no network connection or a tool missing from your `PATH`; check the [prerequisites](#prerequisites). On a platform with no prebuilt grammars, [install from a local folder](#install-from-a-local-folder). A failed install leaves the installed copy of the grammar, if any, working.

**Detection picks the wrong language.** Override with `:set buffer language=<name>`, or add the file pattern to your `init.scm` with `define-language!`.

**Colors look wrong after a HUME update.** The update may expect newer grammars or highlighting rules than the ones you installed. Run `:plum-install-grammar <name>` again for each installed grammar, and `:plum-cleanup-grammars`.

**A fenced code block doesn't highlight, but Markdown emphasis does.** The fenced language's own grammar isn't installed. Markdown's bold/italic/inline-code always come with the Markdown grammar, but a fence's language (` ```rust `, ` ```python `, …) is a separate grammar. Install it by name, e.g. `:plum-install-grammar rust` for a ` ```rust ` fence; you don't need to open a file in that language first.
