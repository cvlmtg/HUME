# core:lsp — Diagnostics and inlay hints

## Diagnostics navigation

`gn`/`gp` (`goto-next-diagnostic`/`goto-prev-diagnostic`) jump to the first diagnostic
strictly after, or last strictly before, the cursor, wrapping around when none qualifies.
A cursor sitting inside a diagnostic still advances past it, never jumps back to it.
They also pop the target's full message in a dismiss-on-any-key overlay; `:diagnostics`'s
drawer selection jumps the same way but skips the popup, since the drawer row already
showed the message. The end-of-line inline summary shows one `"[n] <message>"` per
offending line: the text comes from the leftmost diagnostic on that line, the color from
the most severe one: independent choices, since the most severe diagnostic isn't always
the leftmost. A change to `lsp.diagnostics-severity-floor` needs an explicit refresh of
every buffer's inline summary: the diagnostics store only applies the new floor the next
time it's read, so without this hook every buffer would keep showing the old cut until
its next unrelated diagnostics-changed fire.

The summary's scope is `<severity>.diagnostic.inline` (`error.diagnostic.inline` and
friends), a HUME scope with no Helix counterpart, since Helix has no end-of-line
diagnostic summary to theme. It is deliberately not `diagnostic.<severity>`, which
belongs to the editing-area text span: every bundled theme gives that one an `underlined`
modifier for the squiggle, and virtual text sitting past the end of the line must not
inherit it. The leading-severity spelling puts each severity's own name first so a theme
can colour all four from one `error`/`warning`/`info`/`hint` entry through the usual
dot-notation fallback, which is exactly what a theme that declares none of them gets.

`lsp/first-line`, which both the popup and the EOL summary use for a message's first
line, splits once rather than splitting the whole string: a multi-line rustc message is
routinely 5-20 lines, and every row is rebuilt on every publish, so splitting the whole
message just to keep the first line would allocate and discard the rest for nothing.

