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

The summary's scope is `lsp/severity-scope`'s `<severity>.diagnostic.inline`
(`error.diagnostic.inline` and friends) — a HUME scope with no Helix counterpart,
since Helix has no end-of-line diagnostic summary to theme. It is deliberately not
`diagnostic.<severity>`, which belongs to the editing-area text span: every bundled
theme gives that one an `underlined` modifier for the squiggle, and virtual text
sitting past the end of the line must not inherit it. The leading-severity spelling
puts each severity's own name first so a theme can colour all four from one
`error`/`warning`/`info`/`hint` entry through the usual dot-notation fallback, which
is exactly what a theme that declares none of them gets.

`lsp/first-line`, which both the popup and the EOL summary use for a message's first
line, calls `split-once` rather than `split-many` — a multi-line rustc message is
routinely 5-20 lines, and every row is rebuilt on every publish, so splitting the
whole message just to keep the first line would allocate and discard the rest for
nothing. `split-once` answers `#t` (not `#f`) when the pattern isn't found, so `pair?`
— not truthiness — is what tells "found a split" from "no newline in this message"
apart; a bare `(if parts (car parts) text)` would call `(car #t)` on every
single-line message, which is most of them.

`lsp/diag-jump-to!` takes its diagnostic's own pane value explicitly, as
`goto-location!`'s *target* only — never as its invocation pane, which is always
`(focused-pane)` instead — the diagnostics drawer stays open across a buffer switch
by design (browse-while-editing, below), so a row selected there must jump into the
buffer it was listed for, not whichever buffer happens to be focused when Enter is
pressed, and the pane that opened the drawer may no longer show that buffer at all
by the time a row is picked.

`diagnostics-for-buffer` sorts and deep-clones up to 1000 diagnostics' whole raw LSP
JSON, so every hook below fetches it once and threads the result through to both the
decorations refresh and the drawer refresh, rather than each calling it separately.

## Diagnostics drawer

`:diagnostics` opens a drawer the plugin then owns refreshing itself: unlike a
one-shot picker, this drawer's rows must track the buffer's diagnostics live as
`on-diagnostics-changed` keeps firing. `lsp/*diag-drawer*` holds `#f` when no
drawer is open, or `((buffer-key pane) tok diags)` — one value, not three
hand-synced globals, so "closed" is structural rather than an invariant that would
have to hold across three separately-cleared fields. The first element is a
`buffer-key`, not the pane value `:diagnostics` was invoked with — `on-diagnostics-
changed`'s own `pane` carries no pane of its own, so the refresh path compares by
buffer identity, not by the drawer-opening command's (possibly no-longer-focused)
pane. `tok` is `show-drawer-list!`'s own return, threaded into
every later call (`update-drawer-list!`, `close-drawer!`, `drawer-selected-index`);
Rust ignores any of them the moment `tok` no longer names the open drawer (closed,
replaced, or never this plugin's), so a stale or foreign drawer can never be touched
by mistake, even lazily — every path that could invalidate `tok` (an `Esc` close, a
replacing drawer) is caught the next time this plugin visits it, since every mutator
is already token-guarded. `show-drawer-list!`'s own `#f` return (a stale async open)
leaves tracking untouched — there is no drawer to track.

On refresh, `lsp/diag-refresh-index` picks which row stays selected out of NEW-DIAGS
(guaranteed non-empty — its sole caller guards on `(null? diags)` first): the
surviving diagnostic (`lsp/diag-best-match`, below), or the old index clamped into
the new list's bounds as a fallback — the item now at that position, i.e. the next
one when the selected diagnostic itself was the one fixed. `lsp/diag-best-match`
finds OLD's nearest surviving match in NEW-DIAGS by message + severity — position
stays out of the key on purpose, since the fix's own edit can shift other
diagnostics' lines; ties (the same message twice) break toward the nearest line, and
`#f` means nothing matched. `lsp/diag-refresh-index` and `update-drawer-list!` both
clamp into the new list already, so the selection index passes through raw with no
separate Scheme-side clamp.

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
rather than `lsp/severity-scope`'s form: the gutter glyph and its underlying text
span are different render surfaces, and every bundled theme underlines the
`diagnostic.<severity>` scope for the text squiggle — an underline the gutter glyph
must not inherit.

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
call cancel the first's pending refresh. Building the request's own params fails as
`#f` the same way a hint's position does — the buffer can't be resolved (hidden or
detached by the time a debounced refresh actually fires) — and the refresh simply
skips sending anything that round. A hint whose wire position can't be converted
to a buffer offset — the buffer detached between the request firing and the response
arriving — is silently dropped rather than raising. A legitimate empty/null response
still clears any hints left from a prior, larger response; only a genuine request
error leaves the existing display untouched.

The render bridge itself is deliberately *not* gated on the `lsp.inlay-hints` option —
the hint store is per-source, so an unrelated plugin's hints must not vanish just
because this one setting toggles. This plugin instead owns clearing its own source
when the setting turns off, and re-requesting hints for every visible buffer when it
turns back on; the `on-option-change` handler reads the option back via `get-option`
(already coerced to a bool) rather than trusting the hook's own raw `:set`/
`set-option!` string value.
