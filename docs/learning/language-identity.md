# Language Identity and Detection

## What a buffer "is"

Every buffer carries a language name — a plain string like `"rust"`,
`"python"`, or `"markdown"`. This name is the **single source of truth** for
the buffer's language identity: it lives in one place, and every other
subsystem that cares about language (the statusline, the hook system, the
syntax highlighter) reads it from there.

The name is intentionally just a string, not a pointer to a grammar object or
an enum variant. That choice matters for two reasons.

First, **identity is decoupled from capability**. A buffer can know it is a
Rust file even if no Rust grammar is installed. The statusline can display the
language name, hooks can fire, and plugins can branch on it — all without a
parser. When a grammar is installed later, the buffer gains syntax highlighting
without losing or changing its identity.

Second, **plugins can introduce new languages without recompiling HUME**. A
plugin declares `"my-dsl"` as a language name at runtime. Because the name is
just a string, no Rust enum needs updating, no match arm needs adding. The
language registry learns the name; buffers whose filenames match are tagged
accordingly; the rest of the editor follows along.

## How a buffer learns its language

When a buffer is opened, HUME runs detection to determine the language. The
detection logic tries three strategies in order, stopping at the first match:

1. **Glob patterns** — registered patterns like `Makefile`, `*.config.js`, or
   `{Dockerfile,dockerfile}` are matched against the full filename. This tier
   exists for files that have a meaningful name but no extension, or whose
   extension alone is ambiguous. Glob patterns are ordered by registration
   time; the last registered pattern wins, so plugins can override built-in
   mappings.

2. **Extensions** — if no glob matches, the file extension is looked up
   (`.rs` → Rust, `.py` → Python, etc.). This handles the common case: most
   source files are unambiguous from their extension.

3. **Shebangs** — if the file has no extension (or the extension doesn't
   match), the first line is checked for a shebang (`#!/usr/bin/env python3`,
   `#!/bin/bash`). This covers extensionless scripts like `build` or
   `configure`.

If none of the three tiers match, the buffer is left without a language. That
is a valid state: plain text files, scratch buffers, and binary files are all
language-less.

Detection also runs when a buffer is reloaded, because a file can change what
language it is.

## The funnel

Every change to a buffer's language — whether from automatic detection, the
`:set buffer language=` command, or a plugin API call — goes through a single
function. Nothing writes the language field directly; all callers go through
the funnel.

The funnel does four things in sequence:

1. Writes the new language name to the buffer.
2. Activates any lazy plugins that declared this language as one of their
   activation entries — so a plugin listed as `#:languages '("rust")` loads
   its body the first time a buffer becomes Rust.
3. Queues the `OnLanguageSet` hook so plugins can react; the hook is drained
   at the tail of the current event, after syntax setup has already run.
4. Sets up (or tears down) syntax parsing for the buffer based on the new
   language.

Having one funnel makes it impossible for any code path to change the language
and forget to update syntax state, or to change it without notifying plugins.
The invariant — language field, plugin activation, syntax state, and hook
notification always move together — is structural, not just conventional.

## The `OnLanguageSet` hook

The `on-language-set` hook fires every time a buffer's language is set or
cleared. It receives two arguments: the buffer id, and the language name as a
string (or `#f` if the language was cleared).

When a buffer is first opened, the hook fires *before* `on-buffer-open`. That
ordering is deliberate: if a plugin registers an `on-buffer-open` handler that
branches on language, the language is guaranteed to be set by the time that
handler runs. The plugin doesn't need to check "is the language available yet?"
— it always is.
