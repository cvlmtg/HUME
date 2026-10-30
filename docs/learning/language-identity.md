# Language Identity and Detection

## What a buffer "is"

Every buffer carries a language name: `"rust"`, `"python"`, `"markdown"`. This
name is the **single source of truth** for the buffer's language. It lives in
one place, and every subsystem that cares about language (the statusline, the
hook system, the syntax highlighter, the LSP client) reads it from there.

The name is only a name, not a pointer to a grammar object or a compiled-in
enum variant. That choice matters for two reasons.

First, **identity is separate from capability**. A buffer can know it is a
Rust file even if no Rust grammar is installed. The statusline can show the
language, hooks can fire, and plugins can branch on it, all without a parser.
When a grammar is installed later, the buffer gains syntax highlighting
without its identity changing.

Second, **plugins can add languages without recompiling HUME**. A plugin
declares `"my-dsl"` as a language at runtime. Because languages are named,
not compiled in, no Rust enum needs a new variant and no match needs a new
arm. The language registry learns the name, buffers whose file names match
are tagged with it, and the rest of the editor follows.

## How a buffer learns its language

When a buffer is opened, HUME runs detection. It tries three strategies in
order and stops at the first match:

1. **Glob patterns.** Registered patterns like `Makefile`, `*.config.js` or
   `{Dockerfile,dockerfile}` are matched against the end of the file's path,
   so `Makefile` matches in any directory and `.github/workflows/*.yaml`
   matches only under that directory. This tier exists for files with a
   meaningful name but no extension, or whose extension alone is ambiguous.
   When two languages' patterns match the same file, the language defined
   last wins. Which definition of one language counts is decided by who made
   it, not when: the user's beats a plugin's, and a plugin's beats the
   built-in one.

2. **Extensions.** If no glob matches, the file extension is looked up (`.rs`
   is Rust, `.py` is Python, and so on). This handles the common case: most
   source files are unambiguous from their extension.

3. **Shebangs.** If the file has no extension, or an extension no language
   claims, the first line is checked for a shebang (`#!/usr/bin/env python3`,
   `#!/bin/bash`). This covers extensionless scripts like `build` or
   `configure`. A language lists interpreter names, and a version after the
   name (`python3`, `lua5.4`) still finds the language listing `python` or
   `lua`, unless some language lists that exact version.

If no tier matches, the buffer has no language. That is a valid state: plain
text files, scratch buffers and binary files have none.

Detection also runs when a buffer is reloaded, because a file can change what
language it is. A language set by hand stays: detection never replaces one
the user or a script chose.

## The funnel

The language is an ordinary buffer option, readable and settable like any
other. Every change to it, whether from detection, the
`:set buffer language=` command, or a script setting the `language` buffer
option, goes through a single function. Nothing writes the language
directly. Clearing the option removes the language.

The funnel does five things in order:

1. Writes the new language name to the buffer.
2. Activates any lazy plugins that listed this language as an activation
   entry, so a plugin declared with `#:languages '("rust")` loads its body
   the first time a buffer becomes Rust.
3. Queues the `on-language-set` hook so plugins can react. Queued hooks run
   at the end of the current event, after syntax setup.
4. Sets up (or tears down) syntax parsing for the new language.
5. Attaches the buffer to every language server the new language's list
   names, starting any that is not running yet (see [LSP](lsp.md)).

The identifier sent to a language server is usually the language name, but a
language can register a separate one for that purpose, because some servers
expect a different spelling (TypeScript's servers expect `"typescriptreact"`
where HUME says `"tsx"`). That is the one place the name and the wire
identity may differ; every other subsystem reads the name.

With one funnel, no code path can change the language and forget to update
syntax state or notify plugins. The language, plugin activation, syntax
state, hook notification and server attachment always move together, because
there is only one way to change the language.

## The `on-language-set` hook

`on-language-set` fires every time a buffer's language changes: set to a new
value, or cleared. Setting the same value again fires nothing. A handler
receives two arguments: a pane value for the buffer, and the language name as
a string (the empty string when the language was cleared). The language
belongs to the buffer, not to any view of it, so that pane value names the
buffer only, not a particular pane showing it. It works with anything that
needs only the buffer.

When a buffer is first opened, the hook fires *before* `on-buffer-open`. A
plugin whose `on-buffer-open` handler branches on language can rely on the
language being set by the time the handler runs, with no need to check
whether it is available yet.

---

*See also: [Tree-sitter: Grammars, Queries, and Plum](tree-sitter-pipeline.md)
for the capability half of a language; [Plugin
Architecture](plugin-architecture.md) for `#:languages` activation.*