`lsp/diag-jump-to!` takes its diagnostic's own pane value explicitly, as
`goto-location!`'s *target* only, never as its invocation pane, which is always
`(focused-pane)` instead. The diagnostics drawer stays open across a buffer switch by
design (see [Diagnostics drawer](#diagnostics-drawer) below), so a row selected there
must jump into the buffer it was listed for, not whichever buffer happens to be focused
when Enter is pressed, and the pane that opened the drawer may no longer show that buffer
at all by the time a row is picked. The `gn`/`gp` popup opens on `(focused-pane)` for the
same reason: the jump has already navigated there by the time the popup opens, so it's
the pane actually showing the target diagnostic, not necessarily the pane `gn`/`gp` was
invoked with.

The diagnostics store sorts and deep-clones up to 1000 diagnostics' whole raw LSP JSON,
so every hook below fetches it once and threads the result through to both the
decorations refresh and the drawer refresh, rather than each calling it separately.

## Diagnostics drawer

`:diagnostics` opens a drawer this plugin then owns refreshing itself: unlike a one-shot
picker, this drawer's rows must track the buffer's diagnostics live as they change.
The drawer-tracking state holds either "no drawer open" or one triple (buffer key, open
token, and the diagnostics it's currently showing) as one value, not three hand-synced
globals, so "closed" is structural rather than an invariant that would have to hold
across three separately-cleared fields. The buffer key, not the pane the drawer was
opened with, is what the refresh path compares by, since the diagnostics-changed hook's
own value carries no pane of its own. The open token is threaded into every later call
(update, close, read-selection); the drawer widget ignores any of them the moment the
token no longer names the open drawer (closed, replaced, or never this plugin's), so a
stale or foreign drawer can never be touched by mistake, even lazily: every path that
could invalidate the token (an `Esc` close, a replacing drawer) is caught the next time
this plugin visits it, since every mutator is already token-guarded. A stale-open return
from the drawer widget (a stale async open) leaves tracking untouched, since there is no
drawer to track.

On refresh, the surviving-selection logic picks which row stays selected out of the new
diagnostics list (guaranteed non-empty, since its sole caller guards on an empty list first):
the surviving diagnostic (nearest match by message + severity, position deliberately kept
out of the key since the fix's own edit can shift other diagnostics' lines; ties break
toward the nearest line), or the old index clamped into the new list's bounds as a
fallback: the item now at that position, i.e. the next one when the selected diagnostic
itself was the one fixed. Both the index picker and the drawer update already clamp into
the new list, so the selection index passes through raw with no separate clamp.

## Gutter signs

Gutter signs are the same pull, one call further: the diagnostics-decorations refresh
places them through `set-signs!` under source `"lsp-diagnostics"` (registered per buffer,
priority `10`, the first time this function or the detach handler runs for that buffer;
see `register-sign-source!`), glyph `"●"`, alongside the EOL summary it already built.
This plugin is the only place a diagnostic becomes a gutter mark. A diagnostic spanning
several lines gets one sign per line it touches (start through end line, both inclusive;
the diagnostics store clamps the end line into the buffer's addressable range the same way
it does the start); the most severe diagnostic on a line wins, via the same reduction the
EOL summary uses. The sign's scope is the bare severity name (`error`/`warning`/`info`/
`hint`) rather than the inline-summary's `.diagnostic.inline` form: the gutter glyph and
its underlying text span are different render surfaces, and every bundled theme
underlines the `diagnostic.<severity>` scope for the text squiggle, an underline the
gutter glyph must not inherit.

Severity ranking folds over each diagnostic's own `'severity-rank` field (authored once
in Rust: `0` for error, counting up to `3` for hint) rather than re-encoding that order
in Scheme, so there is exactly one place either decoration's severity comparison happens.
It's a running-best fold, not a sort-then-take-first: this runs once per line group on
every diagnostics-changed fire, and a sort is wasted work when only the minimum is ever
read back out.

The run-length grouping both diagnostic decorations share is one algorithm: the EOL
summary's line grouping (diagnostics already start-ascending, so same-line entries are
contiguous) and the sign path's line-touch grouping (which needs its own explicit sort
first, since one diagnostic can land in more than one line's group). The sign path folds
every diagnostic's line-touch pairs onto one accumulator (no per-diagnostic sublist
spread through `append`) before the single sort and group that turns them into per-line
groups.

The detach hook clears both the summary and the signs; the severity-floor option-change
handler refreshes both together, since they pull from the same diagnostics-store call.

## Inlay hints

Off by default (`:set global lsp.inlay-hints=true` opts in).

| Trigger | Pane carried? | Handling |
|---|---|---|
| `on-viewport-change` | Real, live pane | Refreshed directly |
| `on-diagnostics-changed` | Buffer id only | Resolved via `lsp/resolve-pane`, skipped if nothing shows the buffer |
| `on-text-changed` | Buffer id only | Same as above. Covers undo/redo and any edit that neither scrolls the viewport nor provokes a diagnostics republish, so a hint dropped because its anchor character was deleted comes back once that edit is undone |

Debounced 200ms per buffer (keyed, not global) so a diagnostics batch touching two
buffers can't have the second buffer's call cancel the first's pending refresh.

Building the request's own params fails as `#f` the same way a hint's position does
(the buffer can't be resolved: hidden or detached by the time a debounced refresh actually
fires), and the refresh simply skips sending anything that round. A hint whose wire
position can't be converted to a buffer offset (the buffer detached between the request
firing and the response arriving) is silently dropped rather than raising. A legitimate
empty/null response still clears any hints left from a prior, larger response; only a
genuine request error leaves the existing display untouched.

The render bridge itself is deliberately *not* gated on the `lsp.inlay-hints` option:
the hint store is per-source, so an unrelated plugin's hints must not vanish just because
this one setting toggles. This plugin instead owns clearing its own source when the
setting turns off, and re-requesting hints for every visible buffer when it turns back
on; the option-change handler reads the option back through the normal option-read path
(already coerced to a bool) rather than trusting the hook's own raw string value.
