# core:buffer-words

Offers every identifier already in the buffer as an Insert-mode completion —
works in any buffer, including a scratch buffer or a `.txt` file where
`core:lsp` has no server to ask.

## Usage

```scheme
(declare-plugin "core:stdlib")
(load-plugin "core:buffer-words")
```

Loads eagerly: nothing but `Ctrl-Space` (or a completion source's own
trigger char, if one applies) can ever invoke the completion source it
registers, so a lazy `declare-plugin` would have no other trigger to
activate it — `core:pickers`/`core:vim-keybind`/`core:classic-paste` are
the same shape, for the same reason. Requires `core:stdlib` declared or
loaded first — config validation calls `stdlib/config-enum`/
`stdlib/config-integer` via `call!` while this plugin's body evaluates.

## Configuration

```scheme
(load-plugin "core:buffer-words"
  #:config (hash "match" 'string "lines" 200))
```

| Key | Default | Meaning |
|---|---|---|
| `"match"` | `'string` | `'string` — a case-sensitive prefix gate, the vim `i_CTRL-N` feel; `'fuzzy` — subsequence-scored like `core:lsp`'s own candidates, so the two compete on score rather than priority |
| `"lines"` | `200` | Lines fetched and scanned per side (before/after the cursor) on each background indexing tick. Lower trims the pause a keystroke can add near a huge buffer; raising it finishes indexing a large buffer in fewer ticks |

## How it works

### Cursor-outward, line-windowed indexing

Vim's `i_CTRL-N` rescans the buffer on every invocation, synchronously, in
C — affordable only because it's native. This plugin can't have that: the
scan is interpreted Steel. So it keeps a per-buffer cache instead, rebuilt
on `on-buffer-open` and (debounced 150ms) `on-text-changed`, the same shape
`core:git-diff` uses for its own per-buffer state.

The rebuild walks outward from the cursor's line in both directions, one
bounded batch of lines (`"lines"`) per direction per tick, fetched with
`(buffer-lines bid #:start #:end)` and scanned to completion in the same
tick — a word can't span two lines, so there's nothing to carry across a
tick boundary. Each tick yields back to the editor loop via a short `after`
delay before the next runs (see below for why not zero). That buys three
things: nothing blocks, however large the buffer; the words nearest the
cursor — the ones most likely to matter right now — are indexed first; and
an edit that interrupts a walk (cancelling it, restarting from the new
cursor position) has already covered the region most likely to have
changed. `Ctrl-Space` never scans — it only ever reads whatever the
background walk has indexed so far.

Worth being precise about what this buys and what it doesn't: starting at
the cursor *reorders* the work, it doesn't reduce it. `on-text-changed`
hands the hook only the buffer id, no edit range, so every refresh is a
full rescan of the buffer — the same cost `core:git-diff` already pays on
every debounced edit. A version of `on-text-changed` that carried a change
range would let a refresh touch only what changed; that's a real, separate
change to the editor's event system, not something a plugin can express on
its own, and isn't worth building until a second real consumer motivates it
the way this plugin motivated cursor-outward indexing itself.

The walk fetches by line, not by re-slicing a plain string at a char
offset: finding an arbitrary offset in a UTF-8 string costs time
proportional to that offset, on every call, so a chunked walk built that
way would cost *more* total work than one unchunked pass, growing
quadratically with buffer size. `(buffer-lines bid #:start #:end)` doesn't
have that problem — it seeks into the buffer's rope by line index, not by
scanning from the start — so fetching a growing window of lines is genuinely
cheap however far into the buffer that window is. No giant in-memory list of
the whole buffer is ever held: each tick converts only the handful of lines
it is about to scan, so memory use during a walk is bounded by `"lines"`,
not by buffer size.

One line is always read and scanned whole, uncapped — a pathologically huge
single line (a minified file, say) costs one tick proportional to its own
length rather than being split across several. Accepted as out of scope; a
`max-word-len` option to cap word length within a line is a plausible future
addition if it ever matters in practice.

The split at the cursor is line-grained: the forward side scans the
cursor's own line from its start, and the backward side starts at the line
strictly before it — so words earlier on the cursor's own line are indexed
together with the rest of that line on the first tick, rather than
prioritized separately from it. This is a minor reordering of *when* a word
is found relative to the old char-offset split, never a change to whether
it's found.

