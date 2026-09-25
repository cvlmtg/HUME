# Core plugins

The plugins HUME ships with, all written in Steel like any third-party plugin — none of
them get privileged access. Each has its own README (split plugins also have a `docs/`
directory); this page covers what they have in common, so a plugin's own doc doesn't have
to restate it.

For end-user setup and configuration, see the manual's
[Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html) page. For the general
plugin-authoring API (not core-specific), see
[Writing a plugin](https://cvlmtg.github.io/HUME/plugins.html#writing-a-plugin).

## Catalog

| Plugin | Purpose | Loads | Depends on |
|---|---|---|---|
| [`stdlib`](stdlib/README.md) | Shared helpers for plugin authors | lazy | — |
| [`plum`](plum/README.md) | Plugin/theme/grammar installer | lazy | `stdlib` |
| [`lsp`](lsp/README.md) | Language server client | lazy | `stdlib` |
| [`steel-server`](steel-server/README.md) | Registers a Scheme language server | lazy | `lsp` |
| [`pickers`](pickers/README.md) | Fuzzy file/buffer/git pickers | eager | `stdlib` |
| [`git-diff`](git-diff/README.md) | Inline git diff decorations | lazy | `stdlib` |
| [`buffer-words`](buffer-words/README.md) | Buffer-text completion source | eager | `stdlib` |
| [`vim-keybind`](vim-keybind/README.md) | Vim muscle-memory keys | eager | `stdlib` |
| [`helix-surround`](helix-surround/README.md) | Helix-compat surround keys | eager | — |
| [`classic-paste`](classic-paste/README.md) | GUI-style clipboard/kill-ring split | eager | — |

"Loads": **lazy** means the plugin ships a `manifest.scm` and can be brought in with
`declare-plugin`, activating itself later. **Eager** means it has no `manifest.scm` and
must be loaded with `load-plugin` — see below for why.

## Loading model

`declare-plugin` reads a plugin's `manifest.scm`, which lists the commands, typed
commands, events, and languages that should activate it — the plugin's body doesn't run
until one of those actually fires. `load-plugin` skips all of that and runs the plugin's
body immediately.

A plugin has no `manifest.scm`, and so can only be loaded eagerly, when **its own key
bindings are the only way to reach its commands**. A command with no typed form and no
hook has nothing a manifest could list as an activation trigger — declaring it lazily
would leave it permanently dormant. `buffer-words`, `pickers`, `vim-keybind`,
`helix-surround`, and `classic-paste` are all this shape.

Passing an explicit `#:commands`/`#:events`/`#:languages`/`#:typed-commands` list to
`declare-plugin` bypasses the plugin's own `manifest.scm` entirely. This is how a config
can activate a plugin on a narrower trigger than its default — but see the pitfall below
before doing it to a plugin other code depends on.

## Depending on `core:stdlib`

Plugins never `require` each other's Scheme modules — that would break the namespace
isolation each plugin gets. All cross-plugin calls go through `call!` by command name.

`call!`'s lazy-miss retry means a bare `(declare-plugin "core:stdlib")` is enough to
satisfy a dependency, even for a call made at the dependent plugin's own load time:
`call!` notices the target is only declared, not yet loaded, activates it inline, and
retries. Every core plugin that depends on `stdlib` checks `(declared-plugins)` for it at
load time and errors immediately if it's missing, rather than letting a missing dependency
surface later as a `call!` failure buried in a command few users exercise. See the
manual's [Depending on another plugin](https://cvlmtg.github.io/HUME/plugins.html#depending-on-another-plugin)
for the full mechanism, including the stronger `(loaded-plugins)` check.

**Pitfall:** if `core:stdlib` (or any dependency) is declared with an explicit
`#:commands`/`#:events`/`#:languages`/`#:typed-commands` list that omits a command a
dependent needs, there is no activation stub for that command. `call!` then logs an error
and returns `#f` instead of raising — a dependent's config validation or runtime call
silently resolves to `#f` instead of failing loudly. The `(declared-plugins)` check can't
catch this, since the dependency genuinely is declared.

## Patterns shared across core plugins

A few implementation patterns recur across multiple plugins because they all face the
same constraints. Each is written once here; a plugin's own doc names it by section title
instead of re-explaining it.

### Per-buffer state

A plugin tracking state per buffer keeps one `(box (hash))`, keyed by `(buffer-key pane)`
— never by a raw pane value. A command's own pane and a hook's pane-less value for the
same buffer must resolve to the same table entry, and only `buffer-key` guarantees that.
Steel's `hash` is persistent (immutable — `hash-insert` returns a new map rather than
mutating in place), so updating an entry means read-modify-write the whole box, not
mutating a captured reference to the old map.

`git-diff`, `buffer-words`, and `lsp`'s diagnostics drawer all use this shape.

### Pane values vs. pane-less values

