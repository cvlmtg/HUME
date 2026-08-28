# core:lsp

Language server features: hover, go-to-definition (+ declaration / type-definition /
implementation), references, diagnostics navigation, rename, formatting, code actions,
signature help, completions, inlay hints. Also owns the LSP server lifecycle end to
end — install, uninstall, registration, and runtime management (`servers.scm`,
`registration.scm`) — see `docs/LSP-INSTALL.md` in the repository. `core:plum` (the plugin
manager) is not involved.

## Usage

```scheme
(declare-plugin "core:stdlib")

(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))

(declare-plugin "core:lsp")
```

Requires `core:stdlib` declared or loaded first — `core:lsp` scans installed servers via
`stdlib/list-subdirs` at its own load time, and `call!`'s lazy-miss retry inline-activates a
merely declared `core:stdlib` before that scan runs; diagnostics navigation and
`:lsp-install` call `stdlib/cursor-char-index`/`stdlib/resolve-lang-arg` via `call!` too, at
runtime (see ["Depending on another
plugin"](https://cvlmtg.github.io/HUME/plugins.html#depending-on-another-plugin)).

The bare `declare-plugin` above resolves `manifest.scm`, which declares the plugin with
`#:languages '("*")` (any buffer with a detected language) plus every `lsp-*` command — so it
activates on the first buffer with a language, or the first `lsp-*` command typed, whichever
comes first. An explicit `#:commands`/`#:events`/`#:languages` bypasses the manifest — a
manifest keyed only on `#:events '(on-lsp-attach)` can never activate on its own, since
nothing is registered yet for that event to fire on; `#:languages`, or the four `lsp-*`
install commands (`lsp-install`, `lsp-uninstall`, `lsp-servers`, `lsp-rescan-servers`) in
`#:commands`, give it a real trigger instead. A `register-lsp-server!` override placed
before or after the `declare-plugin` line always wins over the catalog default, since the
post-load scan reads through any registration queued earlier in the same eval and skips a
language that override already claims.

See [Language Servers](https://cvlmtg.github.io/HUME/lsp.html) for the full walkthrough,
commands, keys, and settings, and
[Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-lsp) for the quick
summary.

## Commands

| Command                | Effect                                                                       |
|-------------------------|-------------------------------------------------------------------------------|
| `:lsp-install [lang]`  | Download, verify, unpack, and register the server for a language (default: current buffer's language) |
| `:lsp-uninstall <name>`| Shut down and unregister a server's clients, remove it from disk (by server name, not language) |
| `:lsp-servers`         | Catalog listing: every seeded server, its languages, and install status      |
| `:lsp-rescan-servers`  | Re-scan `<data>/servers/` and register any installed server not yet registered — useful for a server installed out-of-band |
| `:lsp-status`          | Show every running server and its state, plus attached buffers' diagnostic counts |
| `:lsp-stop [lang]`     | Stop a running server (default: focused buffer's)                            |
| `:lsp-restart [lang]`  | Stop and respawn a running server (default: focused buffer's)                |

## How it works

### File layout

One `plugin.scm` entry `require`s a file per feature area (`hover.scm`, `goto.scm`,
`diagnostics.scm`, `rename.scm`, `format.scm`, `actions.scm`, `sighelp.scm`,
`completion.scm`, `inlay.scm`), plus a shared `lib.scm` (capability checks, error
reporting, location-drawer helper), `registration.scm` (the seeded catalog, receipt/path
helpers, and the scan), and `servers.scm` (install/uninstall — see below). Every feature
file is the same three-line shape: send an `lsp-request`, transform the response, call a UI
or store builtin.

### Key layout

Jump-shaped actions (goto/rename/diagnostic-nav) live under `g`; response/action-shaped ones
(references list, hover popup, code-action menu) live under `z` instead, alongside view
commands — freeing `g R`/`g k`/`g a` for the fuzzy-finder picker prefix (`core:pickers`).
`z k` (hover) keeps the `k` mnemonic from `g k`, the same key Vim/Helix use for hover. No
collisions with HUME's native leaves — `g`'s (`g g e h l s u U C`) or `z`'s (`z z t b`), per
`keymap/defaults.rs`. `lsp-fmt` and `diagnostics` have no default key — typed-command only.

### Server install and registration

`servers.scm` downloads, verifies, and unpacks a server (`:lsp-install`), writes a
receipt as the install commit point, and calls `registration.scm`'s scan
(`lsp/register-installed-servers!`) directly afterward so the server attaches
immediately — no cross-plugin notify, since install and registration are the same
plugin. That scan independently reads the seeded `runtime/scheme/lsp-servers.scm`
catalog and `<data>/servers/` for receipts, registering (`register-lsp-server!`) every
installed server it finds; `plugin.scm` runs it once at its own top level, so it also
happens at load or lazy activation. It's the *only* registrar for managed servers.
`:lsp-rescan-servers` exposes the same scan for a server installed outside `:lsp-install`.

Installing (or reinstalling) a single server always starts from a clean slate — this also
serves as the repair/upgrade path, and covers reinstalling over a running client the same
way: (1) blocker check + tool preflight; (2) unregister every seeded language, reaping any
running client; (3) purge any existing install (the receipt dies with it); (4) download,
verify, and unpack (github), or run `npm install`/`cargo install`; (5) write the receipt —
the commit point; (6) a `$PATH` notice, if the seeded command also happens to resolve there
independently of the managed install.

Registering a server's languages skips any language already registered — this is what lets
a mid-session rescan leave a user's own manual `register-lsp-server!` alone instead of
last-wins-clobbering it. `lsp-registered-for-language?` reads through the same-eval pending
op queue, so this filter is always correct in queue order regardless of load order: the
post-install rescan sees `lsp/install-server!`'s own queued `unregister-lsp-server!` calls
the same way, correctly re-admitting those languages instead of treating them as already
taken. `apply_pending_lsp_server_reg`
(`hume-editor/src/editor/lsp/registry.rs`) sweeps every already-open buffer on the Rust side
for every registration this queues.

`:lsp-uninstall` takes a user-typed server name straight into a path join, so it validates
the name (non-empty, not `.`/`..`, no path separators) before touching disk; `lsp-install`
never needs this validation since its name always comes from the seeded
language-to-server index, never a raw argument.

### Install lock

`lsp/with-install-lock!` (`servers.scm`) runs a thunk under a cross-process lock
(`<data>/servers/.install-lock`), releasing it exactly once regardless of outcome — used by
both install and uninstall, so two HUME processes (or two `:lsp-install` calls) never race
the same server directory. It never re-raises the thunk's error through an outer
`with-handler`: re-raising a native-builtin error through a nested handler corrupts the
Steel VM's continuation stack, so every failure path here terminates in a plain `log!`
instead. Returns `#t` on success, `#f` on any failure — a lock the caller couldn't acquire
and a thunk that raised both collapse to the same `#f`, indistinguishable to the caller.

`lsp/lsp-install-or-report!` — the post-lock half of `:lsp-install` — runs the registration
rescan *outside* the lock, after `with-install-lock!` has already released it, so a failure
there surfaces as a distinct, uncaught error instead of being mislabeled "install failed" or
double-releasing the lock.

### Catalog and sources

Two separate hashes, kept intentionally apart: `registration.scm`'s `*lsp-servers*` (from
`runtime/scheme/lsp-servers.scm` — languages, command, args, config per server) is what a
server actually *does* once registered; `servers.scm`'s `*lsp-sources*` (from
`runtime/scheme/lsp-sources.scm` — kind, version, download targets) is how to *get* it. A
third hash, `*lsp-lang->server*`, is derived from the servers catalog at load time for O(1)
language lookup. Languages are disjoint across servers by a sync-time guarantee —
`scripts/sync-grammars.py` takes only each language's primary language server — so building
that index never silently last-wins two servers against each other.

### Discovery hint

`on-language-set` nudges once per language per session: if a buffer's language has a seeded
server that's installable but not yet installed, it suggests `:lsp-install`. The
dedup marker is set regardless of outcome, so a disqualified language (no seeded server, or
blocked on this platform) is never re-evaluated either. Logged `'warn`, not `'info` —
`Severity::Info` is display-only and never reaches `:messages`, so a nudge missed at the
moment it fires must stay reviewable afterward.

### Shared helpers (lib.scm)

- **Popup dismissal** — one `on-mode-change` registration closes whatever popup is open,
  shared by every feature that uses one (hover, signature help) rather than each registering
  its own.
- **Trigger-char lifecycle** — `lsp/setup-trigger-chars!` wires `on-lsp-attach`/
  `on-lsp-detach`/`on-trigger-char` for a feature (completion, signature help). It's keyed
  `(source, language)` on the Rust side, so a second language attaching under the same
  `source-name` gets its own entry rather than clobbering the first.
- **Viewport** — `lsp/visible-lines` wraps the synchronous `viewport-range` builtin; its two
  callers are hover's popup-docking threshold and inlay hints' refresh trigger.
- **Location display** — a raw `Location`/`LocationLink` hashmap's `{uri, range}`-vs-
  `{targetUri, targetRange}` shape dispatch lives in one place, `hume_lsp::location::decode_location`
  (Rust), shared by `goto-location!` (the jump) and `lsp-locations->display-parts` (the
  drawer row) — nothing in this plugin reads a location's wire fields directly. Every
  "L:C" position HUME shows a user goes through the one `lsp/format-position` formatter, so
  `:diagnostics`'s drawer rows and the goto/references drawer agree on what a position reads
  as. The one exception: a goto/references target with no open buffer renders the location's
  own raw wire `character` rather than reading the file to convert it to a grapheme column —
  see CLAUDE.md's "Displayed value" sanctioned exception for the full rationale.

### Diagnostics

`gn`/`gp` (`goto-next-diagnostic`/`goto-prev-diagnostic`) jump to the first diagnostic
strictly after, or last strictly before, the cursor, wrapping around when none qualifies — a
cursor sitting inside a diagnostic still advances past it, never jumps back to it. They also
pop the target's full message in a dismiss-on-any-key overlay; `:diagnostics`'s drawer
selection jumps the same way but skips the popup, since the drawer row already showed the
message. The end-of-line inline summary shows one `"[n] <message>"` per offending line: the
text comes from the leftmost diagnostic on that line, the color from the most severe one —
independent choices, since the most severe diagnostic isn't always the leftmost. A change to
`lsp.diagnostics-severity-floor` needs an explicit refresh of every buffer's inline summary:
`diagnostics-for-buffer` only applies the new floor the next time it's called, so without this
hook every buffer would keep showing the old cut until its next unrelated
`on-diagnostics-changed` fire.

Gutter signs are the same pull, one call further: `lsp/refresh-diagnostic-decorations` places
them through `set-signs!` under source `"lsp-diagnostics"` (registered per buffer, priority
`10`, the first time this function or the `on-lsp-detach` handler below runs for that buffer —
see `register-sign-source!`), glyph `"●"`, alongside the EOL summary it already built; this
plugin is the only place a diagnostic becomes a gutter mark. A diagnostic
spanning several lines gets one sign per line it touches (`"line"` through `"end-line"`,
both inclusive — `diagnostics-for-buffer` clamps `"end-line"` into the buffer's addressable
range the same way it does `"line"`); the most severe diagnostic on a line wins, via the same
`lsp/most-severe` reduction the EOL summary uses. The sign's scope is the bare severity name
(`error`/`warning`/`info`/`hint`) rather than `lsp/severity-scope`'s `diagnostic.*`-prefixed
form: the gutter glyph and its underlying text span are different render surfaces, and every
bundled theme underlines the `diagnostic.*` scope for the text squiggle — an underline the
gutter glyph must not inherit. `lsp/most-severe` itself ranks by each diagnostic's own
`"severity-rank"` field (`DiagSeverity`'s `Ord`, authored once in Rust — 0 for error, counting
up to 3 for hint) rather than re-encoding that order here, so there is exactly one place either
decoration's severity comparison happens. `on-lsp-detach` clears both the summary and the
signs; the severity-floor `on-option-change` handler refreshes both together, since they pull
from the same `diagnostics-for-buffer` call.

### Inlay hints

Off by default (`:set global lsp.inlay-hints=true` opts in). Refreshed on both
`on-viewport-change` and `on-diagnostics-changed`, debounced 200ms per buffer via
`debounce-by` (not `debounce`) so a diagnostics batch touching two buffers can't have the
second buffer's call cancel the first's pending refresh. A hint whose wire position can't be
converted to a buffer offset — the buffer detached between the request firing and the
response arriving — is silently dropped rather than raising. A legitimate empty/null response
still clears any hints left from a prior, larger response; only a genuine request error leaves
the existing display untouched. The render bridge itself is deliberately *not* gated on the
`lsp.inlay-hints` option — the hint store is per-source, so an unrelated plugin's hints must
not vanish just because this one setting toggles. This plugin instead owns clearing its own
source when the setting turns off, and re-requesting hints for every visible buffer when it
turns back on.

### Code actions

`context.diagnostics` must echo back the *raw* wire `Diagnostic` objects in range —
rust-analyzer (confirmed) gates diagnostic-derived quickfixes on this, withholding them for
an empty array; `diagnostics-for-buffer`'s `"raw"` field carries these through unmodified for
exactly this reason. A `CodeAction` is filtered out of the menu if it carries a truthy
`"disabled"` field (LSP 3.16); v1 doesn't otherwise pre-filter by `kind`. Applying an action
runs its `edit` first, then its `command`, per spec order; an action with neither is
lazily-resolved via `codeAction/resolve` first, bounded to a single round trip so a
non-conforming server that re-resolves to a still-empty edit/command can't loop. The bare
legacy `Command` shape (a plain top-level `command` string, no `edit` key) is handled by
passing the whole action object through as the `Command` — its shape already matches what the
executor expects.

### Completion

Never passes `#:allow-stale` to `lsp-request` — unlike hover, a stale completion response is
auto-cancelled/dropped rather than shown. Snippet stripping happens in Rust at the store
ingress, so items arriving here already have plain `insertText`/`textEdit.newText`. Two entry
points reach the same request function: `Ctrl+Space` (bound to `lsp-completion-trigger`) and a
registered server trigger character. Per-keystroke refiltering can re-issue the request before
a prior response lands, so it's sent with `#:supersede "completion"` rather than racing two
sessions; the `on-completion-refilter` hook needs no capability re-guard, since the capability
was already confirmed to start the session in the first place. There's deliberately no
`on-completion-accept` handler: Rust applies the main edit, `additionalTextEdits`, and
`completionItem/resolve` atomically on accept, leaving nothing for Scheme to do.

### Hover

A `MarkedString` (bare string or `{language, value}`) or `MarkupContent` (`{kind, value}`)
response is decoded to raw text. A `{language, value}` `MarkedString` arrives with its code
fence already stripped, so it's re-added — `#:lang`'s markdown injection needs the fence to
highlight it, rather than falling back to plain text. Only an explicit `MarkupContent` with
`kind: "plaintext"` opts out of markdown highlighting — a bare `MarkedString` is always
markdown per the LSP spec. The popup docks at the bottom instead of floating near the cursor
once its line count exceeds ⅓ of the last-known viewport height (falling back to a flat 15
lines before the first `on-viewport-change` event) — either way it's still `show-popup!`, just
with a different `#:anchor`. Dismissal (any key, mouse input, or mode change, except
Ctrl+u/d scrolling) is shared with signature help via `lib.scm`'s registration, not
duplicated here.

### Signature help

A parameter label is either a plain string or a `[start, end)` offset pair into the
signature's own label — the offset form is what a server sends because HUME declares
`labelOffsetSupport`, and those offsets count code units in the server's negotiated encoding,
so the host (not this file) does the slicing. There's no styling API in `show-popup!` v1, so
the active parameter's text is marked with `⟨…⟩` on a second line instead of highlighted
in place. `")"` is registered as a trigger character but treated as a dismiss, not a request —
it still has to be registered or it would never reach Insert-mode text at all. The request
callback is guarded against a stale trigger character left registered past detach (or a
server that never advertised `signatureHelpProvider`), so a matching keystroke on such a
buffer skips politely instead of hitting `lsp-request`'s server-resolution failure.

### Goto and references

All four goto-family commands and `lsp-references` share one response-handling cascade:
an error is reported; a null/empty response says "no results"; a single `Location` hashmap
jumps directly; a `Location[]`/`LocationLink[]` array jumps directly if it has exactly one
entry, otherwise lists them in the drawer. `lsp-references` passes `#:always-drawer? #t` to
force the drawer even for a single result — "where is this used" expects a list, unlike
goto's "take me there" — and reuses the same cascade rather than reimplementing it, so its
bare-`Location` branch is simply unreached: `textDocument/references` only ever returns
`Location[] | null` per spec, never a bare `Location`.

### Formatting and rename

Format-on-save is not wired by default — v1 is manual `:lsp-fmt` only; `format.scm` carries a
commented-out `on-buffer-save` hook to opt in. `:lsp-fmt` picks range vs. whole-buffer
formatting (and the matching capability to guard on) based on whether the current selection
spans one or more complete lines. Rename has no tree-sitter fallback in v1 — a buffer with no
attached server just reports "not supported" via the ordinary capability guard, the same as
any other unsupported feature.
