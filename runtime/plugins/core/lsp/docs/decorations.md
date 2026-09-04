# core:lsp — Diagnostics and inlay hints

## Diagnostics navigation

`gn`/`gp` (`goto-next-diagnostic`/`goto-prev-diagnostic`) jump to the first diagnostic
strictly after, or last strictly before, the cursor, wrapping around when none
qualifies — a cursor sitting inside a diagnostic still advances past it, never jumps
back to it. They also pop the target's full message in a dismiss-on-any-key overlay;
`:diagnostics`'s drawer selection jumps the same way but skips the popup, since the
drawer row already showed the message. The end-of-line inline summary shows one
`"[n] <message>"` per offending line: the text comes from the leftmost diagnostic on
that line, the color from the most severe one — independent choices, since the most
severe diagnostic isn't always the leftmost. A change to
`lsp.diagnostics-severity-floor` needs an explicit refresh of every buffer's inline
summary: `diagnostics-for-buffer` only applies the new floor the next time it's
called, so without this hook every buffer would keep showing the old cut until its
next unrelated `on-diagnostics-changed` fire.

## Gutter signs

Gutter signs are the same pull, one call further: `lsp/refresh-diagnostic-decorations`
places them through `set-signs!` under source `"lsp-diagnostics"` (registered per
buffer, priority `10`, the first time this function or the `on-lsp-detach` handler
runs for that buffer — see `register-sign-source!`), glyph `"●"`, alongside the EOL
summary it already built; this plugin is the only place a diagnostic becomes a gutter
mark. A diagnostic spanning several lines gets one sign per line it touches (`"line"`
through `"end-line"`, both inclusive — `diagnostics-for-buffer` clamps `"end-line"`
into the buffer's addressable range the same way it does `"line"`); the most severe
diagnostic on a line wins, via the same `lsp/most-severe` reduction the EOL summary
uses. The sign's scope is the bare severity name (`error`/`warning`/`info`/`hint`)
rather than `lsp/severity-scope`'s `diagnostic.*`-prefixed form: the gutter glyph and
its underlying text span are different render surfaces, and every bundled theme
underlines the `diagnostic.*` scope for the text squiggle — an underline the gutter
glyph must not inherit.

`lsp/most-severe` ranks by each diagnostic's own `"severity-rank"` field
(`DiagSeverity`'s `Ord`, authored once in Rust — 0 for error, counting up to 3 for
hint) rather than re-encoding that order here, so there is exactly one place either
decoration's severity comparison happens. It's a running-best fold, not a
sort-then-take-`car`: this runs once per line group on every `on-diagnostics-changed`
fire, and a sort is wasted work when only the minimum is ever read back out.

`lsp/group-by` is the one run-length grouping algorithm both diagnostic decorations
share: the EOL summary's `lsp/group-by-line` (diagnostics already start-ascending, so
same-line entries are contiguous) and the sign path's line-touch grouping
(`lsp/diagnostic-signs`), which needs its own explicit sort first since one diagnostic
can land in more than one line's group there. `lsp/diagnostic-signs` folds every
diagnostic's line-touch pairs onto one accumulator — no per-diagnostic sublist spread
through `apply` — before the single sort + `lsp/group-by` that turns them into
per-line groups.

`on-lsp-detach` clears both the summary and the signs; the severity-floor
`on-option-change` handler refreshes both together, since they pull from the same
`diagnostics-for-buffer` call.

## Inlay hints

Off by default (`:set global lsp.inlay-hints=true` opts in). Refreshed on
`on-viewport-change`, `on-diagnostics-changed`, and `on-text-changed` — the last
covers undo/redo and any other edit that neither scrolls the viewport nor provokes a
diagnostics republish, so a hint dropped because its anchor character was deleted
comes back once that edit is undone. Debounced 200ms per buffer via `debounce-by` (not
`debounce`) so a diagnostics batch touching two buffers can't have the second buffer's
call cancel the first's pending refresh. A hint whose wire position can't be converted
to a buffer offset — the buffer detached between the request firing and the response
arriving — is silently dropped rather than raising. A legitimate empty/null response
still clears any hints left from a prior, larger response; only a genuine request
error leaves the existing display untouched.

The render bridge itself is deliberately *not* gated on the `lsp.inlay-hints` option —
the hint store is per-source, so an unrelated plugin's hints must not vanish just
because this one setting toggles. This plugin instead owns clearing its own source
when the setting turns off, and re-requesting hints for every visible buffer when it
turns back on.
