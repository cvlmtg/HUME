# core:buffer-words

Offers every identifier already in the buffer as an Insert-mode completion. It works in
any buffer, including a scratch buffer or a `.txt` file where `core:lsp` has no server to
ask.

## Usage

```scheme
(declare-plugin! "core:stdlib")
(load-plugin! "core:buffer-words" #:config (hash "match" 'string "lines" 200))
```

- **Depends on:** `core:stdlib`: config validation calls `stdlib/config-enum`/
  `stdlib/config-integer` at load time.
- **Activates on:** `Ctrl-Space` (or a completion source's own trigger char) only. It
  has no `manifest.scm`, so it must be loaded eagerly (see the
  [core plugins index](../README.md#loading-model)).
- **User docs:** [Core Plugins](https://cvlmtg.github.io/HUME/core-plugins.html#core-buffer-words).

## Configuration

| Key | Default | Meaning |
|---|---|---|
| `"match"` | `'string` | `'string`: a case-sensitive prefix gate, the vim `i_CTRL-N` feel. `'fuzzy`: subsequence-scored like `core:lsp`'s own candidates, so the two compete on score rather than priority |
| `"lines"` | `200` | Lines fetched and scanned per side (before/after the cursor) on each background indexing tick. Lower trims the pause a keystroke can add near a huge buffer; raising it finishes indexing a large buffer in fewer ticks |

## How it works

Vim's `i_CTRL-N` rescans the buffer on every invocation, synchronously, in C,
affordable only because it's native. This plugin can't do that: the scan is interpreted
Steel. Instead, it keeps a per-buffer cache, built by a background walk that yields back
to the editor between ticks, and `Ctrl-Space` only ever reads whatever that walk has
indexed so far.

```
cursor line
     │
     ▼
  ◀── bwd-lines (200) ── │ fwd-lines (200) ──▶      one tick
  ◀────── bwd-lines (200) ──── │ ──── fwd-lines (200) ──────▶   next tick
        …outward in both directions until both hit a buffer edge…
```

### Cursor-outward, line-windowed indexing

The cache is rebuilt on `on-buffer-open` and, debounced 150ms, on `on-text-changed`,
the same shape `core:git-diff` uses for its own per-buffer state (see the
[core plugins index](../README.md#per-buffer-state)). Each rebuild walks outward from the
cursor's line in both directions, one bounded batch of lines (`"lines"`) per direction
per tick, fetched with `(buffer-lines pane #:start #:end)` and scanned to completion in
the same tick: a word can't span two lines, so there's nothing to carry across a tick
boundary. This buys three things: nothing blocks, however large the buffer; the words
nearest the cursor (the ones most likely to matter right now) are indexed first; and an
edit that interrupts a walk (cancelling it, restarting from the new cursor position) has
already covered the region most likely to have changed.

This *reorders* the work, it doesn't reduce it: `on-text-changed` hands the hook only the
buffer id, no edit range, so every refresh is a full rescan of the buffer, the same cost
`core:git-diff` already pays on every debounced edit. A version of `on-text-changed` that
carried a change range would let a refresh touch only what changed; that's a real,
separate change to the editor's event system, not something a plugin can express on its
own.

The walk fetches by line, not by re-slicing a plain string at a char offset: finding an
arbitrary offset in a UTF-8 string costs time proportional to that offset on every call,
so a chunked walk built that way would cost *more* total work than one unchunked pass,
growing quadratically with buffer size. `buffer-lines` seeks into the buffer's rope by
line index instead, so fetching a growing window of lines stays cheap however far into
the buffer that window is. No giant in-memory list of the whole buffer is ever held:
each tick converts only the handful of lines it's about to scan, so memory use during a
walk is bounded by `"lines"`, not by buffer size. One line is always read and scanned
whole, uncapped. A pathologically huge single line (a minified file, say) costs one tick
proportional to its own length rather than being split across several.

Each tick reschedules itself after a short, non-zero delay (16ms) rather than firing
immediately: a zero-delay timer is already due the instant it's scheduled, which pins the
event loop's wake timeout at zero for the whole walk. The loop never blocks on input and
repaints a full, unchanged frame once per tick, as fast as it can, until the walk
finishes. A large buffer's walk is hundreds of ticks; that's hundreds of full
display-line rebuilds and terminal flushes producing no visible change. A short delay
lets the loop block between ticks instead, at the cost of the walk taking proportionally
longer in wall-clock time, an acceptable trade, since nothing waits on the walk
finishing.

The anchor line comes from the cursor's line, resolved through `(buffer-panes pane)`
first (see the [core plugins index](../README.md#pane-values-vs-pane-less-values)).
`on-buffer-open`/`on-text-changed` hand the hook a pane-less value, and reading the
cursor directly needs a real pane. A background buffer (never focused, or edited by a
script or an LSP `applyEdit`) walks from the top instead, since it has no meaningful
cursor to anchor to. The split at the cursor is line-grained: the forward side scans the
cursor's own line from its start, and the backward side starts at the line strictly
before it.

### Staleness and cancellation

A tick closes over the generation the walk started under (`"gen"`, bumped by every
restart) and no-ops unless the entry's current generation still matches: the standard
stale-async guard described in the
[core plugins index](../README.md#stale-async-work). This matters here specifically
because `cancel-timer!` cannot stop a tick that's already been dequeued off the timer
wheel and queued to run: since a new walk can start before that orphaned tick fires, "the
entry still exists" alone isn't enough to tell the old walk apart from the new one; the
generation check is.

Closing the buffer this plugin's entry belongs to doesn't always mean the entry should be
dropped for good: closing drops it as usual, but a debounced reindex tick that outlives
its own entry (queued before a close, draining after) resurrects it rather than leaving
the index dead for the rest of the session. This is gated on the buffer still being open, so a
debounced reindex that outlives a *genuine* close doesn't resurrect state for a buffer
that's actually gone.

The backward window's upper bound is clamped against the live line count the same way
the forward window's is: both sides carry their anchor across ticks, and a buffer that
shrinks mid-walk (`:e!` onto a shorter file, a large undo, an LSP `applyEdit`) would
otherwise let the backward side ask for a range past the buffer's new end, which
`buffer-lines` raises on rather than clamps.

The in-progress word set is reset to empty on every restart, along with bumping the
generation. A cancelled walk's partial set is likely stale by the time a new one starts
(the reindex was itself triggered by an edit), so the fresh walk starts from nothing. On
a large buffer, an edit that keeps interrupting the walk near the cursor (the common
case: a user typing continuously) means the far ends never finish indexing as long as
edits keep arriving (see [Known limitations](#known-limitations)).

### Double-buffered cache

Each buffer keeps two sets: the last *complete* index (what `Ctrl-Space` reads) and the
walk in progress. The in-progress set is rebuilt tick by tick; once a walk finishes, it's
promoted into a plain word list a trigger can hand straight to `completion-emit!`. Until
then, the complete index keeps answering triggers untouched. A deleted word can therefore
linger for up to one refresh cycle, the same bounded staleness the completion framework
already accepts elsewhere ("the old answer stays ranked until the new one lands, so the
menu never blinks empty"). Before the first walk ever completes there is no complete
index yet, so a trigger falls back to the in-progress set's own word list instead;
otherwise the very first `Ctrl-Space` after a buffer opens would show nothing.

### Case twins

`#:match 'string`'s prefix gate is case-sensitive, so a word cached exactly as written
only ever answers a prefix typed in that same case. A buffer holding `Apply` (capitalized
because it started a sentence) then offers nothing for `app` typed mid-sentence, where
the word is never capitalized. The case that's actually typed most of the time is the
one the cache can't answer.

Instead of lowercasing every candidate at scan time (lossy and irreversible: `HashMap` →
`hashmap`, `MAX_LEN` → `max_len`, a proper noun loses its capital; and it only relocates
the same bug to sentence starts), `bw/add-word` inserts a *plain* word's other-case twin
alongside it:

| Original | Twin | Why |
|---|---|---|
| `apply` (all-lowercase) | `Apply` | Titlecase, "plain" (everything after the first letter is already lowercase) |
| `Apply` (Titlecase) | `apply` | all-lowercase |
| `HashMap`, `iPhone` (inner capital) | *(none)* | Flipping only the head would destroy case information the tail still carries |
| `MAX_LEN` (ALL-CAPS) | *(none)* | Same reason |

The existing case-sensitive gate then does the rest for free: typing `app` matches only
the `apply` twin, typing `App` matches only `Apply`: the vim `'infercase'` feel, with no
sentence-boundary detection anywhere in this plugin. Nothing is ever rewritten. Both the
original word and its twin sit in the index as two independent candidates, so a genuine
mixed-case identifier never has one flipped away.

This does mean a plain lowercase identifier in a code buffer also gets a capitalized
twin that was never actually written (`config` conjures `Config`), and vice versa. This is
accepted rather than gated behind a config key, since `core:lsp` already outranks
`core:buffer-words` on any real score once the user types anything, so a code buffer with
an attached server rarely surfaces the fake twin at all.

The twin is computed once per word per walk, at scan time, not as a second pass over the
finished set. Scanning stays a bounded per-tick cost either way, and computing it inline
means the not-yet-finished fallback (see [Double-buffered cache](#double-buffered-cache)
above) already carries twins for whatever's been walked so far, with no second site to
keep in sync. In `#:match 'fuzzy` mode the same twins are still inserted (there's no
second case-twin rule for fuzzy matching), so a lowercase fuzzy query can match both a
word and its Titlecase twin.

### Pushing a finished index to an open menu

A trigger only ever answers the invocation it was called for. It has no way to notice a
*later* event on its own. That's fine for typing (the editor's own re-ranking tracks the
live cursor on every keystroke with no help from this plugin) but not for the background
walk finishing: if `Ctrl-Space` lands on a large buffer whose walk is still partial,
sitting still with the menu open would otherwise leave it stuck at that partial answer
forever.

So the completion source stashes the invocation id it was last called with, one per
buffer, and the walk pushes straight to that id (via a second, unsolicited
`completion-emit!` call, not a fresh answer to a fresh question) the moment it promotes
the in-progress set to the complete index. `completion-emit!` accepts this the same way
it accepts an LSP source streaming a second answer for a still-open call: the id just has
to still be the latest one for this source's slot. A stale id (the menu was dismissed,
or a later trigger already replaced it with its own) is silently dropped; this plugin
never finds out which, since there's no hook that would tell it a menu closed.

### Matching

The completion framework re-filters this source's items against whatever's typed on
every keystroke, without re-invoking the source (`#:match 'string`'s prefix gate, or
`#:match 'fuzzy`'s subsequence score, both run in Rust). So this plugin emits its
*whole* cached set once per trigger rather than pre-filtering by what's typed so far in
Steel. The framework never re-invokes this source on its own; the plugin still pushes a
second answer itself when the background walk finishes (see
[Pushing a finished index to an open menu](#pushing-a-finished-index-to-an-open-menu)
above).

The scan collects the word under the cursor like any other word, but offering it back
would be a no-op to accept, but this plugin doesn't filter it out itself. The editor's own
ranking drops any item that exactly matches the live token before scoring, for every
source, not just this one; it re-derives that comparison from the live cursor on every
keystroke, so typing past an excluded word (`ca` → `cat`) drops `cat` the instant it's
typed, and backspacing away from it brings it straight back.

### `word-chars` invalidation

`word-chars` is read once per reindex and baked into the cached word set for as long as
that index stands. Nothing about the cache invalidates on its own when the option
changes later. A global `:set global word-chars=…` reindexes every open buffer, so that
case stays live. A *buffer-scoped* `:set buffer word-chars=…` does not: the corresponding
hook is raised only by the global write path, never by the buffer-scoped one, so there is
no Steel-visible event this plugin can subscribe to for that case (see
[Known limitations](#known-limitations)).

### Non-ASCII words

Steel has no Unicode character-category table (no `char-alphabetic?`, no regex), so
classification isn't done in Steel at all: the scan calls the native `split-words`
builtin, which tokenizes a whole line at once using the same word/character classifier
`w`/`b` motions and text objects use (`hume-ops`'s word-motion scan primitives, built on
grapheme-cluster-safe stepping). A word this plugin offers is, by construction, exactly
what a `w` motion would select: `café` (with a combining accent) stays one candidate,
`l'élément` splits at the curly apostrophe into `l` and `élément`, and `foo—bar` (em dash)
splits into `foo` and `bar`, with no approximation left to apologize for. `word-chars` is
honored per buffer exactly as it is for motions, so `foo-bar` merges into one candidate
wherever `word-chars` includes `-` (a CSS buffer, say) and stays two words everywhere
else.

## Known limitations

- **A full rescan runs on every edit.** `on-text-changed` carries no change range, so
  cursor-outward indexing reorders the work without reducing it (see
  [Cursor-outward, line-windowed indexing](#cursor-outward-line-windowed-indexing) above).
- **Continuous typing near the cursor can starve the far ends of a large buffer.** Every
  restart discards in-progress work (see
  [Staleness and cancellation](#staleness-and-cancellation) above).
- **A buffer-scoped `word-chars` change doesn't invalidate that buffer's cache** (see
  [`word-chars` invalidation](#word-chars-invalidation) above). Fixing this needs a
  Rust-side event this plugin can't add on its own.
- **No length cap on a single line's scan**: a pathologically huge single line costs one
  tick proportional to its own length. A `max-word-len` option to cap word length within
  a line is a plausible future addition if it ever matters in practice.