The anchor line comes from `(current-line-number)`, which takes no `bid`
and always answers for the focused buffer — there is no way to read
another buffer's cursor position at all. A background buffer (never
focused, or edited by a script or an LSP `applyEdit`) walks from the top
instead, since it has no meaningful cursor to anchor to.

Each tick re-reads the buffer's entry fresh from the shared table rather
than trusting a binding captured before the previous tick's update —
Steel's hashes are persistent, so a stale binding still points at the
pre-update snapshot. A tick silently no-ops if the entry is gone entirely:
closing a buffer cancels its pending timer, but a tick already queued past
that cancellation's reach needs this guard too. The same cancel-before-
replace guard runs when `:reload-config` replays `on-buffer-open` on an
already-open buffer — without it, a walk already in flight would keep
running against the entry that replay just replaced.

"The entry is gone" isn't the only stale-tick shape: `cancel-timer!` cannot
stop a tick that's already been dequeued off the timer wheel and queued to
run — cancelling at that point is a no-op, and the tick fires anyway against
whatever entry now exists. Since `bw/reindex!` can start a *new* walk before
that orphaned tick runs, "an entry exists" alone isn't enough to tell the old
walk apart from the new one — the orphan would union its stale words into the
new walk's `"building"` set and clobber its `"timer"` slot with its own,
leaving the live chain uncancellable. Each entry therefore also carries a
`"gen"` counter, bumped by every `bw/reindex!`; a walk closes over the
generation it started under and a tick no-ops unless the entry's `"gen"`
still matches. `bw/cancel-timer!` stays — it still saves a wasted tick when
it lands in time — but `"gen"` is what makes correctness not depend on that
timing.

Each tick reschedules itself with a short, non-zero delay rather than
`(after 0 …)`: a zero-delay timer is already due the instant it's scheduled,
which pins the event loop's wake timeout at zero for the whole walk — the
loop never blocks on input and repaints a full, unchanged frame once per
tick, as fast as it can, until the walk finishes. A large buffer's walk is
hundreds of ticks; that's hundreds of full display-line rebuilds and
terminal flushes producing no visible change. A short delay lets the loop
block between ticks instead, at the cost of the walk taking proportionally
longer in wall-clock time — an acceptable trade, since nothing waits on the
walk finishing (`Ctrl-Space` reads whatever's indexed so far, complete or
not).

The in-progress `"building"` set survives a `bw/reindex!` restart instead of
being reset to empty. The walk is monotone — each tick only ever adds words,
never removes them — so on a large buffer, an edit that keeps interrupting
the walk near the cursor (the common case: a user typing continuously) still
makes progress at the far ends across restarts, rather than the walk
restarting its far-end coverage from nothing on every keystroke.

The backward window's upper bound is clamped against the live line count the
same way the forward window's is (`fwd-hi`) — both sides carry their anchor
across ticks, and a buffer that shrinks mid-walk (`:e!` onto a shorter file,
a large undo, an LSP `applyEdit`) would otherwise let the backward side ask
for a range past the buffer's new end, which `buffer-lines` raises on rather
than clamps.

Closing the buffer this plugin's entry belongs to doesn't always mean the
entry should be dropped for good. Closing the *last* open buffer reuses that
same `BufferId` in place for a fresh scratch buffer rather than opening a new
one — and that reuse fires no `on-buffer-open`, only the `on-text-changed`
this plugin already reacts to (the swap still bumps the buffer's text
generation). `bw/forget!` drops the entry on `on-buffer-close` as usual;
`bw/reindex!` resurrects a missing entry before indexing rather than
no-opping, so the debounced `on-text-changed` that follows a few keystrokes
into the replacement scratch buffer rebuilds it instead of leaving the index
dead for the rest of the session. The resurrection is gated on the buffer
still being open (`(member bid (buffers))`) — a debounced reindex that
outlives a *genuine* close, with nothing reusing the id, must not
resurrect state for a buffer that's actually gone, the same reason
`core:git-diff`'s `entry-set!` no-ops rather than resurrects
(`runtime/plugins/core/git-diff/state.scm`) while its own `ensure-entry!`
does the opposite for a write path that must succeed regardless.

### Double-buffered cache

