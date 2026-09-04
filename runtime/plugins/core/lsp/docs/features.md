# core:lsp — Request-driven features

## Goto and references

All four goto-family commands and `lsp-references` share one response-handling
cascade (`lsp/goto-response`): an error is reported; a null/empty response says "no
results"; a single `Location` hashmap jumps directly; a `Location[]`/`LocationLink[]`
array jumps directly if it has exactly one entry, otherwise lists them in the drawer.
`lsp-references` passes `#:always-drawer? #t` to force the drawer even for a single
result — "where is this used" expects a list, unlike goto's "take me there" — and
reuses the same cascade rather than reimplementing it, so its bare-`Location` branch
is simply unreached: `textDocument/references` only ever returns `Location[] | null`
per spec, never a bare `Location`.

## Hover

A `MarkedString` (bare string or `{language, value}`) or `MarkupContent`
(`{kind, value}`) response is decoded to raw text. A `{language, value}`
`MarkedString` arrives with its code fence already stripped, so it's re-added —
`#:lang`'s markdown injection needs the fence to highlight it, rather than falling
back to plain text. Only an explicit `MarkupContent` with `kind: "plaintext"` opts out
of markdown highlighting — a bare `MarkedString` is always markdown per the LSP spec.
The popup docks at the bottom instead of floating near the cursor once its line count
exceeds ⅓ of the last-known viewport height (falling back to a flat 15 lines before
the first `on-viewport-change` event) — either way it's still `show-popup!`, just with
a different `#:anchor`. Dismissal (any key, mouse input, or mode change, except
Ctrl+u/d scrolling) is shared with signature help via `lib.scm`'s registration, not
duplicated here.

## Signature help

A parameter label is either a plain string or a `[start, end)` offset pair into the
signature's own label — the offset form is what a server sends because HUME declares
`labelOffsetSupport`, and those offsets count code units in the server's negotiated
encoding, so the host (not this file) does the slicing. There's no styling API in
`show-popup!` v1, so the active parameter's text is marked with `⟨…⟩` on a second line
instead of highlighted in place. `")"` is registered as a trigger character but
treated as a dismiss, not a request — it still has to be registered or it would never
reach Insert-mode text at all. The request callback is guarded against a stale
trigger character left registered past detach (or a server that never advertised
`signatureHelpProvider`), so a matching keystroke on such a buffer skips politely
instead of hitting `lsp-request`'s server-resolution failure.

`lsp/clamp-index` clamps a server-sent signature/parameter index into
`[0, (length lst) - 1]` rather than trusting it verbatim. An empty `signatures: []` is
spec-valid ("nothing to show"), handled the same as a null/void response.

## Completion

Never passes `#:allow-stale` to `lsp-request` — unlike hover, a stale completion
response is auto-cancelled/dropped rather than shown. Snippet stripping happens in
Rust at the store ingress, so items arriving here already have plain
`insertText`/`textEdit.newText`. Two entry points reach the same request function:
`Ctrl+Space` (bound to `lsp-completion-trigger`) and a registered server trigger
character. Per-keystroke refiltering can re-issue the request before a prior response
lands, so it's sent with `#:supersede "completion"` rather than racing two sessions;
the `on-completion-refilter` hook needs no capability re-guard, since the capability
was already confirmed to start the session in the first place. There's deliberately no
`on-completion-accept` handler: Rust applies the main edit, `additionalTextEdits`, and
`completionItem/resolve` atomically on accept, leaving nothing for Scheme to do.

## Code actions

`context.diagnostics` must echo back the *raw* wire `Diagnostic` objects in range —
rust-analyzer (confirmed) gates diagnostic-derived quickfixes on this, withholding
them for an empty array; `diagnostics-for-buffer`'s `"raw"` field carries these
through unmodified for exactly this reason. A `CodeAction` is filtered out of the menu
if it carries a truthy `"disabled"` field (LSP 3.16); v1 doesn't otherwise pre-filter
by `kind`. `lsp/primary-selection-range` returns `#f` when there's no primary
selection, which `diagnostics-for-buffer`'s `#:range` filter reads as "no range
filter" (not "empty range"). Applying an action runs its `edit` first, then its
`command`, per spec order; an action with neither is lazily-resolved via
`codeAction/resolve` first, bounded to a single round trip so a non-conforming server
that re-resolves to a still-empty edit/command can't loop. The bare legacy `Command`
shape (a plain top-level `command` string, no `edit` key) is handled by passing the
whole action object through as the `Command` — its shape already matches what the
executor expects.

## Formatting

Format-on-save is not wired by default — v1 is manual `:lsp-fmt` only. To opt in,
uncomment `format.scm`'s commented-out hook:

```scheme
(register-hook! 'on-buffer-save (lambda (bid) (call! "lsp-fmt")))
```

`:lsp-fmt` classifies the selection set with `(selections-linewise? bid)` and
`(selections-charwise? bid)`, and reads `(lsp-linewise-ranges-params bid)`'s
`"ranges"` as payload only: all selections linewise formats those ranges (touching
selections coalesced, disjoint ones kept separate — an LSP range is one contiguous
span, so a gap can't be expressed as a single range), none linewise formats the whole
buffer, and a mix of the two warns and formats nothing rather than guessing which
reading was meant. A collapsed cursor that happens to land on a blank line is
ambiguous either way, so it's excluded from all three of `selections-linewise?`,
`selections-charwise?`, and `lsp-linewise-ranges-params`'s ranges — it never masks a
real selection elsewhere in the set into "mixed", and never bridges two real linewise
selections it happens to touch on both sides into one coalesced range. Disjoint ranges
go out as one `textDocument/rangesFormatting` request (LSP 3.18) when the server
advertises `rangesSupport`, otherwise one `rangeFormatting` request per range, capped
at `lsp.format-max-ranges` — past the cap, `:lsp-fmt` warns and formats nothing, the
same refusal a mixed selection set gets, rather than silently narrowing to one
selection. A buffer with no path, or no attached server, is distinguished by
`lsp-server-for-buffer` (a capability guard can't tell the two apart — without a
server there's no `lsp-capabilities` to check in the first place).

`lsp/format-fan-out!`'s join tracks three boxes: `pending` (requests still in
flight), `edits` (accumulated so far), `aborted`. `aborted` only suppresses duplicate
error log lines when two or more ranges fail — the no-partial-format guarantee comes
from `pending` never reaching zero after an error, not from this flag. Responses can
land in any order (`aborted` set means the fan-out is already dead; otherwise the
fold order doesn't matter) — `apply-text-edits!` sorts edits by position before
applying, and coalescing guarantees no two ranges can tie.

## Rename

No tree-sitter fallback in v1 — a buffer with no attached server just reports "not
supported" via the ordinary capability guard, the same as any other unsupported
feature.
