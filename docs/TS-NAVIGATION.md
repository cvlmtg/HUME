# HUME — Structural Text Objects & Navigation

Design reference for tree-sitter-backed structural text objects (`m i f`, `m a c`, …) and
navigation (`goto-next-<kind>` / `goto-prev-<kind>`). Shipped feature — this file records the
architecture and the *why* behind it, not a build log.

| Topic | Where |
|---|---|
| Query vocabulary, span collection, freshness | `hume-treesitter/src/textobjects.rs`, `syntax.rs` |
| Selection policy (Move/Extend, count, multi-cursor) | `hume-ops/src/text_object/argument.rs`, `motion/object.rs` |
| Command dispatch, registration, keymap | `hume-editor/src/editor/commands/structural.rs`, `registry/defaults/structural.rs`, `keymap/defaults.rs` |
| `register-grammar!` Scheme API | `runtime/scheme/prelude.scm` |
| End-user docs | `user-manual/docs/selections.md`, `default-keys.md`, `builtin-commands.md`, `moving-around.md`, `syntax-highlighting.md` |

## Scope and principles

- Object kinds: `function`, `class`, `parameter`, `comment`, `test`, `entry` — every kind Helix's
  query files define except `xml-element` (see *Future work*). Adding a kind is one enum variant,
  one table row, one manual row. `entry`'s user-facing text-object/navigation commands are named
  `value`, not `entry` — see `registry/defaults/structural.rs`'s entry row for why that split
  exists (the tree-sitter capture name isn't HUME's to rename; the command name is).
- Crate boundaries are compiler-enforced: `hume-editor` production code names no `tree_sitter::*`
  type (`tree-sitter` is a dev-dependency there); `hume-ops` depends on neither `tree-sitter` nor
  `hume-treesitter`. Consequently:
  - **query execution** (tree, layers, byte offsets) lives in `hume-treesitter`, whose public
    surface takes `&BufferText` and returns inclusive char-offset spans;
  - **selection policy** (Move/Extend, count, multi-cursor, separator rule) lives in `hume-ops`;
  - **wiring** (registry, dispatch funnel, keymap) lives in `hume-editor`.
- Fail-soft exactly where the lexical objects already were: no grammar, no `textobjects.scm`, no
  match → the selection is left unchanged, silently. Fail-loud everywhere else: a broken
  `textobjects.scm` fails the whole grammar attach (the policy `injections.scm` already has); a
  span that does not fit the text is a bug, not a case to paper over.
- Helix's query files are consumed as written. HUME does not second-guess what a capture spans
  (`function.inside` is the body *including* its braces, because that is the node the query
  captures); HUME only decides which captured object a cursor means and how a selection is built
  from it. The one exception is contiguity (see the hull rule below): a query can be under-anchored
  in a way that makes tree-sitter report a match spanning unrelated content no Helix pattern
  intended as one object, and rejecting that isn't second-guessing what a capture spans — it's
  refusing a match whose captures were never contiguous to begin with.

## Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Navigation's selected object | Whole object, cursor (head) at its **start** | The viewport lands on the object's signature and a following `w` walks into its body. |
| Extend after a Move result | Union with the running selection, not a plain anchor-keep | A Move result's anchor sits at the object's *end* (reversed, for the head-at-start rule above); a plain "keep the anchor, move the head" replacement — the convention elsewhere in `hume-ops` — would drop everything between the object's near edge and a newly found span, or shrink the selection when the found span is nested inside what's already selected. Taking the min/max of the current selection and the found span with every step fixes both, self-correcting the same way `apply_word_select_extend`'s own union-based growth does. |
| Navigation's default keys | `g <key>`/`g <KEY>` (lowercase next, uppercase previous), reusing each kind's `m i`/`m a` key | Slots into the `g` goto prefix — a structural jump is a goto like any other. Two kinds' letters diverge from Helix's own (`test` `T`→`u`, `entry` `e`→`v`) because deriving "previous" by uppercasing requires every key lowercase and no two uppercased forms to collide; `T` had no lowercase form, and `e` collides with `goto-last-line`. See `keymap/defaults::build_goto_trie` and `STRUCTURAL_OBJECTS`'s doc comment. |
| `m i a` / `m a a` | One command family, structure-aware | `inner-argument`/`around-argument` use the grammar's `parameter` object when the buffer has one, the lexical scan otherwise, and HUME's own separator rule for "around" in both cases. There is no separate `parameter` object family. |
| `register-grammar!` call syntax | Pure positional, no keyword arguments | A `#:kw`-sugared form was tried and reverted: Steel 0.8.2 miscompiles a keyword-arg call nested inside another (`define-command!`), and a keyword-free rest-arg workaround has its own, independent miscompile trigger. Positional-only sidesteps both — see `docs/LESSONS.md` L12. |