Some hooks (`on-buffer-open`, `on-text-changed`, `on-buffer-close`, `on-diagnostics-changed`)
hand their callback a pane-less value that identifies a buffer but names no pane showing
it. A builtin that needs a real pane (to read the cursor, the viewport, or the live
selection set) raises on a pane-less value rather than degrading gracefully.

The standard resolution is `(buffer-panes pane)`: `car` of the result if some pane shows
the buffer, otherwise a documented fallback (line 0, or skip the operation) if it isn't
shown anywhere. A caller that already holds a genuine, live pane (a command's own leading
argument, a hook that does carry one) should use it directly — re-resolving risks picking
a *different* pane on the same buffer than the one the caller actually meant.

### Stale async work

A plugin that schedules a timer or spawns a subprocess has to handle the callback firing
after the state it was scheduled against has moved on:

- **The entry may be gone.** A buffer can close while a fetch or a walk is in flight.
  A write path serving a live read (a toggle command, an explicit user action) should
  *resurrect* a missing entry rather than no-op; a write path serving an async callback
  should no-op, since resurrecting state for a buffer that's genuinely gone would leak it
  forever.
- **`cancel-timer!`/canceling a job can't stop a callback that's already been dequeued
  onto the run queue.** For anything more than a single in-flight timer, an entry needs
  a generation counter, bumped every time the operation restarts; a callback closes over
  the generation it started under and no-ops if the entry's current generation has since
  moved on. `buffer-words`' `gen` field is the reference implementation.

### Debouncing

`debounce-by` (keyed) rather than plain `debounce` (global) for anything that fires per
buffer — a command's own pane and a hook's pane-less value for the same buffer must
collapse into the same pending timer, and one buffer's edits must never cancel a
different buffer's pending refresh. `git-diff`, `buffer-words`, and `lsp`'s inlay hints
all key by `buffer-key`.

### Capture the target, don't re-read focus

A callback that runs later — a debounced fire, an async process result, a menu selection
after a network round trip — should capture the pane (and, where relevant, the buffer's
edit generation) at the moment the request was made, not re-read `(focused-pane)` when
the callback finally runs. The user is free to switch buffers while something is in
flight; re-reading focus at completion time silently redirects the result to wherever the
user happens to be looking now instead of where it was requested. `lsp`'s code-action
menu and `git-diff`'s fetch/diff pipeline both follow this.

### Log severity

`log!` supports `'error`/`'warn`/`'info`/`'trace`. Route by how actionable the failure is
to the *user*, not by how verbose the plugin author wants to be:

- `'error` — the operation genuinely couldn't run (a required tool is missing, a process
  failed to start at all).
- `'warn` — a real failure that's a direct answer to something the user just typed (a
  bad ref, an invalid name), or a security-relevant refusal (a path-traversal attempt)
  worth a persistent record.
- `'trace` — an expected, common failure that would otherwise spam `:messages` for every
  buffer that never opted into the feature (e.g. "not a git repository" for branch
  tracking, which runs unconditionally for every buffer).
- `'info` (via `Severity::Info`) is status-line only and never reaches `:messages` — never
  use it for anything that should stay reviewable after the moment it flashed by.

## Steel pitfalls worth knowing before you hit them

- **Never re-raise a native-builtin error through a nested `with-handler`.** Catching an
  error from a native-backed call (a subprocess, a file operation) and re-raising it
  through an outer `with-handler` corrupts Steel 0.8.3's VM continuation stack. If you
  need to guarantee cleanup around a call that can raise, guard only the cleanup step, and
  let the original raise propagate uncaught.
- **JSON `null` decodes to Steel `void`, not `#f`.** Check `(void? res)` for "no results"
  from an LSP response; a bare `(not res)` check will not catch it.
- **Lists are linked, not arrays.** Walk with `car`/`cdr`, never `list-ref` in a loop —
  indexing a linked list by position is quadratic in the list's length.

## Doc conventions for this directory

A plugin's `.scm` files carry a banner comment and, where useful, a one-line pointer to
the README section that explains a given piece of code (`;;; see README.md's "X"`) —
never the explanation itself. All prose — design rationale, algorithms, non-obvious
gotchas — lives in the plugin's README (or its `docs/*.md`, for a plugin split into
several).

Each plugin's own doc follows the same shape:

1. **What it does and why**, in a couple of paragraphs.
2. **Usage** — a config snippet, then bullets for dependencies, activation, and a link to
   the user manual for anything end-user-facing.
3. **Commands** — a table.
4. **How it works** — concepts and algorithms, one `###` heading per topic.
5. **Design decisions** — short "X, not Y — because Z" bullets for choices a contributor
   might otherwise second-guess.
6. **Known limitations** — where any exist.
7. **Implementation notes** — terse, file-grouped bullets for function-level gotchas that
   don't fit the narrative above but are still worth knowing before touching that code.

A split plugin's README covers 1–3 plus a map of its `docs/*.md` files; each of those
files covers 4–7 for its own area.
