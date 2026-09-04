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
- **Capability guards** (`lsp/supports?`, `lsp/supports-for-buffer?`,
  `lsp/guard-capability`) read `(lsp-capabilities server)`, a hash of provider
  capabilities. `lsp/caps-has-cap?` treats a capability as present only when the hash
  exists, contains the key, and that key isn't explicitly `#f` — a provider
  capability can be declared and then disabled with `#f`, which is different from
  never being declared. `lsp/cap-field`/`lsp/cap-flag?` read a nested field off a
  capability that can be the bare `#t` or an options hash (e.g.
  `codeActionProvider.resolveProvider`, `completionProvider.triggerCharacters`,
  `documentRangeFormattingProvider.rangesSupport`), returning a caller-supplied
  default on every kind of miss alike.

## Shared helpers (`lib.scm`)

- **Popup dismissal** — one `on-mode-change` registration closes whatever popup is
  open, shared by every feature that uses one (hover, signature help) rather than each
  registering its own.
- **Trigger-char lifecycle** — `lsp/setup-trigger-chars!` wires `on-lsp-attach`/
  `on-lsp-detach`/`on-trigger-char` for a feature (completion, signature help). It's
  keyed `(source, language)` on the Rust side, so a second language attaching under
  the same `source-name` gets its own entry rather than clobbering the first.
- **Viewport** — `lsp/visible-lines` wraps the synchronous `viewport-range` builtin,
  which is 0-based end-exclusive, so the visible-line count is just the range's width
  (no `+ 1`). Its two callers are hover's popup-docking threshold and inlay hints'
  refresh trigger.
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