## Query layer (`hume-treesitter`)

`SyntaxLayer` (`layers.rs`) carries its grammar bundle (`Arc<GrammarBundle>`), not just a
highlighter — every per-language query a layer may need (a `textobjects.scm`, a future
`locals.scm`) comes for free, and an injected layer's own query is otherwise unreachable.

`textobjects.rs` defines the query vocabulary:
- `ObjectKind { Function, Class, Parameter, Comment, Test, Entry }` and `ObjectSpan { Inside,
  Around, Movement }` — both emitted, along with their `ALL` array and `capture_name`/
  `from_capture_name` pair, from one macro-driven variant↦name list per type, so a variant added to
  either enum can't drift out of sync with the array `TextObjectsQuery`'s capture table is sized
  and indexed by.
- `TextObjectsQuery` resolves a compiled query's capture names once, at attach time, into a dense
  `(kind, span) → capture index` table by splitting each name on its last `.`
  (`@parameter.inside`); names that don't parse (`@_helper`, `@function.x`) map to nothing.
- A structural object is the **hull** (min start ‥ max end) of every node one match captured
  under a `<kind>.<span>` name — Helix's query files put one capture name on several nodes of a
  single match (a function's leading attributes, an argument plus its trailing comma), so
  collection always hulls a match's captured nodes rather than taking one node at a time —
  **provided those nodes are contiguous**: each one is either overlapping/nested with what's
  hulled so far, or is literally the previous node's `next_sibling()` (`hume-treesitter`'s
  `capture_hull`). An *unanchored* quantifier in a query (no `.` between two of its repetitions,
  or between its last repetition and what follows) lets tree-sitter match across unrelated
  intervening siblings — the rust `test.around` pattern's `[(attribute_item)|(line_comment)]*`
  group has exactly this shape, and on a file with two `#[test] fn`s also reports a match whose
  captured nodes span from the first test's attribute to the *second* test's `fn`, skipping the
  first test's whole body in between. A match failing the contiguity check is dropped whole, never
  trimmed to its contiguous prefix — trimming would still fabricate an object out of content the
  query never grouped (a later, unrelated function tagged as a test because it happened to follow
  one). Confirmed present on Helix's own `runtime/queries/rust/textobjects.scm` at the pin this
  repo fetches (`f6f3eb1fe4a7`) and unfixable from here — PLUM downloads queries from that pin with
  no local-override mechanism — so the check lives at the consumption layer instead.
- `ObjectSpans` is the owned, sorted, deduplicated list of spans collection produces — owned
  rather than an iterator, since `hume-editor` needs disjoint mutable borrows to apply the
  resulting selection, and N cursors × `count` steps then probe a vector instead of re-running the
  query per step. `enclosing(pos)` returns the smallest span containing `pos`; `adjacent(pos, dir)`
  returns the nearest span start-keyed in *both* directions (a backward press from inside an
  object lands on that object's own start first, matching Vim's `[m`, rather than never being able
  to reach the object currently enclosing the cursor).
- Navigation priority is `Movement → Around → Inside` per kind, reordered (not truncated) for
  `Parameter`: `Inside → Movement → Around`, since Helix's `parameter.around` hull includes the
  trailing comma — a wart `m i a`/`m a a` reject for selection — while `parameter.inside` is
  exactly the span `m i a` selects. The `Movement`/`Around` fallbacks stay reachable: a grammar
  defining only `@parameter.around` still gets a navigable span, rather than a silent no-op.

## Freshness

