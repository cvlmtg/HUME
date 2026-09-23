# HUME — Completion Sources

Design record for the completion system: one model in which a *source*
(native Rust, or Steel-registered — `core:lsp` is one) is registered once
against one of two targets (Insert mode, or the `:` command line — each
with its own fixed token rule, not the source's own choice), invoked by one
orchestrator, and answers a specific *invocation* that carries its own
document snapshot and span. Several sources rank together in one menu, each
against its own target's token.

The sibling fuzzy-finder (picker) shipped as `core:pickers` (roadmap for
what's left: `docs/FUZZY-FINDERS.md`). The two share the "Rust store, Steel
policy" pattern and the same `hume-editor/src/editor/fuzzy.rs` matcher (each
with its own `FuzzyProfile`); see `hume-editor/src/editor/input_stack/picker/
session.rs`'s module doc for why they stay separate session types.

**Status: shipped**, including the buffer-words source
(`runtime/plugins/core/buffer-words/`, see its own README for design
rationale) — everything below describes the code as it is.

## How to use this document

Same rules as `docs/LSP.md`:

1. **Verify before you write.** The codebase moves — `rg 'symbol_name'`
   before relying on anything named here. No line numbers anywhere in this
   doc — navigate by symbol search.
2. **If the doc contradicts the code, STOP** and report; don't silently
   adapt.
3. Project-wide rules from `CLAUDE.md` apply (no `.unwrap()` outside tests,
   grapheme discipline, every command tested).

## Architecture constraints inherited from LSP work

These are settled project decisions (see `docs/LSP.md` Decisions table) and
this design respects them:

- **Frequency cut**: per-user-intent work (a trigger keypress, an answer
  arriving, a selection made) may run in Steel; per-keystroke filtering,
  per-frame rendering, and unbounded-collection work must be Rust.
- **Bulk-data guardrail**: bulk item lists never cross the Rust↔Steel
  boundary on recurring paths. One-time ingest at user-intent frequency is
  the calibrated exception (measured: ~1ms for 1k completion items through
  the boundary — acceptable; do not assume this scales to 100k file paths).
- **Steel never on the render path**: Steel writes models/stores; Rust
  providers render from `Arc<RwLock<…>>` snapshots each frame.
- **Rust-rendered, Steel-fed widgets**: "LSP is their first client, not
  their owner."

---

## The model

Six concepts, each with one owner — everything under
`hume-editor/src/editor/completion/`:

| Concept | Type | Where |
|---|---|---|
| **Source** — a named producer of candidates, with its static facts: how its items score, its priority, and its body (a native fn or a Steel proc). Two separate namespaces, one per target — a name in one has no bearing on the same name in the other | `BufferSourceEntry`/`MinibufSourceEntry` in `SourceRegistry` (`buffer`/`minibuf` fields, `BufferSourceId`/`MinibufSourceId` index them) | `registry.rs`; the registry lives on `ConfigState.completion_sources`, so `:reload-config` rebuilds it from the natives by construction |
| **Invocation** — one call of one source for one trigger: the id the source answers to, the document snapshot it saw (`rope` + `head` + every edit observed since, composed, `Buffer`-target only), its token span in live coordinates, and its answer once it has one | `Invocation<S>` (generic over the span shape, `BufferSpan`/`MinibufSpan`) | `session.rs` |
| **Session** — the one open session: one `SourceSlot<Id, S>` per participating source (its latest `shown` invocation and, if re-invoked since, the newer `inflight` one), the ranked `(slot, item)` index, the matcher, a rank-time dedup mask | `CompletionSession` | `session.rs`; `session/accept.rs` applies the accepted item |
| **Target** — where an accepted item lands, and each target's own slots, typed against that target's own id/span shape so a `Minibuf` session can't hold `Buffer` coordinates or vice versa | `Target::{Buffer{bt,slots}, Minibuf{mt,slots}}` | `session.rs` |
| **Orchestrator** — the one driver for both targets: picks the sources a trigger applies to, mints invocations, runs them, lands answers, reacts to edits, applies the `:` line's eager policy | `impl EditorState` | `orchestrate.rs` |
| **Layer** — keys, selection, render sync | `CompletionLayer` | `input_stack/completion.rs` |

### Sources

```rust
struct BufferSourceEntry { name, match_kind, priority, proc: SteelVal, resolve: bool, trigger_chars }

enum MinibufBody {
    NativeUniverse(fn(&CompletionCtx) -> Vec<CompletionItem>),          // 'arg span
    NativeDelegated(fn(&str, usize, &CompletionCtx) -> (Range<usize>, Vec<CompletionItem>)),  // own span
    Steel(SteelVal),
}
struct MinibufSourceEntry { name, match_kind, priority, body: MinibufBody }
```

Every `Buffer` source is Steel, by design — there is no native `Buffer`
shape to have a body enum over; the buffer-words source
(`runtime/plugins/core/buffer-words/`) dogfoods this plugin-facing API
rather than the Rust-internal machinery the six native minibuffer sources
already validate. A `Buffer` source's own token is always the identifier
before the cursor, a `Minibuf` source's always the whitespace-delimited
argument — the rule belongs to the *target*, not a per-source choice, so
there is no separate token keyword to decode.

The six native minibuffer sources (`command`, `buffer-name`, `theme` —
`NativeUniverse`; `path`, `path-dirs-only`, `set` — `NativeDelegated`) are
compiled in. `TypedCommand.completer` (`Option<Cow<'static, str>>`) names any
entry by name — a built-in's `&'static str` constant, or the runtime string
`define-typed-command! … #:complete "name"` hands over.

**Every buffer source's token is the identifier before the cursor; every
minibuffer source's is the whitespace-delimited argument the cursor is in —
except `NativeDelegated`, which computes its own span.** The editor resolves
a `Buffer` source's token *before* the source runs, against the invocation's
own snapshot: `hume_ops::edit::word_start_before(text, head,
word_chars)..head` (the seeded filter is that text, handed to the proc as
its `prefix` argument). A `Minibuf` source's token is likewise resolved
upfront, via `arg_prefix`/`token_end_at` (the framework's own command-line
grammar) — except `NativeDelegated`, whose candidate universe *is* the live
input, so the orchestrator calls its function first and takes the span it
returns (`:e`'s path, `:set`'s phase-dependent token). Nothing in the
framework guesses a boundary on a source's behalf, and nothing forces one
source's boundary on another. An earlier design let a source name its own
span via a `'custom` token and `#:span` on every answer, and a buffer source
could ask for no seeding at all via a `'cursor` token; no shipping source
ever used either, so both were removed rather than carried as unvalidated
surface.

### Invocations and answers

A trigger mints one `Invocation` per source (`widget_token::next()` for its
id), its span already resolved (above) — a native source answers inline; a
Steel one is *queued* via `EditorState::queue_steel_call` — `(proc id bid
prefix)` for a buffer source, `(proc id input cursor)` for a `:`-line one —
and answers with `(completion-emit! id items #:incomplete)`, sync or from
any later callback, exactly once. An empty list is "nothing from this
source".

**An answer applies only to the latest call of its slot.** Re-invoking a
source (a later keystroke while its last answer was `isIncomplete`, a second
Ctrl-Space, a trigger char) supersedes the earlier call: its id goes stale,
and an answer carrying it is dropped (`completion-emit!` returns `#f`). A
repeated answer for a still-latest id replaces the earlier one, so a source
may stream. This is the entire stale-async story — there is no session
token, no `async_opener_stale` gate, and no way for a slow LSP response to
overwrite a newer one. The old answer stays ranked (against the new token
text) until the new one lands, so the menu never blinks empty.

### Edits

`Editor::apply_insert_edit` — the chokepoint every Insert-mode keystroke that
lands an edit goes through — calls `EditorState::completion_observe_edit`,
which:

1. composes the `ChangeSet` into *every* invocation's `cs_since` (shown and
   in-flight alike) and remaps each live span — `start` with `Assoc::Before`
   (text inserted exactly at the token's start belongs to the token), `end`
   with `Assoc::After` (text typed at its end extends it);
2. drops a slot's answer if the cursor left its token (`head ∉ [start,
   end]`) or the character *before* the token was deleted — detected via
   `PosMapCursor::map_anchor`'s `anchor_deleted` on `start - 1`, which
   answers "was the token's own start character deleted?" directly, rather
   than inferring it from two positions mapping to the same spot (a
   deletion elsewhere, a second cursor's, say, cannot trigger it). Deleting
   the token's own first char stays inside it; a slot narrowed to zero *matches*
   is still live (Backspace brings its items back). A dropped slot also
   rebuilds the cross-source dedup mask (Q-A1, below) — the item set it
   compared against changed;
3. re-ranks (`CompletionSession::rank`: score desc, source priority desc,
   sortText asc, index asc — each shown item scored against *its own* slot's
   token text with its source's `MatchKind`, after skipping a no-op item and
   any item the dedup mask already hid), resets the menu selection;
4. re-invokes every source whose last answer was `isIncomplete` or that is
   still pending against the pre-edit document; and
5. dismisses the session, silently, once no slot has an answer with items
   and nothing is in flight.

Auto-pair skip-close — typing a closer the cursor already sits on just moves
past it, via a motion rather than an edit — is the one Insert-mode keystroke
that bypasses this chokepoint entirely: with no `ChangeSet` to remap a
session's tokens against, it dismisses the session outright instead.

A `ChangeSet` not built against the session's tracked length is an edit the
session never saw; `observe_edit` refuses it and the session is dismissed.
Edits that bypass the chokepoint entirely (an LSP `applyEdit`, `:e!`) are
caught by `Editor::dismiss_invalid_completion`'s settle-time generation check.

### Accept

`CompletionSession::accept` reads everything from the *selected item's own*
invocation: a server `textEdit` (and `additionalTextEdits`) decodes against
that invocation's `rope` and maps through its `cs_since`; the `insertText`
fallback replaces its live token span. Both land as one undo step at every
cursor, with the uniform `(back, forward)` distance model and containment
check `session/accept.rs` documents. `completionItem/resolve` follows when
the item's own source declared `#:resolve #t` at registration, it lacked
`additionalTextEdits`, and the server offers `resolveProvider` — the source
flag (`core:lsp` sets it, `core:buffer-words` doesn't) keeps a second
`Buffer` source sharing the same LSP-attached buffer from having a resolve
request sent for an item the server never produced.

### The `:` line

`trigger_minibuf_completion` resolves the one source the input shape names
(the command name itself, or the command's declared completer), runs it, and
applies the eager policy once nothing is pending: no candidate → no popup;
exactly one → applied silently; two or more → the first applied and the
popup open for Tab to cycle. Cycling restores the input every source saw and
splices the selected candidate over *its* source's span — idempotent in
those coordinates, with no "what did the previous candidate leave behind"
bookkeeping. A `Minibuf` source may answer asynchronously (the popup opens
empty and the policy runs when the answer lands); this path is exercised by
`a_steel_minibuf_source_completes_a_typed_commands_argument` but has no real
caller yet.

### Rendering

`sync_completion_menu_view`/`sync_minibuf_completion_view` (and
`show-menu!`'s `sync_menu_view`) all go through `hume_ui::popup::menu_window`
— the visible window from *counts alone* — then `CompletionSession::rows_in`
for exactly that range, then `resolve_menu`, which measures width over the
rows it is handed. There is no full-list row accessor, so a candidate
scrolled out of view cannot inflate the box: the property is unwritable,
not merely untested-for. The menu anchors at the leftmost token start among
the sources with a ranked candidate.

## Steel surface

| Builtin | Notes |
|---|---|
| `(register-completion-source! name proc #:target #:match ['fuzzy] #:priority [0] #:resolve [#f])` | config-time; crosses as `Effect::RegisterCompletionSource`, so a failed activation's registration is never applied (the `Effect::BindKey` rationale); `#:target` picks one of two separate namespaces (`'buffer`, `'minibuf`) — re-registering a name replaces it, natives included, *within that namespace*; the same name in the other namespace is a second, unrelated source, since there is no shared id space between them to collide in. `#:resolve #t` (`'buffer` only) claims this source's items are wire items from the buffer's attached server, licensing `completionItem/resolve` on accept |
| `(completion-emit! id items #:incomplete [#f])` | the one way items enter a session; `#f` once `id` is stale |
| `(completion-top n)`, `(completion-accept! idx)`, `(completion-dismiss!)` | unchanged |
| `(register-trigger-chars! source language chars)` + `on-trigger-char` | shared, listener-agnostic table (signature help uses it) — *not* how a `'buffer` completion source's own trigger chars are joined |
| `(completion-set-trigger-chars! source language chars)` | a `'buffer` completion source's own trigger chars for `language`, replacing that pair's previous set; the editor invokes `source` directly when one lands, no hook round trip. Crosses as `Effect::SetCompletionTriggerChars` (a `register-completion-source!` queued earlier in the same eval may still be the one supplying `source`); `source` naming no registered `'buffer` source at apply time is reported as a log message, not raised back to the caller |
| `(define-typed-command! … #:complete "name")` | a `:` command's argument completer |
| command `completion-trigger` (Insert, default `Ctrl-Space`) | native; `(call! "completion-trigger")` from Steel |

`core:lsp/completion.scm` is the reference source: `register-completion-
source! "lsp"` with `#:priority 10 #:resolve #t`, whose proc sends
`textDocument/completion` (`#:supersede "completion"`) and answers with the
decoded list and its `isIncomplete` flag, declining with an empty answer when
the buffer's server has no `completionProvider`; `lsp/setup-trigger-chars!`
registers the server's trigger characters as `"lsp"`'s own, via
`completion-set-trigger-chars!`.

## Tests

- `hume-editor/src/editor/completion/session/tests.rs` — the store alone:
  per-slot tokens, the crossing rule, stale answers, no-op item drop,
  cross-source dedup.
- `hume-editor/src/editor/completion/registry/tests.rs` — the registry
  alone: namespace independence, trigger-char join/clear, trigger chars
  surviving a same-name re-registration.
- `hume-editor/src/editor/tests/completion/` — through the real seams, one
  file per concern: `sources.rs` (registration, invocation, token rules,
  stale answers, re-invocation, several sources), `menu_keys.rs`,
  `accept.rs`, `minibuf.rs`, `render.rs`.
- `hume-editor/src/editor/tests/unix/lsp_completion_feature.rs` — the real
  `core:lsp` plugin against a recording backend.
- `hume-editor/src/editor/tests/unix/buffer_words_plugin.rs` — the real
  `core:buffer-words` plugin; its last few tests load `core:lsp` too, the
  only place two real, independently-motivated `Buffer` sources rank
  together, dedup, and gate `resolve` in one session.
- `hume-scripting/src/builtins/completion/tests.rs` — the builtins' argument
  decoding.
- `hume-ui/src/popup/tests.rs` — `menu_window`/`resolve_menu`, including the
  visible-window width guarantee.

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Source registry | **One, in Rust, on `ConfigState`, holding native and Steel sources** | The minibuffer's six native sources must work with `hume --no-config`, so a Rust registry exists regardless; a second, Steel-only registry for Insert mode meant two registries and two source contracts. The frequency-cut rule is about what runs when, not where the list of sources lives — invocation is user-intent frequency either way. |
| Orchestration | **Rust, `impl EditorState`** | One driver for both targets, reachable from `EditorHostImpl` (`completion-emit!`) and key handlers alike; Steel sources are only ever queued (`queue_steel_call`), the picker's live-source precedent. |
| Token boundary | **One rule per target (buffer: the word before the cursor; minibuf: the whitespace-delimited argument), resolved by Rust before the source runs** | Deterministic, identical on every re-invocation, known while the source is pending. A `'cursor`/`'custom` per-source choice existed once; no shipping source used either, so both were removed rather than carried as unvalidated surface. |
| Async identity | **Per-invocation id; an answer applies only to its slot's latest call** | Strictly stronger than a session token plus replace-per-source: a superseded call's late answer can never land at all. |
| Snapshots | **Per invocation, never session-wide** | An `isIncomplete` re-request is computed against a later document than the first answer; each decodes its own `textEdit` ranges against its own snapshot. |
| Filter text | **Derived per slot from its live span, never set** | Removes `completion-update-filter!` and the accept-time extension it forced. |
| `:` line cycle-apply | **Restore the invoke-time input, then splice over the slot's span** | Idempotent in invoke-time coordinates; two sources with different spans coexist by construction. |
| Menu width | **`menu_window` from counts, `rows_in(range)`, `resolve_menu(&rows[window])`** | No full-list accessor exists, so width over the whole list is unwritable. |
| Source registry namespaces | **Two, `SourceRegistry::{buffer,minibuf}`, distinct `BufferSourceId`/`MinibufSourceId` — not one `Vec` plus a runtime target tag** | A name taken in one namespace was refused as "the wrong target" for the other, at runtime, only once a second real `Buffer` source (buffer-words) existed to collide with `core:lsp`. Splitting the namespace makes the collision impossible by construction instead. |
| Session slots | **Typed inside their own `Target` variant (`SourceSlot<BufferSourceId, BufferSpan>` vs. `<MinibufSourceId, MinibufSpan>`) — not one flat span enum re-checked at every read** | The old `SpanTrack` enum could disagree with which `Target` variant held it (a fallback arm, a silent `_ => continue`, an `unreachable!` in accept) even though a slot's target has never actually been able to vary independently of its own span shape. |
| `completionItem/resolve` gating | **A source's own `#:resolve #t` claim (`BufferSourceEntry::resolve`), checked in `accept`, not just "the buffer has a server with `resolveProvider`"** | A second `Buffer` source sharing an LSP-attached buffer (buffer-words) would otherwise have a resolve request sent for an item the server never produced — a real gap the first source (`core:lsp`) never surfaced, since "an item in an LSP-attached buffer" and "an LSP item" were the same fact until a second source existed. |
| Cross-source dedup | **Rank-time, priority-ordered, plain items only (`CompletionSession::recompute_dedup`)** | See Q-A1, below — the same identifier from two sources otherwise shows twice. |
| Trigger-char join for `Buffer` sources | **A source's own table (`completion-set-trigger-chars!`, `BufferSourceEntry::trigger_chars`), not `register-trigger-chars!`'s shared, listener-agnostic one** | The shared table has to accept an unknown name (it serves non-completion listeners too), so a typo on either side of the join silently disabled trigger-char completion with no error anywhere. The new table validates against the `Buffer` namespace and errors on a miss. `register-trigger-chars!`/`on-trigger-char` are unchanged for every other listener (signature help). |
| Item schema | **LSP `CompletionItem` JSON shape as lingua franca** | The store already parses it with label-fallbacks; the minimal item is `{"label": …}`; a non-LSP source omits `textEdit` and rides the token-span accept path. |
| Completion vs picker core | **Siblings sharing the matcher, not a shared session type** | See `hume-editor/src/editor/input_stack/picker/session.rs`. |

## Open questions

**Q-A1 — dedup across sources — answered, priority-ordered dedup of plain
items.** `CompletionSession::recompute_dedup` hides a *plain* item (no
`textEdit`, no `additionalTextEdits`) when a strictly-higher-priority slot's
shown answer has an item with the same `filter_text` — so the same
identifier from `core:lsp` and buffer-words shows once, as `core:lsp`'s own
item (`#:priority 10` beats buffer-words' default `0`). An item carrying
edits is never hidden this way — accepting it does something a
duplicate-*looking* plain item wouldn't, e.g. an auto-import buffer-words
could never offer. This only compares *shown* answers, and only recomputes
when the shown item set changes (an answer lands, a slot is dropped by
`observe_edit`), not every keystroke — `rank` reads the precomputed result.

Ranking otherwise still runs by score first (`CompletionSession::rank`,
score desc before priority desc): once something is typed, a `'fuzzy` LSP
candidate that scores well *always* outranks a `'string` buffer-words one
scoring a flat `0` — deliberate policy, not a gap (see `MatchKind::String`'s
own doc, `hume-editor/src/editor/completion/session.rs`): in an
LSP-attached buffer, LSP should win once the user narrows by typing, and
buffer-words earns its keep where LSP has nothing (a comment, a string
literal, a plain-text buffer with no server). An empty pattern scores
*every* haystack `0` (`hume-editor/src/editor/fuzzy.rs`'s
`empty_query_scores_every_haystack_equally`), so on a bare trigger — buffer-
words' own headline case — both sources tie at score `0` and priority
decides. `#:config (hash "match" 'fuzzy)` opts buffer-words into subsequence
scoring too, at which point it competes with LSP on score like any other
`'fuzzy` source and the priority tiebreak no longer decides (dedup still
applies regardless of `#:match` — it runs before scoring, not as part of
it).

**Q-A5 — buffer-words matching — answered, `#:match 'string`, not
prefix-at-collection.** The source emits its *whole* cached word set once
per trigger; the prefix gate (vim `i_CTRL-N` feel) happens in Rust, per
keystroke, via `MatchKind::String` (see that enum's own doc,
`hume-editor/src/editor/completion/session.rs`) — the same rule any
`'string` source gets. It carries no `#:incomplete` —
`CompletionSession::rank`'s own no-op check (`CompletionItem::is_noop_for`)
drops the exact-typed token for every source, derived fresh from the live
cursor on every keystroke, so this source never needs re-invoking just to
keep that exclusion correct as the token changes (README.md's "Matching").
`#:config (hash "match" 'fuzzy)` opts a user into subsequence scoring
instead, at the same per-keystroke, Rust-side cost as `core:lsp`'s own
matching.

**Q-A7 — kind display.** `kind: i64` is display-unused (`menu_row` puts
`label` in the main column and `detail` right-aligned in a trailing one).
*Default: a static Rust map (LSP kind numbers as the universal enum), a
single-char column, no per-kind theming — a separate task from the
two-column layout `resolve_menu` already gives every menu.*

**Q-B6** (unifying completion's matcher with the picker's) shipped: both
route through `hume-editor/src/editor/fuzzy.rs`'s `FuzzyMatcher`,
distinguished by `FuzzyProfile`.
