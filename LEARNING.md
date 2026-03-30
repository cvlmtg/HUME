# HUME — Learning Notes

Concepts that come up while building HUME, explained in enough depth to be
useful later. Sections are added as topics arise during development.

---

## Unicode Position Model: Bytes, Chars, and Grapheme Clusters

Understanding this hierarchy is essential for HUME's architecture. Three
different units can describe a "position" in text, and choosing the wrong one
at the wrong layer causes subtle, hard-to-reproduce bugs.

### Byte offset

A byte offset is a raw index into memory. In UTF-8 (Rust's string encoding),
characters are **variable-width**: 1 to 4 bytes each.

```
"café"
 c  a  f  é
 1  1  1  2   ← bytes per character
```

| Char | Bytes   | Byte offsets |
|------|---------|-------------|
| `c`  | `63`    | 0 |
| `a`  | `61`    | 1 |
| `f`  | `66`    | 2 |
| `é`  | `C3 A9` | 3, 4 |

`é` occupies bytes 3 **and** 4. Byte offset 4 points into the **middle** of a
character — it is not a valid character boundary. This is why `s[3..4]` on
`"café"` panics in Rust: slicing through a multi-byte character is undefined.

Byte offsets are used internally by Rust's `str` and by `ropey`, but they are
**never exposed across module boundaries** in HUME. They are an implementation
detail.

### Char offset

A char offset counts **Unicode scalar values** (Rust's `char` type),
regardless of how many bytes each one takes.

```
"café"
 c  a  f  é
 0  1  2  3   ← char offsets
```

`é` is a single `char` at offset 3 — no partial-character hazard. This is
`ropey`'s native addressing unit, and it is what HUME's `Buffer`, `Selection`,
and `SelectionSet` use for all positions.

Char offsets make sense for an editor at the storage layer:
- `insert(at, text)` and `remove(from, to)` can be expressed cleanly.
- `anchor` and `head` in a `Selection` are meaningful without knowing the
  encoding of any particular character.

### Grapheme cluster

A char offset solves the byte problem, but there is a level above it:
**grapheme clusters** — what a user perceives as a single indivisible
character, which may be composed of multiple Unicode scalar values.

```
"é"  can be:
  U+00E9             → 1 char  (precomposed NFC form)
  U+0065 + U+0301    → 2 chars (base 'e' + combining acute accent)

"👨‍👩‍👧"              → 1 visible character, but 5 chars
                       (joined with zero-width joiners U+200D)
```

Pressing the right-arrow key on `"👨‍👩‍👧"` should advance the cursor past the
entire emoji in one step, not stop five times. This is the job of
`grapheme.rs`: it takes a `RopeSlice` and returns the next/previous **valid
grapheme boundary** as a char offset.

### Architectural rule

| Unit | Granularity | Role in HUME |
|------|-------------|--------------|
| Byte offset | Raw memory | Internal to `ropey` — never exposed |
| Char offset | Unicode scalar value (`char`) | Storage, selections, `Buffer` API |
| Grapheme cluster | User-perceived character | Cursor movement, motions (`grapheme.rs`) |

The boundary between layers is strict: `grapheme.rs` **consumes** char offsets
and **produces** char offsets that happen to land on grapheme boundaries.
Everything above it works purely in char offsets and never needs to know about
bytes or grapheme internals.

---

## Edit Operations: Acting on Selections

### The select-then-act model

In HUME, edit operations never act on a bare cursor position. They act on a
`SelectionSet` — which is always a `Vec<Selection>`. Selections are always
**inclusive**: `anchor == head` is a 1-char selection covering the character at
that index, not a zero-width point. Each `Selection` is either:

- **Single-character** (`anchor == head`): the cursor sits on exactly one character.
- **Multi-character** (`anchor != head`): a contiguous region of selected text.

An operation like "insert character `x`" means:

- For a **single-character selection**: insert `x` before the cursor character;
  the cursor advances to the next character.
- For a **multi-character selection**: replace the entire selected region with `x`.

This is the same rule in both cases. Single-cursor editing, visual-mode editing,
and multicursor editing all fall out of the same loop.

### Multi-selection edit ordering

A `SelectionSet` can contain multiple selections simultaneously (multicursor).
When an edit touches multiple positions, **the order of application matters**:
inserting a character at offset 0 shifts every position to its right, so
naively applying edits one-by-one would corrupt subsequent offsets.

HUME avoids this entirely with `ChangeSetBuilder`: all input positions are
expressed in **original-buffer coordinates**, and the builder handles the
translation internally. See the Changesets section below.

### Primary vs secondary selections

All selections are **equal for editing** — insert, delete, and motions apply
to every selection in the set simultaneously. The *primary* is just the
"focused" one. It is distinguished in four specific situations:

1. **Status bar**: shows the primary's line and column position. You can't
   display all N cursors at once — one has to be canonical.

2. **Viewport scrolling**: the editor scrolls to keep the primary visible.
   Other cursors may be off-screen — that is fine and expected.

3. **Single-selection commands**: `cmd_keep_primary_selection` (keep primary
   only) and `cmd_remove_primary_selection` (remove primary) operate on
   exactly one selection. The primary determines which one.

4. **Registers** (`src/register.rs`): when you yank with N cursors, the
   register stores a **list of N strings**, one per selection in document
   order. Pasting with N cursors maps each slot back to the corresponding
   cursor. If the cursor count doesn't match at paste time, the full register
   content is pasted at every cursor as a fallback.

   HUME uses mnemonic register names rather than the traditional Vim/Helix
   convention (`"`, `+`, `_`). Since 10 named registers (`0`–`9`) cover all
   real workflows, letters are freed for intuitive special names:

   | Key | Register | Notes |
   |-----|----------|-------|
   | `0`–`9` | Named storage | Text or macros; last write wins |
   | `q` | Default macro | `qq` records, `Q` replays |
   | `c` | System clipboard | Deferred to M3 |
   | `b` | Black hole | Discards writes |
   | `s` | Search | Holds last search pattern |

   The default register (receives all yanks/deletes when no register is
   named) is an internal sentinel (`'"'`) — users never type it.

   **Why not `a`–`z`?** Traditional named registers borrow letters for text
   storage, forcing special registers into punctuation (`+`, `_`). HUME flips
   this: numbers for user storage, letters for special registers.

   **Macro model (M3):** macros are stored in registers (Vim model, not
   Helix's single-slot model). `qq` records into register `q` (the default
   macro register). `q3` records into register `3`. `Q` replays from `q`,
   `Q3` replays from `3`.

   **Why Vim-style macros over Helix-style?** Helix has a single macro slot
   (`Q` records, `Q` replays). Users complained — one slot is enough for a
   single task, but when you need two independent macros (e.g. one that
   transforms a line, another that moves between sections) you must
   re-record the first each time. HUME's register-based macros solve this
   without the full `a`–`z` namespace overhead. Ten slots (`0`–`9`) covers
   real workflows; the `q` default keeps the common case a one-key operation.

5. **Paste-as-replace** (`src/edit.rs`): In a select-then-act model, `p`/`P`
   has to handle two distinct cases:

   - **Cursor** (`anchor == head`, a fresh 1-char selection): insert the
     register contents *after* or *before* the cursor char. Same as Vim's `p`/`P`.
   - **Explicit selection** (more than 1 char, created intentionally): *replace*
     the selected text with the register contents, and return the displaced text
     to the caller so it can be written back to the register (a swap).

   The key insight is `sel.is_cursor()` — the selection state already encodes
   whether the user made an intentional selection. No separate `R` command
   needed. No `"0` register hack needed (in Vim, yanking always writes `"0`
   in addition to `"`, so after a delete you can still paste the pre-delete
   yank with `"0p`; HUME avoids the problem by never clobbering the register
   on replace).

   The return type of `paste_after`/`paste_before` is `(Buffer, SelectionSet,
   ChangeSet, Vec<String>)`. The fourth element contains the displaced text
   (empty strings for cursor pastes). The editor layer writes it back to the
   source register, completing the swap.

**Why cycle the primary?** In a keyboard-only multi-cursor world,
`cmd_cycle_primary_forward` and `cmd_cycle_primary_backward` are how you
"focus" a different cursor — to make the viewport scroll to it, read its
position in the status bar, or target it with `cmd_remove_primary_selection`.
There is no mouse click to promote a cursor; cycling is the keyboard
equivalent.

Internally, `SelectionSet.primary` is an index into the sorted
`Vec<Selection>`. The index is updated whenever the set changes: merges that
absorb the primary, removals before or at it, and splits all adjust the index
so it keeps pointing at the intended selection.

---

## Changesets: Describing Edits as Data

### What is a changeset?

A changeset is a **compact, invertible description** of a document
transformation. Instead of mutating the buffer for each selection, we build
one changeset that describes all the edits, then apply it once.

The representation is a sequence of three operations:

| Operation | Meaning |
|-----------|---------|
| `Retain(n)` | Skip `n` chars unchanged |
| `Delete(n)` | Remove `n` chars from the old doc |
| `Insert(s)` | Add `s` to the new doc |

**Example:** Insert `!` at positions 0 and 6 in `"hello world"`:

```
Insert("!"), Retain(6), Insert("!"), Retain(5)
```

This single object describes the entire multi-cursor edit. `apply` takes the
buffer by reference, clones the underlying rope (O(1) — Ropey uses Arc-based
structural sharing), and executes Delete/Insert operations on the clone —
each O(log n). Retain operations are free (the chars are already there).
Total cost: O(k log n) for k non-retain operations.

The original buffer remains intact after `apply`. The caller must still call
`invert` before `apply` if it needs the inverse, because `invert` reads
deleted text from the original rope at inversion time.

### Why not just mutate the buffer directly?

Direct mutation (clone + edit per selection) works, but the edit is lost
after application — there is no record of what changed. A changeset preserves
the edit as data, which enables:

1. **Undo/redo.** Invert the changeset to get an undo operation:
   - `Retain(n)` → `Retain(n)` (no change)
   - `Delete(n)` → `Insert(deleted text)` (re-insert what was removed)
   - `Insert(s)` → `Delete(len(s))` (remove what was added)

   Applying the inverse to the result buffer gives back the original.

2. **Composition.** Two sequential changesets A→B and B→C can be merged into
   a single A→C changeset. This is essential for grouping keystrokes into
   undo steps (typing a word should undo as one operation, not per-character).

3. **Position mapping.** Given a position in the old document, `map_pos`
   computes where it ends up in the new document — accounting for all
   insertions and deletions. An `Assoc` parameter (`Before`/`After`) controls
   which side of an insertion the position sticks to.

   Note that edit operations and undo/redo never call `map_pos`. Edit
   operations use `new_pos()` directly; undo/redo restores selections from the
   stored `Transaction` (see below). `Assoc` is reserved for **external
   positions** — things that exist independently of any specific edit, like LSP
   diagnostic ranges or bookmarks. When a diagnostic sits at offset 5 and text
   is inserted at offset 5, `Assoc::Before` keeps it glued to the left of the
   insertion; `Assoc::After` pushes it past.

### The builder pattern

Edit operations build changesets incrementally using `ChangeSetBuilder`. The
builder tracks two cursors:

- `old_pos` — how far we have consumed in the old document
- `new_pos` — how far we have produced in the new document

This dual tracking replaces the manual delta accumulator. After each
`insert()` call, `new_pos()` tells you exactly where a cursor should land
in the result buffer.

```text
Builder state for insert_char('x') with cursor at offset 3 in "hello":

  b.retain(3)     →  old_pos=3, new_pos=3    (skip "hel")
  b.insert("x")   →  old_pos=3, new_pos=4    (insert 'x')
  b.retain_rest()  →  old_pos=5, new_pos=6    (keep "lo")

  Result: Retain(3), Insert("x"), Retain(2)
  Cursor: b.new_pos() at time of insert = 4  →  "helx|lo"
```

All positions are in **original-buffer space** — no delta tracking, no
intermediate buffer clones. The builder handles the coordinate translation
internally.

### Transactions: changesets with cursor state

A `ChangeSet` describes only the text change. A `Transaction` pairs it with the
**post-apply** `SelectionSet` — where the cursors land *after* the changeset is
applied. This invariant holds for every Transaction, forward or inverse.

At edit time you build **two** Transactions from the same changeset:

```text
// 1. Capture the inverse BEFORE apply — both read from the same old_buf.
let inv_cs  = cs.invert(&old_buf);
let new_buf = cs.apply(&old_buf);         // old_buf still valid after this

// 2. Build both Transactions from the same cs/inv_cs.
let forward = Transaction::new(cs,     post_edit_sels);  // for redo
let inverse = Transaction::new(inv_cs, pre_edit_sels);   // push to undo stack
```

The inverse Transaction's `selection` is `pre_edit_sels` — the cursors from
*before* the edit — because that is where applying the inverse will leave the
cursors. The "always post-apply" invariant holds: after running the inverse,
the cursors are at `pre_edit_sels`.

**Timing matters.** `invert(&old_buf)` must be called before discarding
`old_buf`. `invert` reads deleted text from the original rope to reconstruct
the `Insert` operations — it captures those chars at inversion time. In
practice `Document::apply_edit` enforces this: it calls `cs.invert(&self.buf)`
while `self.buf` still holds the pre-edit content, then overwrites it.

The history manager stores both Transactions. Applying the inverse restores
both the text and the cursor positions in a single step (undo); applying the
forward Transaction redoes the edit.

### Implementation

- `src/changeset.rs` — `Operation`, `ChangeSet`, `ChangeSetBuilder`, `Assoc`
- `src/transaction.rs` — `Transaction` (pairs ChangeSet with SelectionSet)
- `src/edit.rs` — edit operations build changesets via the builder
- `src/history.rs` — arena-based undo tree; stores forward/inverse Transaction pairs per revision

  An **arena** is a `Vec` that owns all the nodes of a tree or graph. Instead
  of linking nodes with pointers or `Rc<RefCell<...>>`, each node stores plain
  integer indices into the `Vec`. Lookups are O(1) array accesses; there are no
  reference cycles for the borrow checker to worry about; and the allocator sees
  one contiguous allocation instead of many small heap objects. The trade-off is
  that nodes are never individually freed — the whole arena is dropped at once.
  For an undo tree that only grows, this is fine.
- `src/document.rs` — orchestrates Buffer + SelectionSet + History; enforces the invert-before-apply timing invariant in `apply_edit`

---

## Buffer Invariants and Plugin Safety

### The invariants

Every `Buffer` must satisfy two invariants at all times:

1. **Trailing newline**: the rope always ends with `\n`. This is the
   "structural newline" — it guarantees every line has a terminator, and it
   means cursors can always sit on a valid character (the last char is always
   accessible, never past-the-end).

2. **Non-empty**: `len_chars() >= 1`. Follows from the trailing newline, but
   worth naming explicitly because several algorithms assume it.

Every `Selection` must also satisfy:

3. **In-bounds**: `head < buf_len` and `anchor < buf_len` for the buffer it is
   paired with.

### Fail fast, don't silently repair

When an invariant is about to be violated, there are three options:

1. **Silent repair** — e.g. append the missing `\n` automatically.
2. **Panic** — crash immediately with a message.
3. **Return `Err`** — reject the operation, leave the original state untouched.

Silent repair is tempting but dangerous. A changeset carries metadata
(`len_after`) that the rest of the algebra relies on. A repair that appends
`\n` changes the buffer length but not `len_after`, so `compose`,
`invert`, and `map_pos` all silently operate on the wrong value. The corruption
is invisible until it manifests as a wrong cursor position or a length-mismatch
panic somewhere else entirely.

Panicking is better — it fails loudly at the source — but it crashes the editor
for a mistake in one plugin. An editor should be resilient: a broken plugin
should not take down other buffers.

**Return `Err`** is the right choice at the trust boundary, for the same reason
`Result` is preferred over `panic` in Rust library code: the caller can decide
how to handle the failure. In HUME, `Transaction::apply` is that boundary. A
plugin that assembles an invalid `Transaction` gets a clear error; the original
buffer is unmodified; the editor continues running.

### Validate at the trust boundary, not everywhere

There are two kinds of call sites:

- **Internal commands** (`insert_char`, `delete_char_forward`, etc.): these
  build changesets by construction and are always correct. They call
  `ChangeSet::apply` directly and use `.expect()` — a panic here is a bug in
  the engine, not a plugin mistake.

- **Plugin entry point** (`Transaction::apply`): a plugin assembles a
  `Transaction` from raw parts. This is the one place where untrusted data
  enters the system. `apply` validates here and returns `Result`.

Adding `Result` to every internal function would be noise — it would force
callers to handle errors that can never happen. The right design is: validate
once at the boundary, trust everything inside.

`debug_assert!` fills the gap for internal code: these assertions run during
development and tests (where you want loud feedback on engine bugs) but compile
to nothing in release builds (where the cost would be wasted).

### `apply` takes `&Buffer`, not `Buffer`

The original `ChangeSet::apply` consumed the buffer (`buf: Buffer`). This was
an optimization: the rope is mutated in-place rather than cloned. But consuming
the buffer creates a recovery problem — if `apply` fails, the original buffer
is gone.

The solution is to take `&Buffer` instead. Ropey's `Rope::clone()` is **O(1)**
because the tree is built from `Arc`-counted nodes: cloning a rope just bumps a
reference count. `apply` clones the rope, mutates the clone, checks the
post-conditions, and only on success wraps the clone in a new `Buffer`. If
anything goes wrong, the clone is dropped and the caller still holds the
original.

```rust
pub(crate) fn apply(&self, buf: &Buffer) -> Result<Buffer, ApplyError> {
    if buf.len_chars() != self.len_before { return Err(...); }
    let mut rope = buf.rope().clone();   // O(1)
    // ... mutate rope ...
    if rope.char(rope.len_chars() - 1) != '\n' { return Err(...); }
    Ok(Buffer::from_rope(rope))
}
```

### Inverse changeset cleanup is free

When `apply` fails, the caller may have already built an inverse changeset
(for the undo stack). There is no need for explicit cleanup: the inverse is
just a `ChangeSet` on the stack. Rust's ownership model drops it automatically
when the error branch is taken and it goes out of scope.

```rust
let inv_cs = cs.invert(&buf);           // build inverse first
match cs.apply(&buf) {                  // apply takes &buf — original intact
    Ok(new_buf)  => { /* push inv_cs to undo stack */ }
    Err(e)       => { /* inv_cs dropped here — no cleanup needed */ }
}
```

This is a good example of ownership making resource management "fall out for
free". In a garbage-collected language you would need to either let the GC
collect it eventually or explicitly null the reference; in Rust it is
deterministic and requires no code.

### Error type design

`TransactionError` wraps two distinct failure sources:

- `ApplyError` — changeset-level failures (`LengthMismatch`, `TrailingNewlineMissing`)
- `ValidationError` — selection-level failures (`SelectionOutOfBounds`, `EmptyBuffer`)

The `From` trait lets `?` convert each into `TransactionError` automatically:

```rust
pub(crate) fn apply(&self, buf: &Buffer) -> Result<(Buffer, SelectionSet), TransactionError> {
    let new_buf = self.changes.apply(buf)?;        // ApplyError → TransactionError via From
    self.selection.validate(new_buf.len_chars())?; // ValidationError → TransactionError via From
    Ok((new_buf, self.selection.clone()))
}
```

This is the standard Rust pattern for layering error types: inner functions
return narrow errors; outer functions define a wider type that covers all their
failure modes; `From` + `?` compose them with no boilerplate.

### `debug_assert` vs `assert` vs `Result`

| Mechanism | Fires in | Use when |
|---|---|---|
| `debug_assert!` | debug + test builds only | Internal invariant that trusted code should never violate; catching it in tests is enough |
| `assert!` / `assert_eq!` | all builds (panics) | Structural precondition that is unrecoverable and implies a programming error (e.g. `ChangeSetBuilder::finish` verifies you consumed all chars) |
| `Result::Err` | all builds (recoverable) | Trust boundary — caller provided invalid data that the system can reject gracefully |

HUME's rule: `debug_assert` for engine internals, `assert` for builder
contracts that imply a programming mistake too severe to recover from, `Result`
for anything that crosses the plugin boundary.

---

## Motions vs Text Objects

### The conceptual split

Both motions and text objects take a cursor position and produce a selection.
The difference is in how the anchor of that selection is determined:

| Concept | Inner fn output | Anchor of resulting selection |
|---------|----------------|-------------------------------|
| Motion | new *head* position | determined by `MotionMode` (may come from old selection state) |
| Text object | absolute `(start, end)` range | always `start` — independent of previous selection |

A motion inner function only answers "where does the head go?". With
`MotionMode::Move` (`h`, `l`, `j`, `k`), the anchor collapses to the new head,
producing a single-character selection. With `MotionMode::Extend`, the anchor
stays fixed and only the head moves — growing the selection.

A text object bypasses `MotionMode` entirely. It returns a complete range and
the framework always creates `Selection::new(start, end)` — the previous
anchor is discarded.

Word motions (`w`/`b`/`W`/`B`) sit in between: navigational like motions but
returning a full word range. They use a third framework, `apply_word_select`,
described in the next section.

This leads to three framework functions: `apply_motion`, `apply_text_object`,
and `apply_word_select`.

### The inner function pattern

Both frameworks follow the same design: the inner function is *pure and
ignorant of multi-cursor*. It receives one position and returns one result.
The framework function handles iterating over all selections and merging any
that converge to the same range.

```rust
// Motion inner function: position → position
fn move_right(buf: &Buffer, head: usize) -> usize { ... }

// Text object inner function: position → Option<(start, end)>
fn inner_word(buf: &Buffer, pos: usize) -> Option<(usize, usize)> { ... }
```

Returning `Option` from a text object inner function means "no match at this
position". On `None`, the existing selection is preserved — `mi(` when not
inside parens is a no-op, matching Helix behaviour.

### `map_and_merge`

Both `apply_motion` and `apply_text_object` use `map_and_merge` on the
`SelectionSet`. After mapping each selection through the inner function, any
selections that have converged to the same range are automatically merged into
one. This is essential for multicursor correctness: if two cursors are both
inside the same bracket pair and you press `mi(`, you don't want two identical
overlapping selections — you want one.

---

## MotionMode: Separating Position from Anchor Semantics

### A concrete walkthrough

Buffer: `"hello world\n"`, cursor on `'h'` (position 0).

Before `l` is pressed, the `SelectionSet` contains one selection:

```
Selection { anchor: 0, head: 0 }   ← single-char selection on 'h'
```

Pressing `l` calls `apply_motion(buf, sels, MotionMode::Move, 1, move_right)`.

**Step 1 — inner motion function.** `move_right(buf, head=0)` returns
`next_grapheme_boundary(buf, 0)` = 1. It knows nothing about the old selection
or anchors — just a coordinate calculation.

**Step 2 — apply `MotionMode::Move`.** `apply_motion` builds the new selection:

```
Move → anchor = new_head (1), head = new_head (1)
Result: Selection { anchor: 1, head: 1 }
```

The cursor is on `'e'`, a single-character selection.

Now suppose the cursor is at `{ anchor: 2, head: 2 }` on `'l'` and the user
triggers an extend-mode variant:

**Step 2 — apply `MotionMode::Extend`.**

```
Extend → anchor = old_anchor (2), head = new_head (3)
Result: Selection { anchor: 2, head: 3 }
```

The selection grew from `'l'` to cover both `'l'` characters (`"ll"`) — the
anchor stayed put.

### The two modes

| Mode | Anchor | Head | Typical use |
|------|--------|------|-------------|
| `Move`   | `new_head`   | `new_head` | Plain cursor move — `h`, `j`, `k`, `l` |
| `Extend` | `old_anchor` | `new_head` | Grow selection — extend-mode variants |

`Move` always produces a collapsed single-character selection (anchor == head).
`Extend` keeps the existing anchor, only moving the head.

> **Historical note:** `MotionMode` originally had a third value — `Select`,
> which set the anchor to the old *head*. This was the Kakoune model for word
> motions: `w` accumulated the traversed span from cursor to next word start.
> `Select` was removed when `w`/`b`/`W`/`B` were redesigned to select the
> whole destination word via `apply_word_select` — see the next section.

### Why separate the inner function from the mode

The inner function `fn(&Buffer, usize) -> usize` is a pure coordinate
calculation — it knows nothing about anchors or multi-cursor. `MotionMode` is
a concern of the keymap layer, not of the motion itself. This means:

- Adding a new motion (e.g. "next paragraph") requires one position function;
  Move and Extend variants come for free.
- Testing the motion is simple: just assert on the returned position.
- The same `move_right` inner function powers both `l` (Move) and its
  extend-mode variant (Extend).

```rust
match mode {
    MotionMode::Move   => Selection::cursor(new_head),
    MotionMode::Extend => Selection::new(sel.anchor, new_head),
}
```

---

## Word Motions: Selecting the Whole Word

### A third framework

Three distinct patterns exist for creating selections from cursor movement:

| Framework | Inner fn signature | Anchor | Typical use |
|---|---|---|---|
| `apply_motion` | `fn(&Buffer, usize) -> usize` | Via `MotionMode` | `h/j/k/l`, paragraph, goto-line |
| `apply_text_object` | `fn(&Buffer, usize) -> Option<(usize, usize)>` | Always `start` | `iw`, `i(`, `i"` |
| `apply_word_select` | `fn(&Buffer, usize) -> Option<(usize, usize)>` | Always `word_start` | `w/b/W/B` |

`apply_word_select` occupies a middle ground: its inner function returns a full
range like a text object, but it is navigational like a motion — counting,
crossing line boundaries, and stopping at buffer edges.

```rust
fn apply_word_select(
    buf: &Buffer,
    sels: SelectionSet,
    count: usize,
    motion: impl Fn(&Buffer, usize) -> Option<(usize, usize)>,
) -> SelectionSet
```

When the inner function returns `None` (no word in that direction), the
iteration stops early and the current selection is preserved — a true no-op.
Compare `apply_motion`, where the inner function always returns a position.

### Kakoune, Helix, and HUME

Word motions reflect three distinct design philosophies, best illustrated by
"change the second word" starting from column 0 in `"hello world"`:

**Kakoune** (`w` selects the traversed span, anchor at old head):
```
w   → "hello w" selected (traversed span)
e   → reanchors at 'w', extends to end of "world"
c   → change "world"       (3 keystrokes)
```
Motions double as selection builders. Composable, but indirect — you select
what you cross on the way, not the word itself.

**Helix** (`w` = Move, pure navigation):
```
w    → cursor jumps to 'w', single-char selection
iw   → text object selects "world"
c    → change "world"       (3 keystrokes)
```
Predictable — `w` always means "go there". But acting on a word always needs
a second gesture (`iw`).

**HUME** (`w` selects the whole destination word):
```
w    → "world" selected directly
c    → change "world"       (2 keystrokes)
```
The common case — act on a word — requires no second gesture. This also
eliminates `e`/`E`: in Helix/Vim, `e` reaches the end of the current word
(complementing `w` which lands on the start of the next). In HUME, `w`
already selects through the end, making `e` redundant.

### Line crossing: the double-step in `select_next_word`

`Eol` is its own `CharClass` (see the next section), so `next_word_start`
always stops at a `\n`. A single call from the end of a line lands on the
newline itself, not the first word of the next line. `select_next_word` adds
a second step when this happens:

```rust
let mut word_start = next_word_start(buf, pos, is_boundary);

// If we landed on a non-trailing '\n', cross the line.
if word_start < len.saturating_sub(1) {
    if classify_char(buf.char_at(word_start)) == CharClass::Eol {
        word_start = next_word_start(buf, word_start, is_boundary);
    }
}
```

The second call treats the `\n` as whitespace and advances to the first word
of the next line — making `w` cross line boundaries as users expect.

### Mid-word detection: the double-step in `select_prev_word`

`prev_word_start` finds the start of the word *at or containing* a position.
When the cursor is mid-word, it returns the current word's start — not the
previous word. `select_prev_word` detects this and takes an extra backward step:

```rust
let word_start = prev_word_start(buf, pos, is_boundary);
let word_end   = find_word_end_from(buf, word_start, is_boundary);

// pos is inside the current word — one more step back to reach the previous word.
if pos >= word_start && pos <= word_end {
    if word_start == 0 { return None; }  // already at the first word
    let prev_start = prev_word_start(buf, word_start, is_boundary);
    let prev_end   = find_word_end_from(buf, prev_start, is_boundary);
    return Some((prev_start, prev_end));
}
```

This means `b` always selects a *different* word — the one before the cursor —
whether the cursor is between words or inside one.

### `find_word_end_from` and multi-codepoint graphemes

`find_word_end_from` returns the position of the **last codepoint** of the
final grapheme cluster in the word. For single-codepoint graphemes (the common
case) this is the grapheme's only position. For a combining sequence like
`café` where `é` = `U+0065 + U+0301`:

```
c  a  f  e  ◌́
0  1  2  3  4
              ↑  find_word_end_from returns 4, not 3
```

Returning 3 (the base `e`) would leave `U+0301` outside the selection — an
orphaned combining mark. Returning 4 ensures `Selection::new(word_start, 4)`
covers the complete grapheme.

This interacts with `Selection::end_inclusive(buf)` in edit operations. Edit
code calls `end_inclusive` (which computes `next_grapheme_boundary(buf, end) - 1`)
instead of `end()` when building deletion ranges. For a selection built with
`find_word_end_from` (which already stored the last codepoint as `head`), this
is a no-op. For other selections (e.g. a text object that stopped at a
grapheme-start), `end_inclusive` extends to the full grapheme. Both paths
handle combining marks correctly.

### Two sets of word commands

Word motions appear in two flavours with different semantics:

| Command | Framework | Semantics |
|---------|-----------|-----------|
| `cmd_select_next_word` | `apply_word_select` | Fresh selection of the whole word |
| `cmd_extend_next_word_start` | `apply_motion` + `Extend` | Grow existing selection to next word start |

The select variants are hand-written — they call `apply_word_select` and
cannot be generated by the `motion_cmd!` macro, which generates wrappers
around `apply_motion`.

The extend variants are generated by `motion_cmd!` using `next_word_start` (a
position function) with `Extend` mode. They grow the current selection toward
the next word start — useful when building a larger selection across multiple
motions.

---

## CharClass: Word Boundaries and the Eol Split

### word vs WORD

Vim and Helix distinguish two kinds of "word":

- `word` (lowercase): a run of alphanumeric/underscore characters, a run of
  punctuation, or a run of whitespace. Any category change is a boundary.
- `WORD` (uppercase): a run of any non-whitespace characters. Only a
  whitespace boundary counts.

In `helpers.rs`, this is captured by `CharClass` and two boundary predicates:

```rust
pub enum CharClass { Word, Punctuation, Space, Eol }

// word: any class change is a boundary
fn is_word_boundary(a: CharClass, b: CharClass) -> bool { a != b }

// WORD: treat Punctuation as Word — only whitespace/Eol changes count
fn is_WORD_boundary(a: CharClass, b: CharClass) -> bool {
    let merge = |c| if c == Punctuation { Word } else { c };
    merge(a) != merge(b)
}
```

Text object and motion implementations take the boundary predicate as a
parameter (`impl Fn(CharClass, CharClass) -> bool`), so `inner_word_impl`
serves both `iw` and `iW` without duplication.

### Why Eol is its own class

`\n` could be treated as `Space` — after all, it is whitespace. But if it were,
`w` (move to next word start) would skip over newlines the same way it skips
spaces, meaning it could jump two logical lines in one keypress.

Helix stops `w` at newlines: moving forward from the last word on a line lands
on the `\n`, not on the first word of the next line. Making `Eol` a distinct
class in `CharClass` is what enforces this — the `\n` is always a class
boundary, so word-forward stops there.

---

## Inner vs Around: The Text Object Convention

Every text object in HUME comes in two flavours, following Vim/Helix convention:

- **`inner` (`i` prefix)**: the content *without* the delimiters. `i(` selects
  the text inside parentheses; `iw` selects the word without surrounding space.
- **`around` (`a` prefix)**: the content *including* the delimiters. `a(`
  selects the parentheses and their contents; `aw` selects the word plus one
  adjacent whitespace run.

In code, `inner_bracket` and `around_bracket` share `find_bracket_pair` to
locate the pair, then diverge on what range to return:

```rust
fn inner_bracket(buf, pos, open, close) -> Option<(usize, usize)> {
    let (open_pos, close_pos) = find_bracket_pair(buf, pos, open, close)?;
    Some((open_pos + 1, close_pos - 1))  // exclude the brackets
}

fn around_bracket(buf, pos, open, close) -> Option<(usize, usize)> {
    find_bracket_pair(buf, pos, open, close)  // include the brackets
}
```

The around-word rule is more nuanced: prefer including trailing whitespace
(so deleting `aw` leaves no double-space), fall back to leading whitespace if
there is no trailing space. This matches Vim's long-established behaviour.

---

## Quote Scanning: Parity Instead of Depth

### Why quotes are different from brackets

Brackets use distinct open and close characters (`(` vs `)`), which allows a
depth-tracking scan: increment depth on open, decrement on close, stop at
depth zero. This correctly handles nesting.

Quotes use the *same* character for open and close (`"..."`, `'...'`). There
is no depth to track, and nesting is ambiguous anyway (most languages don't
allow nested same-character quotes). A different algorithm is needed.

### The parity scan

`find_quote_pair` scans the current line and uses parity to assign roles:

```
Position:  0   1   2   3   4   5   6   7
           "   h   e   l   l   o   "   !
           ↑                   ↑
         odd (open)          even (close)
```

Every quote character found on the line alternates between "opening" (odd
occurrence) and "closing" (even occurrence). When a complete pair is found,
the algorithm checks whether `pos` falls inside it (`open_pos <= pos <= close_pos`).

```rust
let mut open: Option<usize> = None;
for i in line_start..line_end {
    if buf.char_at(i) == Some(quote) {
        match open {
            None        => open = Some(i),            // odd → opening
            Some(op)    => {                           // even → closing
                if op <= pos && pos <= i {
                    return Some((op, i));
                }
                open = None;                           // reset for next pair
            }
        }
    }
}
```

A cursor ON a quote character is handled by the same parity logic: whether
that quote is the opener or closer depends on how many quotes precede it on
the line. The scan resolves this automatically.

### Why brackets can span lines but quotes cannot

Bracket text objects walk the entire buffer and track depth — they work
correctly across lines because `(` and `)` are asymmetric. The scanner can
always tell which direction it is going.

The parity scan cannot be extended to multiple lines with the same correctness
guarantee. Consider a file where line 1 has an unmatched `"` (a string that
continues on the next line, or a comment, or a stray character). The parity
count is now off by one for every line that follows — every pair assignment
downstream is inverted, and the text object selects the wrong region or fails
silently.

This applies to all three quote characters, including backticks. The backtick
text object silently fails on multiline inline code spans in Markdown
(CommonMark allows `` `foo\nbar` ``).

### The planned fix: tree-sitter when available, parity fallback

Tree-sitter builds a syntax tree that distinguishes string literals from other
uses of quote characters. When a tree-sitter grammar is loaded, quote text
objects can query the tree for the enclosing string node — getting multiline
correctness and proper handling of escaped quotes for free.

When no grammar is loaded (plain text, unsupported language), the line-bounded
parity scan remains the fallback. It is fast and correct for the common case
of same-line pairs; the limitation is documented and visible rather than
silently wrong.

---

## Lifetimes: Borrowing Across Struct Boundaries

### The problem lifetimes solve

Rust's borrow checker guarantees that a reference never outlives the value it
points to. This is easy to enforce inside a single function — the compiler can
see both the value and the reference, and check that the reference is dropped
first.

The problem arises when you put a reference **inside a struct**:

```rust
struct DisplayLine {
    content: &RopeSlice,   // ← which RopeSlice? how long does this live?
}
```

The compiler has no way to know when `content` will be used, so it cannot
verify safety. The lifetime annotation `&'a` is the solution: it gives the
compiler a name to reason about.

### What `'a` means

`'a` is a **lifetime parameter** — a label that says "these references all
point into the same scope and will not outlive it." The `'a` is not a duration
or a timer; it is a constraint.

```rust
pub(crate) struct DisplayLine<'a> {
    pub content: RopeSlice<'a>,
    pub line_number: Option<usize>,
    pub char_offset: Option<usize>,
}
```

This declaration says: "`DisplayLine` borrows from some scope. All references
tagged `'a` must remain valid for at least as long as this `DisplayLine`
exists." When the `DisplayLine` is dropped, those borrows are released — and
the compiler verifies this statically.

The `'a` on the struct and the `'a` on each field are the **same label**. The
compiler unifies them: every `&'a T` field must be borrowed from a scope that
outlives the struct instance.

### How it looks at the call site

In `renderer.rs`, display lines are computed from the buffer for one frame:

```rust
let display_lines = view.display_lines(buf);
// display_lines borrows from buf — 'a is the lifetime of buf
for dl in &display_lines {
    render_gutter(screen_buf, view, colors, dl, cursor_line, x, y);
    // dl: &DisplayLine<'_> — borrow of buf is live here
}
// display_lines dropped here — borrow of buf ends
```

`buf` outlives the loop, so the compiler accepts this. If you tried to store
`display_lines` in a field of `Editor`, the compiler would reject it — the
borrow of `buf` would outlive the frame.

### Lifetime elision: when you don't see `'a`

Rust can infer lifetimes in simple cases and lets you omit them. Function
signatures are the main beneficiary:

```rust
// Written explicitly:
fn render_gutter<'a>(view: &'a ViewState, dl: &DisplayLine<'a>, ...) { ... }

// What you actually write (elision rules fill in the 'a):
fn render_gutter(view: &ViewState, dl: &DisplayLine<'_>, ...) { ... }
```

The `'_` is an anonymous lifetime — "some lifetime, inferred by the compiler."
It signals that a lifetime exists without naming it. You will see `'_` often
in HUME where the lifetime does not need to be shared across multiple
parameters.

### The rule of thumb

- **Named `'a`**: two or more places in the same signature need to share the
  same lifetime (e.g., a struct that holds multiple borrows from the same
  source).
- **`'_`**: one reference, one place, the compiler can figure it out.
- **No annotation**: a function that takes no references, or only owned values.

---

## The Command/Keymap/Dispatch Architecture

HUME's key handling is split across four files, each owning one responsibility.
Understanding the split — and what each layer does *not* know — is the key to
extending the editor safely.

### The four files

| File | Role | Knows about keys? | Knows about `&mut Editor`? |
|---|---|---|---|
| `registry.rs` | Name → function pointer | No | No |
| `commands.rs` | Editor-level command implementations | No | Yes (via `&mut Editor` param) |
| `keymap.rs` | Key sequence → command name | Yes | No |
| `mappings.rs` | Resolve name, call function | No | Yes (`&mut self`) |

**`registry.rs`** is the command registry. Every user-facing operation is a
named `MappableCommand` — a function pointer wrapped with a `&'static str` name
and a doc string. Four variants exist:

```rust
enum MappableCommand {
    Motion    { name, doc, fun: fn(&Buffer, SelectionSet, usize) -> SelectionSet },
    Selection { name, doc, fun: fn(&Buffer, SelectionSet) -> SelectionSet },
    Edit      { name, doc, fun: fn(Buffer, SelectionSet) -> (Buffer, SelectionSet, ChangeSet) },
    EditorCmd { name, doc, fun: fn(&mut Editor, usize) },
}
```

The first three are pure functions — they take buffer/selections and return new
ones. `EditorCmd` takes `&mut Editor` for composite operations that need mode
changes, registers, undo groups, or parameterized motions.

**`commands.rs`** holds the 35 `EditorCmd` implementations as free functions:
`cmd_change`, `cmd_find_forward`, `cmd_open_line_below`, etc. Each is a
`fn(&mut Editor, usize)` registered by name in the registry. This parallels
how `ops/motion.rs` holds pure motion functions.

**`keymap.rs`** is the key binding table. A trie maps key sequences to command
names via `KeymapCommand`:

```rust
struct KeymapCommand {
    name: &'static str,
    extend_name: Option<&'static str>,
}
```

The keymap stores *names*, not function pointers. This is what the Steel
scripting layer will rewrite to support user keymaps — and it can do so without
touching any execution logic.

**`mappings.rs`** is the glue. `execute_keymap_command` takes a name, looks it
up in the registry, and calls the function pointer:

```
keypress
  → keymap.rs  trie walk  →  KeymapCommand { name: "move-right", extend_name: Some("extend-right") }
  → mappings.rs            →  registry.get("move-right")  (or "extend-right" if extend mode is on)
  → registry.rs            →  MappableCommand::Motion { fun: cmd_move_right }
  → mappings.rs            →  fun(buf, sels, count)
```

### Extend-mode duality

Many Normal mode keys do different things depending on whether extend mode is
active. Instead of doubling the keymap, each binding can carry an
`extend_name`:

```rust
// h = move-left normally, extend-left in extend mode
cmd!("move-left", "extend-left")

// d = delete always (no extend variant)
cmd!("delete")
```

Resolution happens at dispatch time in `mappings.rs`. The keymap itself is
unaware of extend mode — it just stores two names.

For `WaitChar` commands (f/t/F/T/r), extend resolution is deferred further:
it happens when the *character argument* arrives, not when the trigger key is
pressed. This is because extend mode could toggle between the two keypresses.

### WaitChar: parameterized commands

Some commands need a character argument: `f` (find), `t` (till), `r` (replace).
The trie stores these as `WaitChar` nodes:

```rust
wait_char!("find-forward", "extend-find-forward")
```

When the trie walk hits a `WaitChar` node, `mappings.rs` stores the pending
command in `Editor.wait_char`. The *next* keypress is consumed as the character
argument, stored in `Editor.pending_char`, and the command is dispatched. The
command function (e.g. `cmd_find_forward` in `commands.rs`) reads the character
via `ed.pending_char.take()`.

### Commands are mode-agnostic

Commands in the registry have no mode affinity. `"flip-selections"` is just a
name that resolves to a function pointer. If Steel binds it to a key in the
insert keymap trie, `handle_insert` walks the trie, gets a `Leaf`, and calls
`execute_keymap_command` — which calls the function. The selection flips, the
editor stays in Insert mode.

Whether that binding is *useful* is the user's responsibility. The editor
doesn't second-guess it.

### Insert mode limitations

Normal mode accumulates multi-key sequences in `pending_keys` and handles
`WaitChar` state. Insert mode does neither — its trie walk is single-key only.
This means:

- **Multi-key sequences** (e.g. `mi` for inner-word) won't work in Insert mode.
- **WaitChar commands** (e.g. `find-forward`, which needs a second keypress for
  the character argument) will silently do nothing, because Insert mode doesn't
  set `wait_char` or consume the follow-up character.
- **Simple Leaf commands** (e.g. `flip-selections`, `collapse-selection`) work
  fine if bound to a single key in the insert trie.

This is a design constraint, not a bug. Insert mode is optimised for typing —
complex command sequences belong in Normal mode. If a future need arises for
multi-key insert bindings, the `pending_keys` / `WaitChar` machinery from
`handle_normal` would need to be replicated in `handle_insert`.

### Independence of layers

The layering means any of the four files can change independently:

- **New command**: add the function in the appropriate `ops/` file or
  `commands.rs`, register it in `registry.rs`, bind a key in `keymap.rs`.
  `mappings.rs` is unchanged.
- **Rebind a key**: only touch `keymap.rs`.
- **Change dispatch** (e.g. add macro recording): only touch `mappings.rs`.
- **User keymaps via Steel**: rewrite `keymap.rs` trie entries. The registry
  and dispatch layer are unaffected.