An edit only records an `InputEdit`; baking and the asynchronous reparse normally happen once a
frame, in `Editor::settle`. A structural command reads the tree after that has already run for the
frame, and a macro or dot-repeat batch replays several edits with no settle in between — either
way the committed tree can be a generation behind by the time a query needs it.

`Syntax::ensure_current` closes that window synchronously, at the query site: bake pending edits,
build the next request (incremental when the baked chain is intact, a full parse otherwise), run
it inline, install the result. Its own freshness gate requires `parsed_gen == Some(text_gen)`
*and* `tree_gen == text_gen`: `parsed_gen` alone isn't enough, since a `ParseFailed` install
advances it while leaving `layers`/`tree_gen` untouched, which would otherwise report "current"
over a tree that predates a broken edit chain. `install`'s own "already installed" guard mirrors
this concern from the other side — it must let a later successful result for a generation that
already failed once actually land (whether a retried `ensure_current` call or a slow async result
that finally arrives), so it's keyed on `parsed_gen`, `tree_gen`, *and* `layers.is_some()`
together: no single field alone distinguishes "already installed" from every other state, since
`bake` also advances `tree_gen` on the ordinary path, before `install`'s own successful arm ever
runs.

`hume-editor` wraps this in `syntax/parse.rs::ensure_syntax_current`, which no-ops instead
of running the synchronous parse in three cases where it can't help: no committed tree yet (before
the worker's first parse lands, `object_spans` already returns nothing regardless — a synchronous
full parse here would just duplicate work the worker is already doing); the buffer is over
`syntax-highlight-max-bytes` (closes a one-frame window `Editor::reparse_stale_buffers`'s own cap
check doesn't cover); and no layer defines a textobjects query at all (a grammar with no
`textobjects.scm` can never make `object_spans` return anything, so reparsing to answer it is pure
waste, worst on a `.`-repeat or macro batch paying it once per step).

## Selection policy (`hume-ops`)

`apply_text_object_by_mode` is the shared Move/Extend/count policy for a text object that resolves
to a whole span (replace on Move, union with a past-end outward retry on Extend). Structural
navigation needs a second policy, since a `goto-next-<kind>` step maps a search origin to a whole
object span rather than a single new head position: `motion::apply_object_motion`.

Move mode's origin is the current selection's far edge in the direction of travel — the object
*just* selected, not its own start — which is what makes a second forward press skip an object
nested inside the one just selected rather than re-finding it. The result anchors at the object's
end and heads at its start in both directions, so the viewport lands on the signature and a
following `w` walks into the body.

Extend mode's origin is always the current head, matching the convention `apply_motion` and
`apply_word_select_extend` use elsewhere — never the far edge Move reads, since a Move result's
head sits at the object's start and searching from the anchor there would skip everything between
them. The result is the *union* of the running selection with the found span (see the Decisions
table above for why a plain anchor-keep isn't enough).

`text_object/argument.rs::around_from_inner` derives an argument's "around" span from its "inside"
span by locating its separator comma — HUME's own rule, independent of whether the inside span
came from the lexical scan or a tree-sitter `parameter.inside` capture, so `m i a`/`m a a` stay one
structure-aware family rather than two separate objects.

## Commands and dispatch (`hume-editor`)

`registry/command.rs::SelectionBody::Structural(StructuralBody)` is the registry's hook for a body
needing more context than a plain function pointer carries. `StructuralBody` has three shapes:
`Select { kind, span }` (`m i <k>` / `m a <k>`), `Goto { kind, dir }` (`goto-next-<k>` /
`goto-prev-<k>`), and `Argument { around: bool }` (`m i a` / `m a a`, the tree-sitter `parameter`
object with the lexical scan as fallback). One arm in the dispatch funnel
(`commands/pipeline.rs::run_native_body`) interprets all three: `ensure_syntax_current`, collect
`ObjectSpans` for the body's kind, apply the body against them.

`registry/defaults/structural.rs::STRUCTURAL_OBJECTS` is the single table driving both
registration (24 commands: four per kind × six kinds) and the keymap
(`keymap/defaults::build_text_object_trie` reads the same table for `m i`/`m a`'s third-level
keys) — a kind added to the table needs no change anywhere else. Each row carries its own static
doc strings per command rather than a shared noun template, since "including its delimiters" is
wrong for an argument (a separator comma, never brackets) and for a function (its signature, not
delimiters).

## Rejected alternatives

Recorded here so they are not re-proposed:

- *Twelve macro-generated `EditorCmd` functions.* Loses `Motion`/`Selection` metadata, needs a new
  builder method, and duplicates a data table as code.
- *Root layer only (no per-layer bundle).* Wrong inside markdown fences and inside injected
  comments; fixed structurally by storing the bundle on every layer.
- *Refuse structural commands when the tree is stale.* Silent failure after every edit until the
  worker catches up.
- *Bake only, no reparse at the query site.* Positions become valid, structure stays stale — wrong
  spans silently.
- *Move-to-start navigation without selecting the whole object.* Composes worse (`]f` then `m a f`
  then `d`) and diverges from `w`/`b`, which already land on a selected unit.
- *A separate `m i p` / `m a p` family for parameters.* Two objects for one concept, and the
  tree-sitter one would ship Helix's raw `parameter.around` wart (deleting the last parameter
  leaves `, `).
