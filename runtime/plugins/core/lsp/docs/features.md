# core:lsp — Request-driven features

## Request flags at a glance

Every `lsp-request!` call in this plugin picks its flags for a reason. Reading them side
by side is more useful than reading each feature's own paragraph in isolation:

| Request | `#:require-focus` | `#:allow-stale` | `#:supersede` | Why |
|---|---|---|---|---|
| `lsp-hover` | ✓ | ✓ | | A popup for a symbol the user isn't looking at anymore would be worse than none; but a slow hover response is still worth showing if focus hasn't moved |
| `lsp-goto-*` (4) | | | | A navigation the user asked for completes even if they looked elsewhere while waiting, like pressing Enter on a slow-loading link |
| `lsp-references` | ✓ | | | Same "complete the navigation" reasoning as goto, but the drawer it may open should reflect where the user actually is |
| Signature help | ✓ | | | Anchors the popup at the request's own invocation pane, confirmed still focused |
| Code actions (menu request) | ✓ | | | Builds a menu from context (selection, diagnostics) captured at invocation. Stale context would build the wrong menu |
| `codeAction/resolve`, `workspace/executeCommand` | | ✓ | | The user already picked an action from the menu; dropping the follow-up would silently do nothing after that choice. `apply-workspace-edit!`'s own generation check still fails loudly if the buffer actually changed |
| Completion | | | ✓ `"completion"` | A stale completion response should be replaced by a fresher one, not shown. The editor drops a response to a call it has since superseded anyway |
| Formatting (all three shapes) | | ✓ | | Same as the code-action follow-ups: the generation check guards correctness, so staleness alone isn't worth dropping a formatting result over |
| Rename | | ✓ | | Same reasoning as formatting |

## Goto and references

All four goto-family commands and `lsp-references` share one response-handling cascade:

1. An error is reported.
2. A null/empty response says "no results".
3. A single `Location` jumps directly.
4. A `Location[]`/`LocationLink[]` array jumps directly if it has exactly one entry,
   otherwise lists them in the drawer.

`lsp-references` forces the drawer even for a single result ("where is this used"
expects a list, unlike goto's "take me there") and reuses the same cascade rather than
reimplementing it, so its single-`Location` branch is simply unreached:
`textDocument/references` only ever returns `Location[] | null` per spec, never a bare
`Location`.

Every jump in the cascade lands on `(focused-pane)`, not a captured invocation pane: the
response's own tagged encoding already carries what it needs to decode correctly, so the
jump itself should go wherever the user actually is once the response lands.

## Hover

A `MarkedString` (bare string or `{language, value}`) or `MarkupContent` (`{kind, value}`)
response is decoded to raw text. A `{language, value}` `MarkedString` arrives with its
code fence already stripped, so it's re-added before rendering: the popup's markdown
injection needs the fence to highlight it, rather than falling back to plain text. Only
an explicit `MarkupContent` with `kind: "plaintext"` opts out of markdown highlighting; a
bare `MarkedString` is always markdown per the LSP spec.

The popup docks at the bottom instead of floating near the cursor once its line count
exceeds ⅓ of the last-known viewport height. Either way it's still the same popup, just
with a different anchor. Any key, paste, or mouse input other than a scrolling Ctrl-u/d
closes it and still does its own job, so no dismiss code needed here.

## Signature help

The popup lives in the editor's current-mode slot and closes on its own once Insert ends,
with no dismiss code needed in this plugin.

A parameter label is either a plain string or a `[start, end)` offset pair into the
signature's own label. The offset form is what a server sends because HUME declares
offset support, and those offsets count code units in the server's negotiated encoding,
so the host, not this file, does the slicing. There's no styling API for the popup, so
the active parameter's text is marked with `⟨…⟩` on a second line instead of highlighted
in place.

`")"` is registered as a trigger character but treated as a dismiss, not a request. It
still has to be registered or it would never reach Insert-mode text at all. The request
callback is guarded against a stale trigger character left registered past detach (or a
server that never advertised signature help), so a matching keystroke on such a buffer
skips politely instead of hitting a server-resolution failure. The debounced request body
also checks that the pane is still live before sending: the pane itself, not just its
buffer, may have closed or switched buffers during the debounce window, and building the
request would raise on either, and a benign "this pane is no longer what it was when the
keystroke armed this timer" isn't worth a logged error.

Signature/parameter indices from the server are clamped into range rather than trusted
verbatim. An empty signature list is spec-valid ("nothing to show"), handled the same as
a null/void response.

## Completion

The plugin is a *source*, not the driver: it registers `"lsp"` (a buffer-target source:
every buffer source's token is the identifier before the cursor, so the editor seeds the
filter from it and accept replaces it) with `#:resolve #t`, its claim that its items are
wire items from the buffer's own attached server, licensing `completionItem/resolve` on
accept, and `#:priority 10`), and the editor calls it: on `Ctrl-Space`, on a server
trigger character (registered as this source's own trigger chars at attach, so the
editor invokes the source directly with no hook round trip), and again after each
keystroke while the last answer said incomplete. The source declines with an empty answer
when the buffer's server has no completion provider.

