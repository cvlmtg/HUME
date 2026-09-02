# SPEC — Tree-sitter structural text objects & navigation

## Context

`docs/ROADMAP.md` carries two open items:
- Tree-sitter text objects — `textobjects.scm` / `locals.scm` (structural select).
- Tree-sitter structural navigation — jump to next/prev function/argument/class/etc.

HUME's existing text objects (`mi(`, `mi"`, `mia`, …) are lexical heuristics in `hume-ops`.
`hume-treesitter` already parses every open buffer (root grammar plus injected layers) for
highlighting. Helix's `textobjects.scm` (one `@<kind>.<span>` capture per object) is the query format;
`main` carries no textobjects plumbing yet — nothing reads, compiles, fetches, or threads such a file.
This spec designs the plumbing and the two capabilities on top of it, end to end. `locals.scm`
(scope-aware rename) stays a separate roadmap item.

## Decisions (locked)

1. **Navigation selects the whole object.** `goto-next-function` makes the next function the
   selection, cursor (head) at its **start**; Extend mode keeps the anchor and grows over the object.
2. **Navigation commands ship unbound.** Every required default key is listed under *Follow-ups*;
   binding them needs the `[`/`]` prefix, which the kill-ring cycle occupies today.
3. **`m i a` / `m a a` are one command family, structure-aware.** `inner-argument` /
   `around-argument` use the grammar's `parameter` object when the buffer has one, the lexical scan
   otherwise, and HUME's own separator rule for "around" in both cases. There is no separate
   `parameter` object family.
4. **`register-grammar!` takes keyword arguments** `#:injections` / `#:textobjects` (breaking
   change to the documented positional form).

## Design

### 1. Scope and principles

- Object kinds: `function`, `class`, `parameter`, `comment`, `test`, `entry` — every kind Helix's
  query files define except `xml-element` (follow-up). Adding a kind later is one enum variant, one
  table row, one manual row.
- Crate boundaries, compiler-enforced: `hume-editor` production code names no `tree_sitter::*`
  type (`tree-sitter` is a dev-dependency there); `hume-ops` depends on neither `tree-sitter` nor
  `hume-treesitter`. Consequently:
  - **query execution** (tree, layers, byte offsets) lives in `hume-treesitter`, whose public
    surface takes `&BufferText` and returns inclusive char-offset spans;
  - **selection policy** (Move/Extend, count, multi-cursor, separator rule) lives in `hume-ops`;
  - **wiring** (registry, dispatch funnel, keymap) lives in `hume-editor`.
- Fail-soft exactly where the lexical objects already are: no grammar, no `textobjects.scm`, no
  match → the selection is left unchanged, silently. Fail-loud everywhere else: a broken
  `textobjects.scm` fails the whole grammar attach (the policy `injections.scm` already has); a span
  that does not fit the text is a bug, not a case to paper over.
- Helix's query files are consumed as written. HUME does not second-guess what a capture spans
  (`function.inside` is the body *including* its braces, because that is the node the query
  captures); HUME only decides which captured object a cursor means and how a selection is built
  from it.

### 2. Query layer — `hume-treesitter`

#### Layers carry their grammar bundle

`SyntaxLayer` (`layers.rs`) becomes
`{ tree, bundle: Arc<GrammarBundle>, ranges: Vec<tree_sitter::Range>, depth: u8 }`. The `bundle`
field replaces `highlighter` (reached as `layer.bundle.highlighter`). Every layer's bundle is
already in hand at install time (`ParseDone.bundle` for the root, `ParsedInjection.bundle` for
injections); storing only the highlighter made an injected layer's `textobjects` query unreachable.
Record on the field why the bundle, not one query, is stored: any per-language query the layer may
need later (`locals.scm`) comes for free.

#### Vocabulary (`textobjects.rs`)

- `ObjectKind { Function, Class, Parameter, Comment, Test, Entry }` — `Copy`, `ALL` (const array of
  every variant), `capture_name(self) -> &'static str` (the `<kind>` half of a capture name, single
  source of truth).
- `ObjectSpan { Inside, Around, Movement }` — `Copy`, `ALL`; `Movement` is Helix's optional
  navigation-only capture, consumed by navigation only (below).
- `Direction { Forward, Backward }` — the only direction enum for this feature; `hume-ops` takes a
  `backward: bool` (the `apply_word_select` convention) rather than importing it.
- `TextObjectsQuery { query: tree_sitter::Query, captures: [[Option<u32>; ObjectSpan::ALL.len()];
  ObjectKind::ALL.len()] }` — capture index per `(kind, span)`, indexed `[kind as usize][span as
  usize]` and sized from the `ALL` arrays (no count literal to keep in step), filled once at
  construction by splitting each capture name on its last `.` (`rsplit_once`); names that do not
  parse (`@_helper`, `@function.x`) map to nothing. `defines(kind, span) -> bool`.

#### Spans are hulls of grouped captures