Each buffer keeps two sets: the last *complete* index (what `Ctrl-Space`
reads) and the walk in progress. The in-progress set is a raw hashset,
rebuilt tick by tick; once a walk finishes, it's promoted into the ready
completion-item shape (`(hash "label" w)` per word) a trigger can hand
straight to `completion-emit!` with no per-keystroke rebuild. Until then,
the complete index keeps answering triggers untouched. A deleted word can
therefore linger for up to one refresh cycle — the same bounded staleness
the completion framework already accepts elsewhere ("the old answer stays
ranked until the new one lands, so the menu never blinks empty"). Before the
first walk ever completes there is no complete index yet, so a trigger
falls back to mapping the in-progress hashset into that same shape instead
— otherwise the very first `Ctrl-Space` after a buffer opens would show
nothing.

### Pushing a finished index to an open menu

A trigger only ever answers the invocation it was called for — it has no
way to notice a *later* event on its own. That's fine for typing (the
editor's own re-ranking, below, tracks the live cursor on every keystroke
with no help from this plugin) but not for the background walk finishing:
if `Ctrl-Space` lands on a large buffer whose walk is still partial, sitting
still with the menu open would otherwise leave it stuck at that partial
answer forever — walk progress isn't something the user *typed*, so nothing
would ever ask this source again.

So the completion source stashes the invocation id it was last called with
(`bw/set-live-id!`, one id per buffer, in the same per-buffer entry
everything else here lives in), and `bw/walk!` pushes straight to that id
— via a second, unsolicited `completion-emit!` call, not a fresh answer to
a fresh question — the moment it promotes `"building"` to `"words"`
(`bw/push-finished-answer!`). `completion-emit!` accepts this the same way
it accepts an LSP source streaming a second answer for a still-open call:
the id just has to still be the latest one for this source's slot. A stale
id — the menu was dismissed, or a later trigger already replaced it with
its own — is silently dropped; this plugin never has to find out which,
since there's no hook that would tell it a menu closed.

### Matching

The completion framework re-filters this source's items against whatever's
typed on every keystroke, without re-invoking the source
(`#:match 'string`'s prefix gate, or `#:match 'fuzzy`'s subsequence score —
both run in Rust). So this plugin emits its *whole* cached set once per
trigger rather than pre-filtering by what's typed so far in Steel — the
cached set is exactly what `bw/walk!` found, no per-keystroke rebuild. The
*framework* never re-invokes this source on its own (`completion-emit!`
carries no `#:incomplete`); the plugin still pushes a second answer itself
when the background walk finishes — see "Pushing a finished index to an
open menu" above.

The scan collects the word under the cursor like any other word, but
offering it back would be a no-op to accept — this plugin doesn't filter it
out itself. The editor's own ranking (`CompletionSession::rank`) drops any
item that exactly matches the live token before scoring, for every source,
not just this one; it re-derives that comparison from the *live* cursor on
every keystroke, so typing past an excluded word (`ca` → `cat`) drops `cat`
the instant it's typed, and backspacing away from it (`cat` → `ca`) brings
it straight back — both for free, since the comparison is never pinned to
whatever was typed at trigger time the way a Steel-side filter would be.

### `word-chars` invalidation

`word-chars` is read once per `bw/reindex!` and baked into the cached word
set for as long as that index stands — nothing about the cache invalidates
on its own when the option changes later. A global `:set global word-chars=…`
reindexes every open buffer (`on-option-change`), so that case stays live.
A *buffer-scoped* `:set buffer word-chars=…` does not: `on-option-change`
is raised only by the global write path, never by the buffer-scoped one, so
there is no Steel-visible event this plugin can subscribe to for that case.
That buffer's index stays stale — classifying by the old value — until its
next edit triggers the normal debounced reindex. Closing this gap needs a
Rust-side event this plugin cannot add on its own.

### Non-ASCII words

Steel has no Unicode character-category table (no `char-alphabetic?`, no
regex), so classification isn't done in Steel at all — `bw/reindex!` reads
`bid`'s own `word-chars` once per walk and the scan calls native
`(split-words line word-chars)` per line, a builtin that tokenizes a whole
line at once using `hume-editing`'s `WordChars`/`CharClass` machinery, the
same classifier `w`/`b` motions and text objects already use. A word this
plugin offers is, by construction, exactly what a `w` motion would select:
`café` (with a combining accent) stays one candidate, `l’élément` splits at
the curly apostrophe into `l` and `élément`, `foo—bar` (em dash) splits into
`foo` and `bar`, and CJK punctuation behaves the same way — no approximation
left to apologize for. `word-chars` is honored per buffer exactly as it is
for motions, so `foo-bar` merges into one candidate wherever `word-chars`
includes `-` (a CSS buffer, say) and stays two words everywhere else.