This source never reads a field of its own response. It hands the response straight to
the store, which reads the incomplete flag and items itself. Snippet stripping happens at
the store's own ingress, so items arriving here already have plain insert text. There's
deliberately no accept handler in Scheme: the host applies the main edit,
`additionalTextEdits`, and `completionItem/resolve` atomically on accept, leaving nothing
for Scheme to do.

## Code actions

`context.diagnostics` must echo back the *raw* wire `Diagnostic` objects in range:
rust-analyzer (confirmed) gates diagnostic-derived quickfixes on this, withholding them
for an empty array; the diagnostics store's raw field carries these through unmodified
for exactly this reason.

A `CodeAction` is filtered out of the menu if it carries a truthy `"disabled"` field (LSP
3.16); v1 doesn't otherwise pre-filter by kind. Applying an action runs its `edit` first,
then its `command`, per spec order; an action with neither is lazily resolved via
`codeAction/resolve` first, bounded to a single round trip so a non-conforming server that
re-resolves to a still-empty edit/command can't loop. The bare legacy `Command` shape (a
plain top-level `command` string, no `edit` key) is handled by passing the whole action
object through as the `Command`, since its shape already matches what the executor expects.

The buffer the action came from, and that buffer's edit generation at the same capture
point, are both captured when the menu request is sent, then threaded through the menu
selection and, for an unresolved action, the `codeAction/resolve` round trip, never
re-read from focus, since both round trips are async (the user picks a menu item, then
waits on the network). The generation is checked before applying, so an edit computed
against text that has since changed fails loudly instead of applying against the wrong
text.

## Formatting

Format-on-save is not wired by default: v1 is manual `:lsp-fmt` only. To opt in,
uncomment `format.scm`'s commented-out hook:

```scheme
(register-hook! 'on-buffer-save
  (lambda (pane)
    ;; on-buffer-save's own pane carries no pane of its own — lsp-fmt
    ;; needs one (it reads the live selection set), so this resolves one
    ;; explicitly first rather than passing the pane-less value straight through.
    (let ((resolved (lsp/resolve-pane pane)))
      (when resolved (call! "lsp-fmt" resolved)))))
```

### Selection classification

| Selection set | Result |
|---|---|
| All selections linewise | Formats those ranges (touching selections coalesced, disjoint ones kept separate, since an LSP range is one contiguous span, so a gap can't be expressed as a single range) |
| None linewise (all charwise) | Formats the whole buffer |
| A mix of the two | Warns and formats nothing rather than guessing which reading was meant |

A collapsed cursor that happens to land on a blank line is ambiguous either way, so it's
excluded from all three classifications. It never masks a real selection elsewhere in
the set into "mixed", and never bridges two real linewise selections it happens to touch
on both sides into one coalesced range.

Disjoint ranges go out as one `rangesFormatting` request (LSP 3.18) when the server
advertises range support, otherwise one `rangeFormatting` request per range, capped at
`lsp.format-max-ranges`. Past the cap, `:lsp-fmt` warns and formats nothing, the same
refusal a mixed selection set gets, rather than silently narrowing to one selection. A
buffer with no path, or no attached server, is distinguished directly, since a capability
guard can't tell the two apart: without a server there's no capabilities to check in the
first place.

The multi-range fan-out tracks three boxes: requests still in flight, edits accumulated
so far, and whether the fan-out has already aborted. The abort flag only suppresses
duplicate error log lines when two or more ranges fail. The no-partial-format guarantee
comes from the in-flight count never reaching zero after an error, not from this flag.
Responses can land in any order (once aborted, the fan-out is already dead; otherwise the
fold order doesn't matter); applying edits sorts them by position first, and coalescing
guarantees no two ranges can tie.

## Rename

No tree-sitter fallback in v1: a buffer with no attached server just reports "not
supported" via the ordinary capability guard, the same as any other unsupported feature.
The rename prompt pre-fills with the symbol under the cursor.
