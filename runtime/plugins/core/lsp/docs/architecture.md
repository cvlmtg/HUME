# core:lsp — Architecture

## File layout

One `plugin.scm` entry `require`s a file per feature area, plus `lib.scm` (shared
helpers), `registration.scm` (catalog, receipts, the install scan), and `servers.scm`
(install/uninstall). Every feature file is the same three-line shape: send an
`lsp-request`, transform the response, call a UI or store builtin.

| File | Owns | Doc |
|---|---|---|
| `lib.scm` | Capability checks, error reporting, location-drawer helper | this file |
| `registration.scm` | Seeded catalog, receipt/path helpers, the registration scan | `servers.md` |
| `servers.scm` | Install/uninstall pipeline, install lock, discovery hint | `servers.md` |
| `diagnostics.scm` | Diagnostics navigation, EOL summary, gutter signs | `decorations.md` |
| `inlay.scm` | Inlay hints | `decorations.md` |
| `goto.scm` | Goto-definition family, references | `features.md` |
| `hover.scm` | Hover | `features.md` |
| `sighelp.scm` | Signature help | `features.md` |
| `completion.scm` | Completion | `features.md` |
| `actions.scm` | Code actions | `features.md` |
| `format.scm` | Formatting | `features.md` |
| `rename.scm` | Rename | `features.md` |

## Key layout

Goto-shaped requests — the four `lsp-goto-*` plus diagnostic nav — live under `g`,
alongside HUME's native line gotos and structural navigation, since each one names a
destination. Requests that answer with a panel rather than a jump (references list,
code-action menu) live under `z` instead, the view prefix, since what they do is open
something over the buffer — the same shape `core:pickers`' fuzzy finders use.

Two are neither. `lsp-rename` goes to `G R`: `G` is where the commands Vim files under
`g` that aren't gotos live (`G L`/`G U`/`G C` are Vim's `gu`/`gU`/`g~`), and nvim's own
rename default `grn` is no more a goto than those are. `lsp-hover` gets bare `K` —
hover is used often enough that a prefix is a tax, and `K` is Vim's own keyword-lookup
key, so it needs no learning.

No collisions with HUME's native leaves — `g`'s (`g e h l s`, plus the structural
`f F t T a A c C u U v V`), `G`'s (`L U C`), or `z`'s (`z k j`) — per
`keymap/defaults.rs`. `z f`/`z b`/`z m` under the same view prefix belong to
`core:pickers`. Every one of these is a two-key sequence or a fresh top-level key,
never a bare key over an existing prefix — a single-key bind is a plain map insert and
would drop the whole subtree under it (see `core:vim-keybind`'s README for the shape of
that hazard). `lsp-fmt` and `diagnostics` have no default key — typed-command only.

## Response conventions

Every feature file shares these:

- **JSON `null` decodes to Steel `void`, not `#f`** — every response handler in this
  plugin checks `(void? res)` for "no results", never `(not res)`.
- **`lsp/report-error`** takes either a `{"code" "message"}` hashmap or the bare
  string `"timeout"` and logs one `'error` line either way.
- **Capability guards** (`lsp/supports?`, `lsp/guard-capability`) read
  `(lsp-capabilities pane)`, a `JsonHandle` onto `pane`'s buffer's attached server's
  provider capabilities, via `json-ref`/`json-contains?`. `lsp/caps-has-cap?`
  treats a capability as present only when the handle exists, contains the key, and
  that key isn't explicitly `#f` — a provider capability can be declared and then
  disabled with `#f`, which is different from never being declared. `lsp/cap-field`/
  `lsp/cap-flag?` read a nested field off a capability that can be the bare `#t` or an
  options object (e.g. `codeActionProvider.resolveProvider`,
  `completionProvider.triggerCharacters`, `documentRangeFormattingProvider.rangesSupport`),
  returning a caller-supplied default on every kind of miss alike.

## Shared helpers (`lib.scm`)

- **Trigger-char lifecycle** — `lsp/setup-trigger-chars!` wires `on-lsp-attach`/
  `on-lsp-detach` for a feature (completion, signature help), registering the
  attached chars through whichever table the feature actually needs: signature
  help (which passes a handler) goes through `register-trigger-chars!` and gets
  `on-trigger-char` wired too; completion (`on-trigger` `#f`) goes through
  `completion-set-trigger-chars!` instead — the editor invokes that source
  directly against its own trigger chars, no hook round trip. Both tables are
  keyed `(source, language)`, so a second language attaching under the same
  `source-name` gets its own entry rather than clobbering the first. At attach,
  the capability's own `"triggerCharacters"` array — read through `lsp/cap-field`
  like any other nested capability field — is unpacked with `json-list` if
  present, empty otherwise.
- **Pane resolution** — `lsp/resolve-pane` answers `(car (buffer-panes pane))`: the
  focused pane if it shows `pane`'s buffer, else the first pane on the active tab,
  else any other; `#f` if the buffer isn't shown anywhere. For a hook whose own value
  carries no pane (`on-diagnostics-changed`, `on-text-changed`, a `(buffers)`
  element) to make an explicit choice before calling a pane-needing builtin, in place
  of the implicit guess those builtins used to make internally. A caller already
  holding a real pane (`on-viewport-change`, `on-trigger-char`, a command's own
  leading `pane`) has no reason to call this — re-resolving risks silently picking a
  *different* pane on the same buffer.
- **Viewport** — `lsp/visible-lines` wraps the synchronous `viewport-range` builtin,
  which is 0-based end-exclusive, so the visible-line count is just the range's width
  (no `+ 1`). `viewport-range` raises (kind-B fail-fast) rather than answering `#f` for
  a pane that doesn't show its buffer, so every caller either already holds a real,
  live pane (hover and signature help both pass their own request's invocation pane,
  confirmed still focused by `#:require-focus`) or resolves one explicitly first via
  `lsp/resolve-pane` (inlay hints, for the hooks whose own pane value carries none).
  `lsp/visible-lines`'s two callers are hover's popup-docking threshold and inlay
  hints' refresh trigger.
- **Location display** — a raw `Location`/`LocationLink` hashmap's `{uri, range}`-vs-
  `{targetUri, targetRange}` shape dispatch lives in one place,
  `hume_lsp::location::decode_location` (Rust), shared by `goto-location!` (the jump)
  and `lsp-locations->display-parts` (the drawer row) — nothing in this plugin reads a
  location's wire fields directly. Every "L:C" position HUME shows a user goes through
  the one `lsp/format-position` formatter, so `:diagnostics`'s drawer rows and the
  goto/references drawer agree on what a position reads as. `lsp/location-display`
  additionally runs `path->display` (`~` collapse, UNC strip) — the only formatting
  still done here — and falls back to bare `path:line` when there's no column. The one
  exception: a goto/references target with no open buffer renders the location's own
  raw wire `character` rather than reading the file to convert it to a grapheme column
  — see CLAUDE.md's "Displayed value" sanctioned exception for the full rationale.
  `lsp/show-locations!` opens the drawer on `(focused-pane)`, not the request's own
  invocation pane: `lsp/goto-response`'s callers deliberately skip `#:require-focus` so
  a slow goto/references response still completes even if the user looked elsewhere
  meanwhile, and the drawer should open wherever the user actually is once the
  response lands rather than raise when the original pane is no longer focused or has
  since closed.