- *Cursor `set_byte_range` as a hull-collection optimization.* Truncates grouped hulls (the
  trailing comma, the leading attributes) instead of merely skipping unrelated matches.
- *`parameter.around` as the argument navigation target.* Same wart: `goto-next-argument` would
  select the trailing comma that `m a a` deliberately does not.
- *Flipping `adjacent`'s largest-end tie-break to smallest-end, to fix the `test.around`
  under-anchored-quantifier bug (`gu`/`gU` selecting through the end of the file).* Only masks the
  symptom for `adjacent`'s particular tie order — the spurious merged match is still collected and
  still wins `enclosing` (`m a u`) from any position inside it. The defect is in what gets
  collected, not in which collected span a lookup prefers.
- *Same-end dedup on the collected spans, for the same bug.* Would break
  `hume-treesitter/src/textobjects/tests.rs`'s `comment_around_on_the_last_line_of_a_block_is_the_whole_block`
  — legitimate same-end nesting in delimiter-less languages relies on more than one span sharing an
  end. See that test's own HARD STOP comment.
- *Trimming a non-contiguous match to its trailing contiguous run, instead of dropping it whole.*
  Still fabricates an object: the bogus `test.around` match's trailing run is `[a later, unrelated
  attribute, that attribute's own fn]` — keeping it tags a function with no `#[test]` of its own as
  a test.
- *A text/whitespace scan of the gap between two captured nodes, instead of a tree-native
  `next_sibling()` check.* Disproven directly: a comment between an attribute and its function is a
  grammar `extra` (`tests/fixtures/grammars/rust/src/grammar.json`), which might suggest tree-sitter
  treats it as transparent — but a raw match dump shows the opposite: `function.around`'s own `.`
  anchor does *not* skip it, and already drops the attribute across a comment gap today, before any
  of this fix's code runs. A whitespace scan would have solved a problem the query's own anchor
  semantics don't actually have.
- *An `is_extra()`-skipping loop in the contiguity check* (tolerate a run of comment/whitespace
  nodes between two captured nodes, not just a direct `next_sibling()` hop). Motivated by the same
  wrong "extras are transparent" assumption above; dropped once it was disproven, since it would
  have been more permissive than tree-sitter's own anchors and untestable against any real query in
  this repo — every case where two captured nodes legitimately have a comment between them (the
  `test.around` group's own `(line_comment)` alternative) already captures that comment directly as
  an ordinary adjacent sibling, so the loop's extra generality was never exercised. A single
  `next_sibling()` hop is both simpler and the only version anything here actually verifies.
- *Preferring a lexical inner span nested inside a tree parameter span.* A second rule so `2` stays
  selectable inside `foo([1, 2, 3])`; `m i v` already covers members, and one rule is easier to
  predict.
- *Keyword arguments (`#:injections`/`#:textobjects`) for `register-grammar!`.* Tried, reverted —
  see the Decisions table above and `docs/LESSONS.md` L12.

## Future work

- `xml-element` object kind (Helix defines it; HUME's lexical tag matcher is the current answer).
- `locals.scm` (scope-aware rename) — stays a separate roadmap item.
