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
per spec, never a bare `Location`. Every jump in the cascade lands via
`(goto-location! (focused-pane) …)`, not a captured invocation pane — the response's
own `JsonHandle` (and every location `json-list` pulls out of it) already carries the
producing server's own tagged encoding, so the decode needs no pane capture, and the
jump itself should go wherever the user actually is once the response lands (see
`lsp/show-locations!`'s own note above on why the drawer works the same way). None of
the four plain goto commands pass `#:require-focus` to their request, unlike hover,
signature help, and code actions: a goto is a navigation the user asked for, not info
anchored to where the cursor happens to be right now, so completing the jump once it
resolves is correct even if the user looked elsewhere while waiting — the same way
pressing Enter on a slow-loading link still navigates.

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
a different `#:anchor`. Hover's popup passes `#:kind 'scrollable` — a response can run long, and unlike
signature help it has no natural end-of-session to tie its lifetime to. Any key,
paste, or mouse input other than a scrolling Ctrl-u/d closes it and still does its
own job — no dismiss code needed here.

`lsp-hover`'s request passes `#:require-focus #t`, so the bridge drops the
response if the focused buffer has moved on by the time it lands — the
request is async, so the user is free to switch buffers while it's in
flight, and a popup for a symbol they're no longer looking at would be worse
than no popup. Signature help and code actions pass the same flag for the
same reason.

## Signature help

The popup uses `#:kind 'sticky`, the default: it lives in the editor's current-mode
slot and closes on its own once Insert ends, with no dismiss code needed in this
plugin.

A parameter label is either a plain string or a `[start, end)` offset pair into the
signature's own label — the offset form is what a server sends because HUME declares
`labelOffsetSupport`, and those offsets count code units in the server's negotiated
encoding, so the host (not this file) does the slicing. The offset pair (when present)
is itself a child of the signature-help response, so it carries that response's own
tagged producing-server encoding straight into `lsp-label-offsets->text`, with no pane
needed for the decode. `lsp/show-sighelp` opens the popup on the request's own
invocation pane — `#:require-focus #t` on the request guarantees it's still focused
by the time the response lands, so it's the right pane to anchor the popup at.
`lsp/sighelp-request`'s debounced body checks `(pane-live? pane)` before sending:
`pane` itself, not just its buffer, may have closed or switched buffers during the
debounce window, and `lsp-position-params` resolves the pane (not just the buffer)
and would raise on either — a benign "this pane is no longer what it was when the
keystroke armed this timer" isn't worth a logged error. There's no styling API in
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

The plugin is a *source*, not the driver: `register-completion-source!` registers
`"lsp"` (a `#:target 'buffer` source — every buffer source's token is the
identifier before the cursor, so the editor seeds the filter from it and accept
replaces it — with `#:resolve #t`, its claim that its items are wire items from
the buffer's own attached server, licensing `completionItem/resolve` on accept),
and the editor calls it — on `Ctrl-Space`, on a
server trigger character (registered as `"lsp"`'s own trigger chars via
`completion-set-trigger-chars!` at attach — `lib.scm`'s `lsp/setup-trigger-chars!`,
not the shared `register-trigger-chars!`/`on-trigger-char` table that signature
help uses instead — so the editor invokes the source directly, no hook round
trip), and again after each
keystroke while the last answer said `isIncomplete`. The source declines with an empty
answer when the buffer's server has no `completionProvider`. Never passes
`#:allow-stale` to `lsp-request` — unlike hover, a stale completion response is
auto-cancelled/dropped rather than shown; a re-request can go out before a prior
response lands, so it's sent with `#:supersede "completion"`, and an answer to a call
the editor has since superseded is dropped by the editor anyway. This source never
reads a field of its own response, so it hands the opaque `JsonHandle` every
`lsp-request` response crosses as straight to `completion-emit!` unopened;
`completion-emit!` reads the handle's `isIncomplete`/`items` itself, in Rust. Snippet stripping
happens in Rust at the store ingress, so items arriving here already have plain
`insertText`/`textEdit.newText`. There's deliberately no `on-completion-accept`
handler: Rust applies the main edit, `additionalTextEdits`, and
`completionItem/resolve` atomically on accept, leaving nothing for Scheme to do.

## Code actions

`context.diagnostics` must echo back the *raw* wire `Diagnostic` objects in range —
rust-analyzer (confirmed) gates diagnostic-derived quickfixes on this, withholding
them for an empty array; `diagnostics-for-buffer`'s `"raw"` field carries these
through unmodified for exactly this reason. A `CodeAction` is filtered out of the menu
if it carries a truthy `"disabled"` field (LSP 3.16); v1 doesn't otherwise pre-filter
by `kind`. `lsp/primary-selection-range` returns `(start . end)` (end exclusive), or
`#f` when there's no primary selection, which `diagnostics-for-buffer`'s `#:range`
filter reads as "no range filter" (not "empty range"). Applying an action runs its
`edit` first, then its `command`, per spec order; an action with neither is
lazily-resolved via `codeAction/resolve` first, bounded to a single round trip so a
non-conforming server that re-resolves to a still-empty edit/command can't loop. The
bare legacy `Command` shape (a plain top-level `command` string, no `edit` key) is
handled by passing the whole action object through as the `Command` — its shape
already matches what the executor expects. An action, and a command's `"arguments"`
array, cross straight back out to `codeAction/resolve`/`workspace/executeCommand` as
the `JsonHandle` they arrived as — `steel_to_json`'s handle arm resolves a nested
handle to its own value once the enclosing hash goes out over the wire, so nothing
here has to read a field just to re-send it unmodified.

`pane` (the buffer the action came from) and `gen` (that buffer's generation at the
same capture point) are both captured when `"lsp-code-actions"` sends its request,
then threaded through the menu selection and, for an unresolved action, the
`codeAction/resolve` round trip — never re-read from focus, since both round trips
are async (the user picks a menu item, then waits on the network), the same
capture-at-source discipline every other chained LSP request here uses. `gen` is
checked by `apply-workspace-edit!`'s own `#:expect-generation` before applying, so an
edit computed against text that has since changed fails loudly instead of applying
against the wrong text. `lsp/exec-command`'s `workspace/executeCommand` request passes
`#:allow-stale #t`: its params carry no `textDocument`, so the bridge's own text-gen
anchor has nothing buffer-specific to check `pane` against — without this, the
anchor's fallback (its own current generation vs. `pane`'s at drain time) would drop
the response on any intervening edit. Safe to skip, since this callback only reports
an error; the command's actual edits (if any) arrive separately via a
server-initiated `workspace/applyEdit`, which carries its own positions and is never
subject to this staleness check. The `codeAction/resolve` request passes
`#:allow-stale #t` too, for a different reason: unlike the menu-building request, this
response *is* the edit — dropping it on an intervening keystroke would silently do
nothing after the user already picked an action from the menu. Safe to deliver stale
here as well, since `apply-workspace-edit!`'s own `#:expect-generation` check still
fails loudly if the buffer actually changed.

## Formatting

Format-on-save is not wired by default — v1 is manual `:lsp-fmt` only. To opt in,
uncomment `format.scm`'s commented-out hook:

```scheme
(register-hook! 'on-buffer-save
  (lambda (pane)
    ;; `on-buffer-save`'s own `pane` carries no pane of its own — `lsp-fmt`
    ;; needs one (it reads the live selection set), so this resolves one
    ;; explicitly first (`(car (buffer-panes pane))`, via `lsp/resolve-pane`)
    ;; rather than passing the pane-less value straight through.
    (let ((resolved (lsp/resolve-pane pane)))
      (when resolved (call! "lsp-fmt" resolved)))))
```

`:lsp-fmt` classifies the selection set with `(selections-linewise? pane)` and
`(selections-charwise? pane)`, and reads `(lsp-linewise-ranges-params pane)`'s
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
