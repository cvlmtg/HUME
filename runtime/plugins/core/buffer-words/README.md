# core:buffer-words

Offers every identifier already in the buffer as an Insert-mode completion. It works in
any buffer, including a scratch buffer or a `.txt` file where `core:lsp` has no server to
ask.

It defines no commands and binds no keys. It registers a `'buffer` completion source named
`"buffer-words"`, which answers `Ctrl-Space` (`completion-trigger`) and any trigger
characters a config gives it with `(set-completion-triggers! language "buffer-words"
chars)`. It sets no trigger characters itself.

## Usage

```scheme
(load-plugin! "core:stdlib")
(load-plugin! "core:buffer-words" #:config (hash 'match 'string 'chars 8000))
```

- **Depends on:** `core:stdlib`, at load time for config validation and at runtime for
  word splitting.
- **Activates on:** `load-plugin!`. It registers hooks and a completion source at load,
  so it has no `manifest.scm` (see the [core plugins index](../README.md#loading-model)).
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-buffer-words).

## Configuration

| Key | Type | Default | Effect |
|---|---|---|---|
| `'match` | `'string` \| `'fuzzy` | `'string` | `'string`: a case-sensitive prefix gate, the vim `i_CTRL-N` feel. `'fuzzy`: subsequence-scored like `core:lsp`'s own candidates, so the two compete on score rather than priority |
| `'chars` | integer (≥ 1) | `8000` | Characters fetched and scanned per side (before/after the cursor) on each walk tick, and the most an edit's changed lines may hold for the index to be updated without a walk |

## How it works

Each buffer has a cached word set. A background walk that yields to the editor between
ticks builds it, and `Ctrl-Space` reads whatever the walk has indexed so far. After an
edit the cache is updated from the lines the edit changed, and a walk runs again only when
that can't be done. The per-buffer entry follows the index's
[Per-buffer state](../README.md#per-buffer-state) shape.

```
cursor line
     │
     ▼
  ◀── N chars ── │ ── N chars ──▶                 one tick
  ◀──── N chars ──── │ ──── N chars ────▶        next tick
        …outward in both directions until both hit a buffer edge…
```

`N` is `'chars`, rounded to whole lines.

### Cursor-outward, line-windowed indexing

A walk starts on `on-buffer-open` and on every restart (see
[Updating after an edit](#updating-after-an-edit)). It goes outward from the cursor's line
in both directions, one window of lines per direction per tick, fetched with
`(buffer-lines pane #:range …)` and scanned within the same tick. A word cannot span two
lines, so nothing carries across a tick boundary. Nothing blocks however large the buffer
is, and the words nearest the cursor are indexed first. Each tick converts only the lines
it scans, so memory during a walk is bounded by `'chars`, not by buffer size. Each tick
reschedules the next after `bw/tick-delay-ms`.

#### Character budget

Each side sizes its window before fetching anything, from the offsets where lines start
(`line->char-offset`, `char-offset->line`) and the buffer's end (`buffer-char-count`). It
takes the whole lines that fit in the budget, and at least one line.

#### Anchor line

The anchor is the cursor's line, read through `(buffer-panes pane)` (see the index's
[Pane values vs. pane-less values](../README.md#pane-values-vs-pane-less-values)), since
`on-buffer-open` and `on-text-changed` hand over a pane-less value. A buffer no pane
shows has no cursor to anchor to and walks from the top. The forward side scans the anchor
line from its start, and the backward side starts at the line before it.

### Updating after an edit

The entry's `"seen"` is the `buffer-generation` its sets and walk positions match. Every
walk tick and every update advance it. On an edit, `buffer-changes-since` from `"seen"`
gives everything since as hunks, which two places apply with the same update
(`bw/updated-set`):

- **A walk's next tick.** A tick that finds the buffer changed updates the set being built
  and moves both walk positions through the hunks (`bw/shift-line`). Lines are read by
  number, so without the move a deletion above the forward side would carry unread lines
  past it, and an insertion above the backward side would carry unread lines below it. A
  position inside a hunk moves to that hunk's end (forward side) or start (backward side),
  since the update already added the hunk's new lines.
- **A finished index.** `on-text-changed` schedules `bw/refresh!`, debounced 150ms with
  `debounce-by` keyed by `buffer-key` (the index's [Debouncing](../README.md#debouncing)).
  A debounced call sees only the last of several fires, so it ignores the hook's hunks and
  asks for everything since `"seen"`. While a walk runs, the refresh does nothing, since
  the walk's next tick catches up.

The update itself:

- **Adding.** Every word of the new lines joins the set, with its case twin.
- **Removing.** A word of the old lines that the new lines don't contain may be gone, or
  may still occur elsewhere. `bw/present?` searches for it with
  `(buffer-search pane (word-search-pattern pane w))`, the whole-word pattern `*` uses,
  searching the all-lowercase twin when the word is capitalized. An all-lowercase pattern
  ignores case, so one search answers for the word and its twin, and both leave the set
  when nothing matches. Only words already in the set are checked, so a walk that hasn't
  reached a line doesn't search for its words.
- **Rebuilding.** When a word joins or leaves a finished index, the word list
  `Ctrl-Space` emits is rebuilt from the set, in time proportional to the number of
  distinct words. An edit that adds and removes nothing returns the same set and skips it.

The walk restarts from the cursor instead when the update has nothing to start from or
would block for too long:

- `buffer-changes-since` returns `#f`;
- the hunks' lines hold more than `'chars` characters;
- more than `bw/max-word-checks` (32) words need a search. A search for a word that is gone
  reads the whole buffer and blocks the editor while it does, where a walk yields between
  ticks.

`on-buffer-open` and a global `word-chars` change also restart it.

### Staleness

Every restart (`bw/reindex!`) cancels the pending tick, bumps the entry's `"gen"` and
empties the in-progress set, which a restart exists to replace. A tick closes over the
`"gen"` it started under and does nothing once the entry's has moved on. Cancelling
already stops the tick, even once it is due, so `"gen"` is a second guard (the index's
[Stale async work](../README.md#stale-async-work)).

`on-buffer-close` forgets the entry and cancels its tick, and `on-buffer-open` forgets it
before it reindexes. A debounced refresh can fire after the entry is gone. It then calls
`bw/reindex!`, which creates a fresh entry only while `buffer-live?` holds, so a refresh
that outlives a close does not recreate state for a closed buffer.

### Double-buffered cache

Each entry keeps two sets: `"set"`, the last complete index, and `"building"`, the walk in
progress. When a walk finishes, its set becomes the complete index, kept both as a set for
later updates and as the word list `"words"` that a trigger passes straight to
`completion-emit!`. Until then the previous complete index keeps answering, so a deleted
word can linger for up to one refresh cycle. Before the first walk completes, a trigger
answers from the in-progress set, so the first `Ctrl-Space` after a buffer opens is not
empty.

### Case twins

`'match 'string`'s prefix gate is case-sensitive, so a word cached as written answers only
a prefix typed in the same case. A buffer holding `Apply` (capitalized because it started
a sentence) would offer nothing for `app` typed mid-sentence. `bw/add-word` therefore
inserts a plain word's other-case twin beside it. A word is plain when everything after
its first letter is already lowercase:

| Original | Twin | Rule |
|---|---|---|
| `apply` (all-lowercase) | `Apply` | Titlecase the first letter |
| `Apply` (Titlecase) | `apply` | Lowercase the first letter |
| `a` (single letter) | `A` | A one-letter word is plain |
| `HashMap`, `iPhone` (inner capital) | none | Flipping only the first letter would lose case the tail still carries |
| `MAX_LEN` (ALL-CAPS) | none | Same |
| `1st` (non-letter first) | none | There is no other case to flip to |

The case-sensitive gate does the rest: `app` matches only `apply` and `App` only `Apply`,
the vim `'infercase'` feel, with no sentence-boundary detection. The word and its twin are
two independent candidates, so a mixed-case identifier keeps its own form. A code buffer
also gets twins nobody wrote (`config` adds `Config`); `core:lsp` outranks this source on
any real score once the user types, so a buffer with an attached server rarely shows them.
Twins are computed at scan time, so the in-progress fallback carries them too, and they
are inserted in `'fuzzy` mode as well.

### Answering a trigger

The completion framework re-filters a source's items on every keystroke without calling
the source again: `'string`'s prefix gate and `'fuzzy`'s score both run in Rust. The
source therefore emits its whole cached list once per trigger and does not pre-filter in
Steel. It includes the word under the cursor, since the editor's ranking drops any item
equal to the live token for every source, recomputed on each keystroke.

A trigger is not told of later events, so a `Ctrl-Space` on a large buffer whose walk is
still partial would leave the menu at that partial answer. While a walk runs, the source
answers with `#:more #t`, which keeps the request open, and stashes the request id in
`"live-id"`. When the walk finishes, `bw/push-finished-answer!` emits the full list on that
id without `#:more`, which closes the request. A request made after the walk finished gets
its whole answer at once and stashes no id, so a later reindex leaves that menu alone. A
stale id, from a menu dismissed or replaced since, is dropped without effect.

### `word-chars` invalidation

`word-chars` is read on every scan tick, so a change during a walk applies to the lines not
yet scanned. A global `:set global word-chars=…` reindexes every open buffer through the
`on-option-change` hook. A buffer-scoped `:set buffer word-chars=…` does not, because that
hook is raised only by the global write path; see [Known limitations](#known-limitations).

### Non-ASCII words

Steel has no Unicode character-category table, so the scan hands each line to
`stdlib/split-words`, which reads the buffer's own `word-chars` and runs the native
`split-words` builtin: the same classifier the `w`/`b` motions use. A candidate is what a
`w` motion would select: `café` (combining accent) stays one word, `l'élément` splits at
the curly apostrophe, `foo—bar` splits at the em dash, and `foo-bar` is one word wherever
`word-chars` includes `-`.

## Design decisions

- **A cache built by a background walk, not a rescan per trigger as vim's `i_CTRL-N`
  does**, because the scan runs in interpreted Steel and a full rescan would block input
  on a large buffer.
- **A budget in characters, not lines**, because a tick's cost grows with the characters
  it scans. A line count would let a tick over a file of long lines (a markdown file with
  one paragraph per line) block input several times longer than a tick over code.
- **Windows fetched by line index, not sliced from a string at a character offset**,
  because `buffer-lines` seeks the rope by line, so a window costs the same wherever it
  is, while re-slicing at an offset costs time proportional to the offset.
- **A 16ms tick delay, not 0**, because a zero-delay timer is due the instant it is
  scheduled, which pins the event loop's wake timeout at zero for the whole walk: the loop
  never blocks on input and repaints an unchanged frame on every tick. The walk takes
  longer in wall-clock time, and nothing waits on it.

## Known limitations

- **Some deleted words stay offered.** A word stays while a form of it that differs only in
  case remains (`APPLY` keeps `apply`), since the search ignores case. It also stays while it
  ends a longer word that `word-chars` joins (`a$x` keeps `$x` when `$` is a word
  character), since `*`'s pattern can't check the character before such an edge. A word
  still in the buffer is never dropped.
- **A buffer-scoped `word-chars` change does not invalidate that buffer's cache.** Lines an
  edit changes are read under the new value, and every other line keeps the split the last
  walk made. Fixing it needs a Rust-side event this plugin cannot add.
- **A single line is scanned whole.** A very long line, a minified file for instance, costs
  one tick proportional to its length.

## Implementation notes

- `bw/updated-set` intersects the words to remove with the set before
  `hashset-difference`, because Steel's `hashset-difference` returns the symmetric
  difference: a twin the set never held (`ix` for `Ix`, when only `ıx` added `Ix`) would
  otherwise join it.
- `bw/walk!` clamps the backward window's upper bound to the live line count, as it does
  the forward window's. `buffer-lines` raises on a range past the end rather than clamping,
  and a buffer can shrink between ticks.
