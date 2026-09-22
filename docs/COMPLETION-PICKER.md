# HUME — Completion Sources

Design record for the completion system: one model in which a *source*
(native Rust, or Steel-registered — `core:lsp` is one) is registered once
with its own token rule, invoked by one orchestrator for either target
(Insert mode, or the `:` command line), and answers a specific *invocation*
that carries its own document snapshot and span. Several sources rank
together in one menu, each against its own token.

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
| **Source** — a named producer of candidates, with its static facts: which target it serves, where its token starts, how its items score, its priority, and its body (a native fn or a Steel proc) | `SourceEntry` in `SourceRegistry` | `registry.rs`; the registry lives on `ConfigState.completion_sources`, so `:reload-config` rebuilds it from the natives by construction |
| **Invocation** — one call of one source for one trigger: the id the source answers to, the document snapshot it saw (`rope` + `head` + every edit observed since, composed), its token span in live coordinates, and its answer once it has one | `Invocation` | `session.rs` |
| **Session** — the one open session: one `SourceSlot` per participating source (its latest `shown` invocation and, if re-invoked since, the newer `inflight` one), the ranked `(slot, item)` index, the matcher | `CompletionSession` | `session.rs`; `session/accept.rs` applies the accepted item |
| **Target** — where an accepted item lands, and what the target alone knows (`Buffer`: bid, pane, generation; `Minibuf`: the `:` input every source saw) | `Target` | `session.rs` |
| **Orchestrator** — the one driver for both targets: picks the sources a trigger applies to, mints invocations, runs them, lands answers, reacts to edits, applies the `:` line's eager policy | `impl EditorState` | `orchestrate.rs` |
| **Layer** — keys, selection, render sync | `CompletionLayer` | `input_stack/completion.rs` |

### Sources

```rust
enum SourceBody {
    NativeUniverse(fn(&CompletionCtx) -> Vec<CompletionItem>),          // ⇒ Minibuf + MinibufToken::Arg
    NativeDelegated(fn(&str, usize, &CompletionCtx) -> (Range<usize>, Vec<CompletionItem>)),  // ⇒ Minibuf + Custom
    Steel { proc: SteelVal, target: SourceTarget },
}
enum SourceTarget { Buffer(BufferToken), Minibuf(MinibufToken) }
enum BufferToken  { Word, Cursor, Custom }
enum MinibufToken { Arg, Custom }
```

A native source's signature *is* its contract (`SourceEntry::target` is
derived from the body); a Steel source declares `#:target`/`#:token`, decoded
as one value at the builtin so a `'buffer` source with an `'arg` token is a
Steel argument error, never a state the editor has to reject later. No native
`Buffer`-target shape exists — the buffer-words source
(`runtime/plugins/core/buffer-words/`) is Steel, deliberately: it dogfoods
this plugin-facing API rather than the Rust-internal machinery the six native
minibuffer sources already validate.

The six native minibuffer sources (`command`, `buffer-name`, `theme` —
`NativeUniverse`; `path`, `path-dirs-only`, `set` — `NativeDelegated`) are
compiled in. `TypedCommand.completer` (`Option<Cow<'static, str>>`) names any
entry by name — a built-in's `&'static str` constant, or the runtime string
`define-typed-command! … #:complete "name"` hands over.