Helix's query files put one capture name on several nodes of a single match:
`((attribute_item)* @function.around . (function_item …) @function.around)`,
`((_) @parameter.inside . ","? @parameter.around) @parameter.around`,
`(line_comment)+ @comment.around`, the whole `test` pattern. One object is therefore **the hull
(min start ‥ max end) of every node a match captured under that name** — never one node at a time.
Implementation: `cursor.matches()` (one visit per match, unlike `captures()`), fed by
`highlight::RopeProvider` as `injections.rs` already does — text predicates (`#eq?` on the `test`
pattern's attribute name, on the `vec!` entry case) are evaluated only when the cursor has a text
provider; without one those patterns silently never match. Per match
`m.nodes_for_capture_index(idx)` for the one `idx` mapped to `(kind, span)`, hull over those nodes;
a match without that capture contributes nothing; a zero-width hull (MISSING nodes) is dropped.
Byte→char via `BufferText::byte_to_char`; the exclusive end becomes HUME's inclusive end via
`prev_grapheme_boundary`, never `- 1`. The query runs over the whole tree, with a fresh
`QueryCursor` per run (one run per keystroke, not per frame). `set_byte_range` is rejected and the
rejection recorded on the hull-building function: the cursor prunes children outside its range, which
truncates a grouped hull (the trailing comma, the leading attributes) instead of merely skipping
unrelated matches.

Every span-producing path asserts `end_byte <= text.len_bytes()` (`debug_assert`); freshness
(§3) makes a stale node impossible by construction, so a violation is a bug and must stay loud.

#### `ObjectSpans` — one owned, sorted list across all layers

```rust
pub struct ObjectSpans { spans: Vec<(usize, usize)> }   // inclusive char spans, sorted by (start, Reverse(end)), deduplicated

impl ObjectSpans {
    pub fn collect(layers: &SyntaxLayers, text: &BufferText, kind: ObjectKind, span: ObjectSpan) -> Self;
    pub fn collect_for_navigation(layers: &SyntaxLayers, text: &BufferText, kind: ObjectKind) -> Self;
    pub fn enclosing(&self, pos: usize) -> Option<(usize, usize)>;
    pub fn adjacent(&self, pos: usize, dir: Direction) -> Option<(usize, usize)>;
}
```

- `collect` runs the query on **every** layer whose bundle has a `textobjects` query and merges the
  results into one list. There is no innermost-layer walk and no "does this layer cover the cursor"
  test: an injected layer's nodes always lie inside the parent node that hosts the injection, so
  smallest-containing and nearest-start already prefer the innermost object, and a layer without a
  query (Rust's `comment` injection, markdown prose) contributes nothing — the outward fallback is a
  consequence, not a mechanism. Record this on `collect`.
- `collect_for_navigation`: per layer, the first of `Movement`, `Around`, `Inside` that the layer's
  query **defines** (Helix's rule: `.movement` exists precisely for the languages where `.around`
  is a poor navigation target). One exception, recorded on the function: `Parameter` navigates
  `Inside` — Helix's `parameter.around` hull is the argument plus its trailing comma (the wart §8
  rejects for selection), and the trimmed argument is what `m i a` selects, so `goto-next-argument`
  lands on the same span.
- `enclosing(pos)`: the smallest span containing `pos` (`start <= pos <= end`).
- `adjacent(pos, Forward)`: the span with the smallest `start > pos`, ties → largest `end`
  (outermost). `adjacent(pos, Backward)`: the largest `start < pos`, ties → largest `end`.
  **Start-keyed in both directions**, so a backward press from inside an object lands on that
  object's own start first (Vim `[m`), then walks further back. Helix keys backward on `end < pos`,
  which can never reach the enclosing object; record the divergence and its reason on `adjacent`.
- The list is owned: `hume-editor` needs `&state.buffers` and `&mut state.panes.state` at once when
  it applies the selection, so the tree borrow must end before that; and N cursors × `count` steps
  probe a vector instead of re-running the query.

Grammar-free unit tests cover the two methods on synthetic spans; the fixture tests cover the hull
and layer behaviour. One expectation is tree-sitter's, not HUME's, and is pinned by test: the query
engine discards a quantified pattern's sub-match whose captures are a subset of a longer match in
progress, so `m a f` on a function with attributes includes the attributes and `m a c` on the last
line of a comment block selects the whole block. **If the fixture contradicts this, stop and report
(HARD STOP).** Do not patch it with a same-end dedup: that breaks legitimate same-end nesting in
delimiter-less languages (a nested `def` ending where its parent ends).

### 3. Freshness — the tree matches the text when a command runs

Today an edit only records an `InputEdit` (`Syntax::record_edit`); bake and the asynchronous reparse
happen in `Editor::settle`. A command that reads the tree runs after render, and macro replay runs a
whole batch with no settle between steps, so a structural object after an edit would read pre-edit
byte ranges — wrong spans at best, an out-of-range `Rope::byte_to_char` panic at worst.

`Syntax` gains one entry point:

```rust
pub fn ensure_current(&mut self, bid: BufferId, text_gen: u64, text: &BufferText,
                      langs: &Arc<FxHashMap<String, Arc<GrammarBundle>>>) -> Option<ChainBreak>
```

- `parsed_gen == text_gen` → nothing to do, `None`.
- Otherwise: `bake(text_gen)` (existing), build a `ParseRequest` (`old_tree` when
  `tree_gen == text_gen` after the bake, `None` after a chain break or before the first install),
  run `parse_worker::do_parse` synchronously on a fresh `tree_sitter::Parser`, `install` the
  result. Returns the bake's `ChainBreak` for the caller to trace-log — the same value `frame_tick`
  carries inside `FrameTickOutcome`.
- An in-flight asynchronous request is left alone. `install` gains one guard so its late result is
  dropped: after the existing config-gen check, the `in_flight` clear, and the stale-gen check,
  **discard a result whose `text_gen == parsed_gen`** (already installed). Document the one edge
  this creates: a synchronous `ParseFailed` (reachable only through cancellation) also makes a later
  successful asynchronous result for that generation discarded; the next edit retries.
- Cost, recorded on the function: after a bake the reparse is incremental (sub-frame); a full parse
  happens only for the first structural command before the worker delivered the initial tree, or
  after a broken edit chain, and is bounded by `syntax-highlight-max-bytes`. Inside a macro or
  dot-repeat batch every step after the first is incremental.
- `attach_sync` becomes `detached` + `ensure_current(BufferId::default(), 1, text, langs)`, keeping
  its empty-text early return (an empty popup is never parsed; the existing hover test is the
  oracle) — one synchronous-parse path, not two.

`hume-editor` wraps it in `commands/structural.rs::ensure_syntax_current(state: &mut EditorState,
bid)`: takes the grammar snapshot (`state.config.languages.grammar_snapshot()`) and an O(1) rope
clone **before** borrowing `buffers.get_mut(bid).syntax`, calls `ensure_current`, and reports a
`ChainBreak` at `Severity::Trace` through the one report helper `reparse_stale_buffers` is refactored
onto (today it inlines the `format!` in `syntax/parse.rs`) — one message, never a second copy of the
string. Returns nothing; callers then read `buf.syntax.as_ref()?.layers()` through a shared borrow.

### 4. Selection policy — `hume-ops`

- `text_object::apply_text_object_by_mode` becomes `pub` (second, cross-crate caller of the exact
  Move/Extend semantics: replace on Move, union with the past-end outward retry on Extend, no-match
  preserves). Its one-line doc grows to state that contract itself — the callees that document it
  today are `pub(crate)`, invisible to a cross-crate reader.
- New `pub fn motion::apply_object_motion(text, sels, mode: MotionMode, count: usize, backward: bool,
  finder: impl Fn(&BufferText, usize) -> Option<(usize, usize)>) -> SelectionSet` in
  `motion/object.rs`, re-exported from `motion/mod.rs` (`pub use object::apply_object_motion;` —
  the `motion/*` modules are private with explicit re-exports):
  - **Move**: per selection, `count` steps; the search origin is `current.end()` going forward and
    `current.start()` going backward; `None` stops early and keeps the last result. Result
    `Selection::new(end, start)` — anchor at the object's end, **head at its start in both
    directions**: the viewport lands on the signature, and a following `w` walks into the object.
    Searching from `end()` means a repeated forward press skips objects nested in the one just
    selected, as Helix does. Record both choices on the function.
  - **Extend**: origin `current.head()`; result `Selection::new(sel.anchor(), if backward { start }
    else { end })` — anchor kept, selection grows to cover the object.
  - Registered as `Motion`, so `SelectionTracking::Extends` applies unchanged: a Move-mode result is
    not replayed by `.` (same as `w`), an Extend-mode step is.
- `text_object/argument.rs`:
  - New `pub fn around_from_inner(text: &BufferText, (start, end): (usize, usize)) -> (usize, usize)`
    — HUME's separator rule. Whitespace means ` `, `\t`, `\n` throughout. **Preceding separator
    first**: scan backward from `start` over whitespace; on a `,` the result is `[that comma, end
    extended forward over whitespace]` (the trailing run before the next `,` or closing delimiter).
    Otherwise scan forward from `end` over whitespace; on a `,` the result is `[start extended
    backward over whitespace, the comma plus the following ` `/`\t` run]` — the backward extension
    always reaches the opening delimiter, never a comma, since the first rule would have fired.
    Otherwise the span is returned unchanged (an only argument has no separator). These extensions
    reproduce the lexical raw-segment bounds exactly, so the existing state triples
    (`foo(-[aaa, ]>bbb, ccc)`, `foo(aaa-[, bbb]>, ccc)`, `foo(aaa, bbb-[, ccc]>)`,
    `foo(-[bar(x, y), ]>z)`) hold unchanged. The grapheme lint scans `hume-ops/src`; its forbidden
    list is the `x += 1`/`x -= 1`/`char_at(x ± 1)` spellings, and `argument.rs`'s existing
    `// grapheme-safe:` markers show the sanctioned way to step over a single-codepoint ASCII comma.
  - Lexical `around_argument` keeps `locate_argument` and its single-segment descent (`foo((a))` on
    the outer bracket resolves to `a` — tested behaviour) and replaces its first-arg/non-first-arg
    branches with `around_from_inner(text, trim_segment(segment)?)`. Behaviour-preserving for every
    existing state triple, which stay the oracle with no red run — say so in the commit. One
    uncovered edge changes and is pinned: an all-whitespace segment (`foo( , bbb)`, cursor in the
    empty slot) matched the raw segment before and is now a no-op, because `trim_segment` yields
    `None` there exactly as it already does for `m i a`; that one new triple gets its red run.
  - `inner_argument`, `around_argument`, `around_from_inner` become reachable from `hume-editor`
    (`pub mod argument`, or re-exports from `text_object`).

### 5. Commands and dispatch — `hume-editor`

#### Body shape

`registry/command.rs::SelectionBody` gains a third shape. It is the registry's existing hook for a
body that needs context the plain signature cannot carry (`Word` precedent, whose doc explains why
such commands stay `Motion`/`Selection` rather than becoming `EditorCmd`s: `meta()` sets `is_motion`
for the `Motion` variant alone, and Move mode's jump-list/dot-repeat handling depends on that flag).

```rust
pub(crate) enum SelectionBody {
    Plain(fn(&BufferText, SelectionSet, usize, MotionMode) -> SelectionSet),
    Word(fn(&BufferText, SelectionSet, usize, WordCtx<'_>) -> SelectionSet),
    Structural(StructuralBody),
}

/// Data, not a fn pointer: kind × span and kind × direction are enumerable, and one interpreter in
/// the dispatch funnel replaces 22 near-identical thin functions.
#[derive(Clone, Copy)]
pub(crate) enum StructuralBody {
    /// `m i <k>` / `m a <k>`: the smallest captured `<kind>.<span>` object at each cursor. Extend
    /// mode grows outward through `apply_text_object_extend`'s past-end retry, so an object that
    /// shares its end with its parent (a nested `def` closing where its class closes) cannot grow to
    /// that parent — the same limit the bracket objects have, accepted rather than special-cased.
    Select { kind: ObjectKind, span: ObjectSpan },
    /// `goto-next-<k>` / `goto-prev-<k>`.
    Goto { kind: ObjectKind, dir: Direction },
    /// `m i a` / `m a a`: `parameter.inside` with the lexical scan as fallback (decision 3).
    Argument { around: bool },
}
```

`Select`/`Argument` register as `MappableCommand::Selection` (`Establishes`, `jump: false`);
`Goto` registers as `MappableCommand::Motion` (`jump: true` — a goto records a jump-list entry).
`meta()` needs no change; Ctrl+key extend is implicit for both variants.

#### One arm in the funnel

`commands/pipeline.rs::run_native_body`'s inner `match fun` (shared by the `Motion | Selection` arm)
gets a `SelectionBody::Structural(body)` arm — the single dispatch funnel is the only place a native
body may be executed (`dispatch_funnel` lint). The arm: `ensure_syntax_current(state, buf)`; in an
inner block, `let spans = object_spans(state.buffers.get(buf), body)`; then
`doc_ops::apply_doc_motion` (`editor/doc_ops.rs`) `(&state.buffers, &mut state.panes.state, focused, buf, |t, s| body.apply(t, s, count, motion_mode, &spans))`.

`commands/structural.rs` holds the rest:
- `object_spans(buf: &Buffer, body: StructuralBody) -> ObjectSpans` — shared borrows only; an empty
  set when the buffer has no syntax, no layers yet, or no layer defines the capture. `Select` →
  `collect(kind, span)`; `Goto` → `collect_for_navigation(kind)` (the `Parameter` → `Inside`
  exception lives there, §2); `Argument` → `collect(Parameter, Inside)`.
- `StructuralBody::apply(self, text, sels, count, mode, spans) -> SelectionSet`:
  - `Select` → `apply_text_object_by_mode(text, sels, mode, |_, p| spans.enclosing(p))`.
  - `Goto` → `apply_object_motion(text, sels, mode, count, dir == Backward, |_, p| spans.adjacent(p, dir))`.
  - `Argument { around: false }` → finder `|t, p| spans.enclosing(p).or_else(|| inner_argument(t, p))`.
  - `Argument { around: true }` → finder
    `|t, p| spans.enclosing(p).map(|s| around_from_inner(t, s)).or_else(|| around_argument(t, p))`.
  The fallback is **per probe**, not per buffer: a comma list the query does not cover (a top-level
  array literal), a region under a syntax error, and a scratch buffer with no grammar all behave
  exactly as today. Where a tree span exists it wins outright: `m i a` on `2` in `foo([1, 2, 3])`
  selects the whole array — the call's argument — where the lexical scan selected `2`; array, tuple
  and struct members are `entry` objects (`m i e`). An empty span set makes `Select`/`Goto` a silent
  no-op through the same path — no early return, no message; record the per-probe choice and the
  tree-wins rule on `Argument`.

#### One table drives registration and keys

`registry/defaults/structural.rs`:

| kind | key | inner | around | next | prev |
|---|---|---|---|---|---|
| Function | `f` | `inner-function` | `around-function` | `goto-next-function` | `goto-prev-function` |
| Class | `t` | `inner-class` | `around-class` | `goto-next-class` | `goto-prev-class` |
| Parameter | `a` | `inner-argument` | `around-argument` | `goto-next-argument` | `goto-prev-argument` |
| Comment | `c` | `inner-comment` | `around-comment` | `goto-next-comment` | `goto-prev-comment` |
| Test | `T` | `inner-test` | `around-test` | `goto-next-test` | `goto-prev-test` |
| Entry | `e` | `inner-entry` | `around-entry` | `goto-next-entry` | `goto-prev-entry` |

Keys follow Helix (`t` = type). `register_structural()` (added to `register_defaults`) loops the
table with struct literals — the `selection!`/`motion!` macros in `registry/defaults/mod.rs` hardcode
`Plain`/`Word` and are not extended — mapping `Parameter` to `Argument` bodies and every other kind
to `Select`/`Goto`. Keep
`fun:` on its own line in those literals: the `dispatch_funnel` lint matches the substring
`Selection { fun` outside `pipeline.rs`. `keymap/defaults.rs::build_text_object_trie` binds the
table's `mi`/`ma` rows from the same table; the `a` row leaves the local lexical list. Names are
`&'static str` literals (built-ins stay `Cow::Borrowed`).

Ripple: `registry/tests.rs::EXPECTED_COMMAND_COUNT` 161 → 183 (22 new names; the two argument
names are reused); every native command is a Steel global, so
`runtime/plugins/core/steel-server/lsp-home/hume-globals.scm` is regenerated
(`HUME_WRITE_STEEL_GLOBALS=1 cargo test -p hume-editor hume_globals_scm_matches_generated_host_names`,
run twice and diffed — `docs/LESSONS.md` L8).

### 6. Keymap

Bound by default (third-level keys under `m i` / `m a`, all currently free except `a`):
`f` function, `t` type (class), `a` argument (existing keys, new body), `c` comment, `T` test,
`e` entry. The twelve `goto-*` commands are registered and **unbound** (decision 2; see Follow-ups).

### 7. Scheme API — `register-grammar!`

`runtime/scheme/prelude.scm` replaces the two-clause `syntax-rules` macro (four- and five-argument
forms) with

```scheme
(define (register-grammar! name grammar-path symbol highlights-path
                           #:injections [injections-path #f]
                           #:textobjects [textobjects-path #f])
  (%register-grammar! name grammar-path symbol highlights-path injections-path textobjects-path))
```

— the same keyword-with-default shape `define-language!` already uses for `#:language-id`. The
`%register-grammar!` builtin stays positional and gains a sixth parameter, `textobjects-path`
(Phase 1). A caller wanting only a textobjects query never pads an injections slot with `#f`, and
a future `#:locals` is a non-breaking addition. `register-grammar!`'s own name in `hume-globals.scm`
is unchanged; Phase 1's `grammar-textobjects-path` helper in `grammars.scm` is a new global, so the
regeneration that phase already performs covers it, as §5's covers the new command names.

### 8. Rejected alternatives (recorded here so they are not re-proposed)

- *Twelve macro-generated `EditorCmd` functions.* Loses `Motion`/`Selection` metadata, needs a new
  builder method, and duplicates a data table as code.
- *Root layer only.* Wrong inside markdown fences and inside injected comments; fixed structurally
  by storing the bundle on every layer.
- *Refuse when the tree is stale.* Silent failure after every edit until the worker catches up.
- *Bake only, no reparse.* Positions become valid, structure stays stale — wrong spans silently.
- *Move-to-start navigation.* Composes worse (`]f` then `m a f` then `d`) and diverges from `w`/`b`,
  which already land on a selected unit.
- *A separate `m i p` / `m a p` family.* Two objects for one concept, and the tree-sitter one would
  ship Helix's raw `parameter.around` wart (deleting the last parameter leaves `, `).
- *Cursor `set_byte_range` as an optimisation.* Truncates grouped hulls.
- *`parameter.around` as the argument navigation target.* Same wart: `goto-next-argument` would
  select the trailing comma that `m a a` deliberately does not.
- *Preferring a lexical inner span nested inside a tree parameter span.* A second rule so `2` stays
  selectable inside `foo([1, 2, 3])`; `m i e` already covers members, and one rule is easier to
  predict.

---

## Implementation plan

Every phase: red run before green for new behaviour (narrow `cargo test <filter>` while iterating);
`cargo fmt`, then `scripts/test-all.sh` once, only in Phase 7. Rationale goes into source comments at
the implementing site, never a reference to this file.

### Phase 1 — Plumbing: `textobjects.scm` reaches the registry

Rust plumbing, the Scheme API in its target shape (§7), the fixtures, and the docs of that API
surface — landed once, directly as specified; no intermediate positional six-argument form.

- [x] `hume-treesitter/src/textobjects.rs` (new): `ObjectKind` (six kinds, `ALL`, `capture_name`),
      `ObjectSpan` (three, `ALL`), `Direction`, `TextObjectsQuery` with the per-`(kind, span)`
      capture-index table and `defines`; grammar-free tests for the table (every `<kind>.<span>`
      name resolves, `@_helper`/`@function.x` resolve to nothing, `defines` on an absent pair).
- [x] `registry.rs`: `GrammarBundle.textobjects: Option<TextObjectsQuery>`;
      `QueryPaths<'a> { highlights: &Path, injections: Option<&Path>, textobjects: Option<&Path> }`
      replaces `LanguageRegistry::attach_grammar`'s two positional query arguments;
      `RegisterError::TextObjectsRead` / `TextObjectsQueryBuild` with `Display` arms — a broken
      file fails the whole attach, like a broken `injections.scm`. `registry/tests.rs`: valid file
      populates the bundle, absent path leaves `None`, broken file fails the attach; existing call
      sites migrate to `QueryPaths`. `test_support.rs::make_bundle` fills the new field.
- [x] `hume-scripting`: `%register-grammar!` gains a sixth positional `textobjects-path`
      (`builtins/syntax.rs::register_grammar` and its doc, `builtins/mod.rs` registration,
      `builtins/syntax/tests.rs`'s two five-value calls); `PendingLanguageReg::Grammar.textobjects_path`;
      `LanguageHost::attach_grammar` (`host.rs`) and `null_host.rs`.
- [x] `hume-editor`: `host_impl.rs`'s `LanguageHost` impl builds `QueryPaths`; `testing/mock_host.rs`;
      `syntax/mod.rs::apply_pending_language_regs` destructures the new field; the fixture-attaching
      tests (`tests/injections_editor.rs`, `incremental_parse.rs`, `lsp_popup_markdown.rs`,
      `scripting_grammar.rs`) migrate to `QueryPaths`.
- [x] Scheme: `prelude.scm` keyword `register-grammar!` (§7); `runtime/scheme/grammars.scm` gains
      `grammar-textobjects-path` and passes `#:injections` / `#:textobjects`, each `#f` when the
      file is absent; `runtime/plugins/core/plum/grammars.scm` gains `plum/try-fetch-textobjects!`
      (best-effort, same `with-handler` shape as `plum/try-fetch-injections!`) and passes it;
      `plum/README.md` lists the query and the pipeline step. `hume-scripting/src/builtins/syntax.rs`'s
      doc no longer describes a macro supplying `#f`.
- [x] Regenerate `hume-globals.scm` (`HUME_WRITE_STEEL_GLOBALS=1 cargo test -p hume-editor
      hume_globals_scm_matches_generated_host_names`, run twice and diffed — `docs/LESSONS.md` L8):
      one new global, `grammar-textobjects-path`.
- [x] Fixtures: `scripts/fetch-test-grammars.sh` generalises `fetch_helix_injections` to
      `fetch_helix_query <name> <file>` and adds `rust` `textobjects.scm` (upstream file has no
      `; inherits:` header, so a plain fetch is safe; a language whose file inherits would need PLUM's
      resolver); `hume-test-fixtures::helix_textobjects_path(name) -> Option<PathBuf>` mirroring
      `helix_injections_path`, always paired with `require_fixture_file` so a missing fixture fails
      loudly instead of passing vacuously with `None`.
- [x] `hume-editor/src/editor/tests/unix/scripting_grammar.rs`: keyword-form tests — `#:injections`,
      and a `#:textobjects`-only call (the motivating shape) asserting the bundle's `textobjects` is
      populated and `injections` is `None`.
- [x] Docs of the API surface: `user-manual/docs/syntax-highlighting.md` (both examples in keyword
      form plus a `#:textobjects` example; "Omit it for a language with nothing embedded" stays
      true), `plugin-api.md` signature row, `configuration.md` mention, `runtime/scheme/prelude.md`
      (no longer "`syntax-rules` macros" — `define-language!` already is a plain `define`),
      `runtime/scheme/README.md` (both optional query files, fail-soft `#f`).

### Phase 2 — Query layer and layer model (`hume-treesitter`)

- [ ] `layers.rs`: `SyntaxLayer.bundle: Arc<GrammarBundle>` replaces `highlighter`; `highlight.rs`
      reads `layer.bundle.highlighter`; `syntax.rs::install` stores the root's and each injection's
      bundle; new `test_support.rs::root_layers(bundle: &Arc<GrammarBundle>, source: &str) ->
      SyntaxLayers` (one root layer, no injections); `layers/tests.rs` builds its layer from
      `make_bundle` instead of a hand-built highlighter.
- [ ] Hull-per-match span collection (`matches()` over `RopeProvider` + `nodes_for_capture_index`,
      zero-width dropped, `prev_grapheme_boundary` end, `debug_assert` on `len_bytes`), with the
      `set_byte_range` rejection recorded on it.
- [ ] `ObjectSpans`: `collect`, `collect_for_navigation` (with the `Parameter` → `Inside` exception),
      `enclosing`, `adjacent`, with the layer-merge rationale on `collect` and the start-keyed
      rationale on `adjacent`.
- [ ] `textobjects/tests.rs`:
      - synthetic-span tests for `enclosing` (nesting → smallest, no containment → `None`, cursor on
        the last char of a span) and `adjacent` (forward/backward, ties → largest end, buffer
        edges → `None`, start-keyed backward lands on the enclosing object);
      - fixture tests with the fetched `rust` `helix-textobjects.scm`: `function.around` on an
        attributed function includes the attributes; `parameter.around` probed on the trailing comma
        is the parameter plus comma; `parameter.inside` on the same parameter excludes it;
        `comment.around` on the last line of a three-line comment is the whole block (HARD STOP on
        failure, see §2); `test.around` on a `#[test]` function spans attribute and body (proves
        the text provider: the pattern's `#eq?` must evaluate); `class.inside` inside an `impl`
        method body picks the `impl` (not the method's block); `collect_for_navigation(Parameter)`
        yields the inside spans (no trailing comma);
      - `.movement` priority with an inline query defining `function.movement` on the name node:
        navigation spans are the names, selection spans are unaffected;
      - layers with the markdown + rust fixtures: a cursor inside a fenced Rust function selects it
        via `collect`; a cursor in prose yields `None`; a cursor in a fence outside any function
        yields `None` for `Function` (markdown defines no objects) — proving the merge, not a walk.
- [ ] `hume-editor/src/editor/lints/grapheme.rs`: add `hume-treesitter/src/textobjects.rs` to the
      scanned set (it is selection code) — the path list, the lint's module doc that enumerates it,
      and the matching "Enforced" sentence in `CLAUDE.md`.

### Phase 3 — Freshness (`hume-treesitter`)

- [ ] `Syntax::ensure_current` as specified in §3; `install`'s `text_gen == parsed_gen` guard in the
      stated position with the `ParseFailed` edge documented; `attach_sync` re-expressed through it,
      empty-text early return kept.
- [ ] `syntax/tests.rs`: stale tree (edit recorded, no `frame_tick`) → `ensure_current` installs a
      tree whose root spans the new text and sets `parsed_gen`; a same-generation `ParseDone`
      installed afterwards is discarded and still clears `in_flight`; a broken chain → full reparse
      with a `ChainBreak` returned; up to date → `None` and no change to `tree_gen`; `attach_sync`
      still yields queryable layers for hover markdown (existing test stays green).

### Phase 4 — Selection policy (`hume-ops`)

- [ ] `apply_text_object_by_mode` → `pub` with its contract in its doc; `argument` finders reachable
      from `hume-editor`.
- [ ] `motion/object.rs::apply_object_motion` (§4), re-exported from `motion/mod.rs`, with
      `motion/tests/object.rs` state triples: Move forward from a cursor, from a selected object
      (nested objects skipped), backward from inside an object (lands on it, head at start), `count`
      2, no next object (unchanged), Extend forward/backward (anchor kept), multi-cursor convergence
      merges.
- [ ] `around_from_inner` with state triples in `text_object/tests/argument.rs` — the initial state
      selects the inner span (`foo(aaa, -[bbb]>, ccc)`), the expected state is the around span:
      first / middle / last / only / multi-line `,\n    arg`; lexical `around_argument` refactored
      onto it — the existing characterization triples unchanged with no red run, plus the one pinned
      edge (`foo( , bbb)` with the cursor in the empty slot → selection unchanged) with its red run.

### Phase 5 — Editor wiring (`hume-editor`)

- [ ] `SelectionBody::Structural` + `StructuralBody` (§5) in `registry/command.rs`.
- [ ] `commands/structural.rs`: `ensure_syntax_current` (the `ChainBreak` trace report
      `reparse_stale_buffers` inlines today becomes the one helper both call), `object_spans`,
      `StructuralBody::apply`.
- [ ] The `Structural` arm in `run_native_body`'s inner `match fun`.
- [ ] `registry/defaults/structural.rs` table + `register_structural()`; `register_defaults` calls
      it; `EXPECTED_COMMAND_COUNT` → 183; the lexical `inner-argument`/`around-argument`
      registrations in `defaults/text_objects.rs` are removed (their names now register here).
- [ ] `keymap/defaults.rs`: `mi`/`ma` rows from the table; the `a` row leaves the lexical list.
- [ ] Regenerate `hume-globals.scm` (twice, diffed).
- [ ] `hume-editor/src/editor/tests/structural.rs` (`rust` fixture grammar + fetched
      `helix-textobjects.scm`, attached through `ed.state.config.languages.attach_grammar(..)` with
      `QueryPaths { textobjects: Some(..), .. }` as `tests/injections_editor.rs` does; keys fed
      through the real keymap):
      - `m i f` / `m a f` inside a function, outside any function (unchanged), inside a nested
        closure (smallest wins), on an attributed function (`m a f` includes attributes);
      - Extend mode: `e` `m i f` unions; a second `m i f` grows outward to the enclosing function;
      - navigation via `execute_keymap_command(name.into(), count, extend, ArgSource::Keymap)` (the
        commands are unbound): `goto-next-function` from a cursor selects the whole next function
        with head at its start; a second press skips a closure nested in it;
        `goto-prev-function` from inside a function selects that function; `2` count; at buffer
        edges the selection is unchanged; a jump-list entry is recorded (`Ctrl+o` returns);
        Extend mode keeps the anchor; `goto-next-argument` selects the trimmed argument, no comma;
      - multi-cursor: two cursors in two functions → two selections; two in one function → merged;
      - no grammar: every structural command is a no-op with the selection unchanged;
      - dot-repeat: `m a f` `d` then `.` deletes the function under the new cursor;
      - **macro replay freshness**: record `d` (deleting a selected line inside a function) followed
        by `m i f` `d` in one macro, replay it on another function in one `drain_replay_queue`
        batch — the second step sees the post-edit tree (no panic, correct span);
      - unified argument: `m a a` on a parameter whose sibling contains `","` inside a string
        literal selects only the parameter plus separator (lexical would be fooled); `m a a` on the
        last parameter eats the preceding `, `; `m i a` in a top-level array literal (no `parameter`
        capture) falls back to the lexical scan; `m i a` on `2` in `foo([1, 2, 3])` selects the
        whole array (tree span wins); `m i a` in a scratch buffer with no grammar still works.

### Phase 6 — Docs, changelog, roadmap

- [ ] `CHANGELOG.md` under *Unreleased*: a **Breaking** bullet for `register-grammar!`; feature
      bullets for the structural objects (with keys), the structure-aware argument object (noting
      that inside a call a nested list is now the argument — `m i e` for its members), and the
      unbound navigation commands (how to bind them).
- [ ] `user-manual/docs/selections.md`: rows for `m i f`/`m a f`, `m i t`/`m a t`, `m i c`/`m a c`,
      `m i T`/`m a T`, `m i e`/`m a e`; the `m i a`/`m a a` row notes it is structure-aware when
      the language's grammar provides objects; one sentence that these need a grammar with a
      `textobjects.scm` (installed by PLUM alongside highlights) and are otherwise no-ops.
- [ ] `user-manual/docs/default-keys.md`: mirror the text-object rows.
- [ ] `user-manual/docs/builtin-commands.md`: the 22 new command rows (navigation rows with no key).
- [ ] `user-manual/docs/moving-around.md`: a "Structural navigation" section listing the twelve
      commands, what they select, that `:goto-next-function` runs them from the command line, and a
      `bind-key!` example.
- [ ] `docs/learning/tree-sitter-pipeline.md`: a section on text-object queries at the concept
      level — captures grouped into one object, smallest-enclosing for select, nearest-start for
      navigation, why injected layers need no special handling, why the tree is brought up to date
      before a query. No file paths or function names.
- [ ] `docs/CRATES.md` and `hume-treesitter/src/lib.rs`'s crate paragraph: add structural text
      objects and navigation to what the crate owns.
- [ ] `docs/ROADMAP.md`: remove both tree-sitter items (the file's convention is removal, not
      `[x]`); `locals.scm` stays covered by the "Scope-aware local rename" item; the open question
      on `goto-matching-pair`'s lexical tag matching stays open — it is the `xml-element` follow-up.

### Phase 7 — Verification

- [ ] `cargo fmt`, then `scripts/test-all.sh` once (fetches the grammar fixtures, runs doctests).
- [ ] Manual smoke: open a `.rs` file with the `rust` grammar and PLUM-fetched `textobjects.scm`;
      `m i f` / `m a f` inside a method; `m a f` on an attributed function includes the attribute;
      `m a a` on the last parameter then `d` leaves no dangling `, `; `:goto-next-function` from
      the command line (any mappable command runs there without a binding) selects the next
      function with the cursor on its first line; `m i f` in a scratch buffer does nothing.

## Follow-ups (not part of this spec)

- **Default keys for navigation** — the complete set, Helix/unimpaired layout:
  `]f` / `[f` function, `]t` / `[t` type (class), `]a` / `[a` argument, `]c` / `[c` comment,
  `]T` / `[T` test, `]e` / `[e` entry. Prerequisite: `[` / `]` currently cycle the kill ring
  (`paste-ring-older` / `paste-ring-newer`); they need a new home first (candidate: `Ctrl+y` older /
  `Ctrl+n` newer — both legacy-terminal-safe control bytes, both unbound). Natural later residents of
  the same prefix: `]d` / `[d` diagnostics (now `g n` / `g p` in `core:lsp`), `]g` / `[g` git hunks,
  `]p` / `[p` paragraph (now `}` / `{`).
- `xml-element` object kind (Helix defines it; HUME's lexical tag matcher is the current answer).
- `locals.scm` (scope-aware rename) — `#:locals` on `register-grammar!` is now a non-breaking
  addition.
