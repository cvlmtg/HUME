# core:git-diff — Rendering (`render.scm`)

Every function in `render.scm` is a pure `hunks → decoration records` view over the one
hunk shape `state.scm` stores (the additivity invariant described in
`docs/architecture.md`'s "State" section), ending in exactly one setter call —
`render-inline!` is the one documented exception, below. All setters share a
feature-scoped source name, `"git-diff"`, not the `core:git-diff` plugin id — matching
`core:lsp`'s own decoration sources (`"lsp-diagnostics"`, `"lsp-inlay-hints"`).

## Signs

Signs render at VSCode/gitsigns density (one per changed line, not one per hunk — a
20-line paste shows 20 `+` marks). Its gutter slot comes from registering `"git-diff"` as
a sign source *per buffer* (`git-diff/render-signs!`, idempotently, right before every
`set-signs!` call it makes) rather than from anything in the call itself. A buffer whose
signs never render (untracked, or `signs?` never turned on) never reserves this slot.
`git-diff/*sign-priority*` is `0` — its rank against every other source registered for
the same buffer, `core:lsp`'s `"lsp-diagnostics"` source (priority `10`) included,
decides this plugin's gutter slot there (see `docs/LSP.md`'s `register-sign-source!`
entry). `0` puts git-diff last in a buffer's registry, so its column is the first to
fall off signcolumn's auto-size cap when several higher-priority channels share the
buffer.

Deciding a hunk's sign kind: a pure deletion has no new-side lines to anchor on, so the
sign lands on the line above the gap instead (gitsigns' convention) — `(- new-start 1)`
rather than `new-start` sidesteps an out-of-range `set-signs!` call for a deletion at
end of file, and `(max 0 …)` covers a deletion at line 0. A pure addition (`old-count`
zero) gets a `+` sign on every new-side line; anything else gets `~` on every new-side
line.

`git-diff/render-signs!`'s `(apply append …)`, not `flatten`, joins the per-hunk sign
lists — a sign entry is itself a list, and `flatten` would tear each one apart. An empty
`hunks` clears the gutter (`set-signs!` replaces `source`'s signs wholesale), so this
doubles as the clear function.

## Inline: deleted lines + word highlights

Where a hunk's removed old-side lines attach, as a `(kind . line)` anchor pair:
`'after (- new-start 1)` when a preceding line exists — renders at the same position
`'before new-start` would, but stays valid when `new-start` is the buffer's content line
count (a deletion at end of file, where `'before new-start` would address the phantom
trailing line and raise). `'before 0` only for a deletion at the very start.

The virtual-line hashmap `set-virtual-lines!` expects uses symbol keys, not strings —
`set-virtual-lines!` looks each field up as `(SteelVal::SymbolV k)`; a string key raises
"hashmap key must be a symbol". `'segments` is omitted when empty rather than set to
`'()`, keeping the hash to what's used. A whole removed line with no word-level detail
passes `old-line` straight through as `'text` — `set-virtual-lines!` accepts a literal
tab and expands it itself.

A removed line's word-del `'segments` come from `diff-words`' `(old-start old-end
new-start new-end old-text new-text)` hunks, filtered to `old-start < old-end` — a pure
insertion has nothing to underline on the old-side line, and a zero-width segment would
raise (`set-virtual-lines!`'s `start < end` check). The new-side counterpart —
`(start end scope)` triples in *buffer* char offsets, since `set-extra-highlights!`
addresses the whole buffer, not one line — is filtered to `new-start < new-end` for the
same reason.

Char offsets for a hunk's paired new-side lines are computed without one `line->offset`
host call per line: only the hunk's first new-side line needs it (`line->offset`); every
later line is exactly its predecessor's length plus one `\n` further along.

One paired `(old-line . new-line)` becomes a `(virtual-line . spans)` pair from a single
`diff-words` call shared by both sides — the sanctioned two-setter exception described
below starts here. Within a hunk, old-lines `[0, paired-count)` have a same-index
new-line counterpart to word-diff against; any remainder gets a plain whole-line row. The
three-list walk (old-lines, new-lines, offsets) advances via `cdr`, not `list-ref` by
index — Steel lists are linked, so indexing would make this quadratic in `paired-count`.

A pure addition (`old-count` zero) contributes nothing to the inline pass — nothing was
removed to show as a virtual row; `render-line-bgs!` alone covers its new-side tint.

`render-inline!` makes two setter calls (`set-virtual-lines!`/`set-extra-highlights!`)
instead of this file's usual one: a single `diff-words` pass inherently produces two
decoration kinds — old-side virtual rows and new-side highlight spans — and splitting it
into two renderers to keep one setter each would call `diff-words` twice for no benefit.

## Row background tint

One hunk becomes `(line scope)` entries, one per new-side line: pure add → `diff.plus`,
change → `diff.delta`. A pure delete contributes nothing — `render-inline!`'s virtual
rows already cover the removed content. No priority field on this setter (unlike
`set-signs!`) — this plugin is the only tint producer for its own scopes.

## Flag → renderer dispatch

`render-for!` is the one place a flag key (`"signs?"`/`"inline?"`) maps to its
renderer(s), so every caller that needs to paint or clear a rendering (`diff.scm`'s
`apply-hunks!` on a live refresh, `plugin.scm`'s toggle command on enable/disable) goes
through it rather than re-stating the mapping.