**The token rule is the source's, chosen once at registration.** For a
`Buffer` source the editor resolves it *before* the source runs, against the
invocation's own snapshot: `Word` is `hume_ops::edit::word_start_before(text,
head, word_chars)..head` (the LSP source's choice — the seeded filter is that
text, handed to the proc as its `prefix` argument); `Cursor` is `head..head`
(no seeding, and accept replaces nothing before the cursor); `Custom` leaves
it to the answer's `#:span`. For a `Minibuf` source, `Arg` is the
whitespace-delimited argument the cursor is in (`arg_prefix`/`token_end_at`,
the framework's own command-line grammar) and `Custom` is the answer's own
span (`:e`'s path, `:set`'s phase-dependent token). Nothing in the framework
guesses a boundary on a source's behalf, and nothing forces one source's
boundary on another.

### Invocations and answers

A trigger mints one `Invocation` per source (`widget_token::next()` for its
id). A native source answers inline; a Steel one is *queued* via
`EditorState::queue_steel_call` — `(proc id bid prefix)` for a buffer source,
`(proc id input cursor)` for a `:`-line one — and answers with
`(completion-emit! id items #:incomplete #:span)`, sync or from any later
callback, exactly once. An empty list is "nothing from this source".

**An answer applies only to the latest call of its slot.** Re-invoking a
source (a later keystroke while its last answer was `isIncomplete`, a second
Ctrl-Space, a trigger char) supersedes the earlier call: its id goes stale,
and an answer carrying it is dropped (`completion-emit!` returns `#f`). A
repeated answer for a still-latest id replaces the earlier one, so a source
may stream. This is the entire stale-async story — there is no session
token, no `async_opener_stale` gate, and no way for a slow LSP response to
overwrite a newer one. The old answer stays ranked (against the new token
text) until the new one lands, so the menu never blinks empty.

`#:span` (required for, and only honoured by, a `Custom` token) is in the
*invocation's own* coordinates — the snapshot a buffer source was handed,
the `input` a `:`-line source was — never live ones: the source computes it
from what it was given, and the session maps it forward through whatever was
typed since. Validated where it enters (`DocSnapshot::resolve_custom_span`):
in range, grapheme-snapped, containing the cursor as it stood then, on one
line — a completion token never spans a line, the one bound left on an
otherwise source-chosen value, since it doubles as accept's replacement span.
A streaming source's later answer re-resolves and replaces the earlier span,
same as its items — nothing about a `Custom` token's *first* answer pins the
ones that follow.

### Edits

`Editor::apply_insert_edit` — the chokepoint every Insert-mode keystroke that
lands an edit goes through — calls `EditorState::completion_observe_edit`,
which:

1. composes the `ChangeSet` into *every* invocation's `cs_since` (shown and
   in-flight alike) and remaps each live span — `start` with `Assoc::Before`
   (text inserted exactly at the token's start belongs to the token), `end`
   with `Assoc::After` (text typed at its end extends it);
2. drops a slot's answer if the cursor left its token (`head ∉ [start,
   end]`) or the character *before* the token was deleted — detected by
   `start` and `start - 1` mapping to the same live position, which a
   deletion elsewhere (a second cursor's, say) cannot cause. Deleting the
   token's own first char stays inside it; a slot narrowed to zero *matches*
   is still live (Backspace brings its items back);
3. re-ranks (`CompletionSession::rank`: score desc, source priority desc,
   sortText asc, index asc — each shown item scored against *its own* slot's
   token text with its source's `MatchKind`), resets the menu selection;
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
the item lacked `additionalTextEdits` and the server offers it.

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
| `(register-completion-source! name proc #:target #:token #:match ['fuzzy] #:priority [0])` | config-time; crosses as `Effect::RegisterCompletionSource`, so a failed activation's registration is never applied (the `Effect::BindKey` rationale); re-registering a name replaces it, natives included, *as long as the new source serves the same target* — a name taken by the other target is refused with a loud error, not silently clobbered |
| `(completion-emit! id items #:incomplete [#f] #:span [#f])` | the one way items enter a session; `#f` once `id` is stale |
| `(completion-top n)`, `(completion-accept! idx)`, `(completion-dismiss!)` | unchanged |
| `(register-trigger-chars! source language chars)` + `on-trigger-char` | unchanged, shared with signature help; the editor additionally invokes the completion source registered under `source`'s name, by name |
| `(define-typed-command! … #:complete "name")` | a `:` command's argument completer |
| command `completion-trigger` (Insert, default `Ctrl-Space`) | native; `(call! "completion-trigger")` from Steel |

`core:lsp/completion.scm` is the reference source: `register-completion-
source! "lsp"` with `#:token 'word #:priority 10`, whose proc sends
`textDocument/completion` (`#:supersede "completion"`) and answers with the
decoded list and its `isIncomplete` flag, declining with an empty answer when
the buffer's server has no `completionProvider`; `lsp/setup-trigger-chars!`
registers the server's trigger characters under the same `"lsp"` name.

## Tests

- `hume-editor/src/editor/completion/session/tests.rs` — the store alone:
  per-slot tokens, the crossing rule, stale answers, custom-span validation.
- `hume-editor/src/editor/tests/completion/` — through the real seams, one
  file per concern: `sources.rs` (registration, invocation, token rules,
  stale answers, re-invocation, several sources), `menu_keys.rs`,
  `accept.rs`, `minibuf.rs`, `render.rs`.
- `hume-editor/src/editor/tests/unix/lsp_completion_feature.rs` — the real
  `core:lsp` plugin against a recording backend.
- `hume-scripting/src/builtins/completion/tests.rs` — the builtins' argument
  decoding.
- `hume-ui/src/popup/tests.rs` — `menu_window`/`resolve_menu`, including the
  visible-window width guarantee.

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Source registry | **One, in Rust, on `ConfigState`, holding native and Steel sources** | The minibuffer's six native sources must work with `hume --no-config`, so a Rust registry exists regardless; a second, Steel-only registry for Insert mode meant two registries and two source contracts. The frequency-cut rule is about what runs when, not where the list of sources lives — invocation is user-intent frequency either way. |
| Orchestration | **Rust, `impl EditorState`** | One driver for both targets, reachable from `EditorHostImpl` (`completion-emit!`) and key handlers alike; Steel sources are only ever queued (`queue_steel_call`), the picker's live-source precedent. |
| Token boundary | **Per source, chosen at registration (`BufferToken`/`MinibufToken`), resolved by Rust before a buffer source runs** | Deterministic, identical on every re-invocation, known while the source is pending; no source is forced onto another's boundary. |
| Async identity | **Per-invocation id; an answer applies only to its slot's latest call** | Strictly stronger than a session token plus replace-per-source: a superseded call's late answer can never land at all. |
| Snapshots | **Per invocation, never session-wide** | An `isIncomplete` re-request is computed against a later document than the first answer; each decodes its own `textEdit` ranges against its own snapshot. |
| Filter text | **Derived per slot from its live span, never set** | Removes `completion-update-filter!` and the accept-time extension it forced. |
| `:` line cycle-apply | **Restore the invoke-time input, then splice over the slot's span** | Idempotent in invoke-time coordinates; two sources with different spans coexist by construction. |
| Menu width | **`menu_window` from counts, `rows_in(range)`, `resolve_menu(&rows[window])`** | No full-list accessor exists, so width over the whole list is unwritable. |
| Item schema | **LSP `CompletionItem` JSON shape as lingua franca** | The store already parses it with label-fallbacks; the minimal item is `{"label": …}`; a non-LSP source omits `textEdit` and rides the token-span accept path. |
| Completion vs picker core | **Siblings sharing the matcher, not a shared session type** | See `hume-editor/src/editor/input_stack/picker/session.rs`. |

## Open questions

**Q-A1 — dedup across sources — answered, no dedup.** Buffer-words echoes
identifiers LSP also returns; there is no dedup across sources. In practice
this falls out for free rather than needing `CompletionSession::rank` logic:
buffer-words registers `#:match 'string`, which scores every match `0`,
while `core:lsp`'s `#:match 'fuzzy` scores positively — so a tied-*priority*
tiebreak was never needed, the *score* ordering alone puts every LSP item
ahead of every buffer-words item.

**Q-A5 — buffer-words matching — answered, `#:match 'string`, not
prefix-at-collection.** The source emits its *whole* cached word set on
every trigger; the prefix gate (vim `i_CTRL-N` feel) happens in Rust, per
keystroke, via `MatchKind::String`, same as this doc's `MatchKind` table
already describes for any `'string` source. Filtering by prefix in Steel at
collection time — the shape this question originally proposed — turns out
to be wrong, not just less convenient: the source has no `#:incomplete`, so
it answers once per trigger, and a set narrowed to the trigger-time prefix
could never widen back out on Backspace. `#:config (hash "match" 'fuzzy)`
opts a user into subsequence scoring instead, at the same per-keystroke,
Rust-side cost as `core:lsp`'s own matching.

**Q-A7 — kind display.** `kind: i64` is display-unused (`menu_row` puts
`label` in the main column and `detail` right-aligned in a trailing one).
*Default: a static Rust map (LSP kind numbers as the universal enum), a
single-char column, no per-kind theming — a separate task from the
two-column layout `resolve_menu` already gives every menu.*

**Q-B6** (unifying completion's matcher with the picker's) shipped: both
route through `hume-editor/src/editor/fuzzy.rs`'s `FuzzyMatcher`,
distinguished by `FuzzyProfile`.
