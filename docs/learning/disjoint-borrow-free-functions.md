# Disjoint Borrows and Free Functions

## The problem

`Editor` is a flat struct with ~35 fields. Many command bodies need to read one
field while mutating another. That's fine in principle — Rust allows simultaneous
`&T` and `&mut U` as long as they point to separate memory.

The trouble arises with method calls. When you write `self.do_thing()`, the
borrow checker sees `&mut self` — a mutable borrow of the **entire** struct.
Any other borrow of `self` in scope at that point, even of a completely
unrelated field, causes a compile error.

A typical symptom looks like this:

```rust
// Inside some &mut self method:
let values = self.kill_ring.head();  // borrows self.kill_ring
self.doc_edit_grouped(|b, s| paste(b, s, values));  // ERROR: &mut self conflicts
```

The workaround the codebase used to reach for was `.clone()`:

```rust
let values = self.kill_ring.head().to_vec();  // clone to end the borrow
self.doc_edit_grouped(|b, s| paste(b, s, &values));  // now compiles
```

Repeated across all command bodies, these clones added up to ~52 allocations
per common keystroke.

## The rule

> **Inside a `&mut self` method, never call another `&mut self` method.**
> Read and write fields directly. Delegate logic to free functions that take
> only the fields they need.

Rust's **Non-Lexical Lifetimes (NLL)** can prove that two named fields are
disjoint. It cannot do that through a method boundary — the method signature
`&mut self` hides which fields are actually touched.

## The pattern

Convert facility methods to free functions:

```rust
// Before — method on Editor:
fn doc_edit_grouped(&mut self, cmd: impl FnOnce(Text, SelectionSet) -> R) {
    let pbs = &mut self.pane_state[self.focused_pane_id][bid];
    let sels = pbs.selections.clone();  // clone because &mut self is locked
    // ...
}

// After — free function:
pub(crate) fn apply_doc_edit_grouped(
    buffers: &mut BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    focused_pane_id: PaneId,
    buf_id: BufferId,
    cmd: impl FnOnce(Text, SelectionSet) -> R,
) {
    let pbs = &mut pane_state[focused_pane_id][buf_id];
    let sels = std::mem::take(&mut pbs.selections);  // no clone — mem::take
    // ...
}
```

The caller passes disjoint `Editor` fields as separate parameters:

```rust
// No clone needed — kill_ring and buffers/pane_state are separate fields:
let values = self.kill_ring.head();
doc_ops::apply_doc_edit_grouped(&mut self.buffers, &mut self.pane_state, pid, bid, |b, s| {
    paste(b, s, values)
});
```

## `mem::take` instead of `.clone()`

When a value needs to be moved out of a struct to be transformed and written
back, `std::mem::take` avoids the clone:

```rust
// Clone (allocates a copy of the whole SelectionSet):
let sels = pbs.selections.clone();
pbs.selections = transform(sels);

// mem::take (moves out, leaves Default behind, no allocation):
let sels = std::mem::take(&mut pbs.selections);
pbs.selections = transform(sels);
```

`SelectionSet::default()` is minimal-valid (cursor at position 0) — designed
for exactly this transient-empty-then-overwrite pattern.

## Disjoint field borrows directly

When two fields are separate, you can hold an immutable reference to one while
mutably writing to the other — no clone needed at all:

```rust
// pane_transient and pane_state are different fields on Editor.
// The borrow checker allows both simultaneously:
if let Some(sels) = self.pane_transient[pid].pre_search_sels.as_ref() {
    self.pane_state[pid][bid].selections = sels.clone();
    //   ^^^^^^^^^^^^^^^^                  ^^^^^
    //   mut borrow of pane_state          shared borrow of pane_transient — OK!
}
```

The old code needed `let sels = sels.clone()` + a separate `set_current_selections()`
call because `set_current_selections` took `&mut self`, locking the whole struct.
With direct field writes, no method boundary blocks the borrow proof.

## Thin delegators

The `impl Editor` method blocks don't disappear entirely — the entry points
(`handle_key`, `execute_keymap_command`, etc.) still take `&mut Editor`. They
become thin delegators:

```rust
fn begin_edit_group_current(&mut self) {
    let pane_id = self.focused_pane_id;
    let buf_id  = self.focused_buffer_id();
    doc_ops::begin_edit_group(&self.buffers, &mut self.pane_state, pane_id, buf_id);
}
```

These are fine because they don't call other `&mut self` methods — they call
free functions immediately.

## Enforcement

`editor/src/core/lints.rs` contains `no_self_mut_method_calls_in_editor_module`,
a `cargo test` lint that scans the command/dispatch source files for calls to
denylisted `&mut self` methods. New facility methods are added to the denylist
before migration; `migrated: true` is set once the method is deleted, causing
the lint to also verify the definition doesn't reappear.
