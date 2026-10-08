# core:lsp — Diagnostics and inlay hints

`diagnostics.scm` sends no request. It reads the diagnostics store through
`diagnostics-for-buffer`, which returns up to 1000 entries for a buffer, and turns them
into navigation, a drawer, an end-of-line (EOL) summary and gutter signs. `inlay.scm`
requests inlay hints. Both are covered here.

## Diagnostics navigation

`g n` and `g p` (`goto-next-diagnostic`, `goto-prev-diagnostic`) jump to the first
diagnostic starting strictly after the cursor, or the last starting strictly before it,
and wrap around when none qualifies. A cursor inside a diagnostic therefore moves past it.
The jump also pops the target's full message in a dismiss-on-any-key overlay.

`lsp/diag-jump-to!` passes the diagnostic's own pane to `goto-location!` as the target
only. The invocation pane is always `(focused-pane)`. The drawer stays open across a
buffer switch (see below), so a row picked there must jump into the buffer it was listed
for, not whichever buffer is focused, and the pane that opened the drawer may show a
different buffer by then. The `g n`/`g p` popup opens on `(focused-pane)` for the same reason:
the jump has already navigated there, so that is the pane showing the target.

## Diagnostics drawer

`:diagnostics` lists the buffer's diagnostics in a drawer, one row per diagnostic:
a severity glyph, `line:col`, the diagnostic's `source` (the server's own name when it sets none) and the message's first line. Selecting a row jumps the
same way navigation does, without the popup, since the row already showed the message.
Unlike a one-shot picker, the drawer's rows track the buffer's diagnostics as they
change, and this plugin owns refreshing them.

- **State.** The drawer-tracking state is either "no drawer open" or one triple: buffer
  key, open token, and the diagnostics currently shown. A single value makes "closed"
  structural instead of an invariant over three separate variables. The refresh path
  compares the buffer key, not the opening pane, because the `on-diagnostics-changed`
  hook's value carries no pane.
- **Token.** The token is passed to every later call (update, close, read selection).
  The drawer ignores any call whose token does not name the open drawer (closed,
  replaced, or never this plugin's), so a stale drawer is never touched. A drawer closed
  by `Esc` or replaced is noticed the next time the plugin visits it. An update that
  reports the drawer gone leaves tracking cleared.
- **Selection survives a refresh.** The new list is never empty here: an empty list closes
  the drawer first. The selected row moves to the diagnostic with the same message and
  severity nearest in line to the previous selection (the position is left out of the match because
  the fix's own edit can shift lines; on equal distance the first wins). With no match it
  keeps the previous index, clamped into the new list, which lands on the next diagnostic when
  the selected one was fixed.

## Refresh path

Three hooks keep every decoration current, and each fetches the diagnostics once and uses
the result for both the decorations and the drawer:

| Hook | Effect |
|---|---|
| `on-diagnostics-changed` | Refreshes the EOL summary, signs and drawer for that buffer |
| `on-option-change` for `lsp.diagnostics-severity-floor` or `lsp.diagnostics-on-insert-line` | Refreshes every buffer. The store applies a new floor only when it is read, so without this each buffer would keep the previous cut until its next diagnostics change |
| `on-lsp-detach` | Refreshes the summary, signs and drawer from what the buffer's other servers published; the detached server's diagnostics are already gone |

## Grouping and severity

The EOL summary and the signs share two helpers.

- **Severity.** Each diagnostic carries a `'severity-rank` field, authored once in Rust
  (`0` for error up to `3` for hint). `lsp/most-severe` folds over it keeping the minimum,
  so the comparison lives in one place and no sort is needed to find the worst.
- **Run-length grouping.** `lsp/group-by` groups adjacent items with equal keys. The EOL
  summary groups by line, relying on the store returning diagnostics start-ascending so
  same-line entries are contiguous. The signs first expand each diagnostic into one
  `(line . diagnostic)` pair per line it touches, accumulate all pairs, sort them by line,
  and group by line.

## End-of-line summary

One entry per line that has diagnostics, set through `set-eol-text!` under source
`"lsp-diagnostics"`. The text is the message's first line from the leftmost diagnostic on
the line, prefixed with `[n]` when the line has `n > 1` diagnostics. The color comes from
the most severe one. The two choices are independent: the most severe diagnostic is not
always the leftmost. `lsp/first-line` splits once at the first newline, which the popup
and drawer rows also use.

- **Scope.** `<severity>.diagnostic.inline` (for example `error.diagnostic.inline`), a
  HUME-specific scope. It is separate from `diagnostic.<severity>`,
  which themes use for the text squiggle (the bundled `sand` theme underlines it) and
  which virtual text past the end of the line must not inherit. Putting the severity name
  first lets a theme color all four from one `error`/`warning`/`info`/`hint` entry through
  the usual dot-notation fallback.
- **Insert line.** The plugin passes the negation of `lsp.diagnostics-on-insert-line` as
  `#:hide-on-insert-line`, and the render side drops flagged entries on the primary
  cursor's line while its pane is in Insert mode. The underline is owned by Rust and
  reads the option directly.

## Gutter signs

Signs are set with `set-signs!` under the same source, glyph `●`. Every refresh
registers the source for the buffer with `register-sign-source!` at priority `10`, as
does the detach handler before it clears. A diagnostic spanning several lines gets a sign
on each line from its start line through its end line, both inclusive, and the most severe
diagnostic on a line wins. The sign's scope is the bare severity name (`error`, `warning`,
`info`, `hint`) rather than the summary's `.diagnostic.inline` form, because the gutter
glyph is a separate render surface and must not inherit the text squiggle's underline.
This plugin is the only place a diagnostic becomes a gutter mark.

## Inlay hints

Hints are requested for the visible lines and are off by default. `:set global
lsp.inlay-hints=true` turns them on.

| Trigger | Pane carried | Handling |
|---|---|---|
| `on-viewport-change` | Live pane | Refreshed directly |
| `on-diagnostics-changed` | Buffer only | Resolved with `lsp/resolve-pane`, skipped when no pane shows the buffer |
| `on-lsp-detach` | Buffer only | Same, except that a buffer no pane shows has its hints cleared |
| `on-text-changed` | Buffer only | Same. It covers undo, redo and edits that neither scroll nor republish diagnostics, so a hint dropped with its anchor character returns when the edit is undone |

Refreshes are debounced by 200 ms per buffer (keyed, not global), so a diagnostics batch
touching two buffers cannot have the second buffer's call cancel the first's pending
refresh. A refresh whose request is sent while the same buffer's previous one is still
unanswered cancels it, so a slow server is never left with a queue of stale requests. A refresh asks every server of the buffer that offers inlay hints and merges their
hints into one set. It sends nothing when the option is off or the request params cannot be
built because the buffer is hidden by the time the debounce fires, and clears the buffer's
hints when no server offers them.

On the response:

- A hint whose wire position lands at or past the end of the text (the text changed
  between request and response) is dropped.
- The hints shown are those of every server that answered; a null answer contributes
  none, so the hints left from an earlier, larger answer go.
- A request no server could take clears the buffer's hints.
- When every server failed (an error, a timeout, a stop), the existing hints stay and each
  error is reported. A failure says nothing about what the buffer has, where a null answer
  says there are none.

Turning `lsp.inlay-hints` off clears this plugin's hint source for every buffer, and
turning it on refreshes every buffer. Detaching a server refreshes that buffer, so the
other servers' hints stay; a buffer no pane shows has its hints cleared instead, and they
return when it is shown again. The hint store is per source, so another plugin's hints are
unaffected.
