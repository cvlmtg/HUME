# The Command/Keymap/Dispatch Architecture

HUME's key handling is split across four files, each owning one responsibility.
Understanding the split — and what each layer does *not* know — is the key to
extending the editor safely.

## The four files

| File | Role | Knows about keys? | Knows about `&mut Editor`? |
|---|---|---|---|
| `registry.rs` | Name → function pointer lookup | No | No |
| `commands.rs` | Editor-level command implementations | No | Yes (via `&mut Editor` param) |
| `keymap.rs` | Key sequence → command name | Yes | No |
| `mappings.rs` | Resolve name, call function | No | Yes (`&mut self`) |

## Layer 1: Command Registry (`registry.rs`)

The registry is the single source of truth for what commands exist. Every
user-facing operation is a named `MappableCommand` — a function pointer wrapped
with metadata. Four variants exist:

```rust
enum MappableCommand {
    Motion      { name, fun: fn(&Buffer, SelectionSet, usize, MotionMode) -> SelectionSet },
    Selection   { name, fun: fn(&Buffer, SelectionSet, MotionMode) -> SelectionSet },
    Edit        { name, fun: fn(Buffer, SelectionSet) -> (Buffer, SelectionSet, ChangeSet), repeatable },
    EditorCmd   { name, fun: fn(&mut Editor, usize, MotionMode), repeatable, extendable },
    SteelBacked { name, doc, steel_proc: String, extendable },
}
```

The first three are pure functions — they take buffer/selections and return new
ones. `EditorCmd` takes `&mut Editor` for composite operations that need mode
changes, registers, undo groups, or parameterized motions.

All variants except `Edit` accept a `MotionMode` parameter. The dispatcher
passes `MotionMode::Move` or `MotionMode::Extend` depending on whether extend
mode is active. Commands that don't care about extend (e.g. `undo`, `quit`)
accept `_mode: MotionMode` and ignore it.

### Extend mode is a runtime parameter, not separate commands

There are no `"extend-left"` or `"extend-select-line"` commands. Each base
command (e.g. `"move-left"`, `"select-line"`) receives `MotionMode::Extend` at
dispatch time when the user is in extend mode. The command branches internally:

```rust
match mode {
    MotionMode::Move   => Selection::collapsed(new_head),  // re-anchor
    MotionMode::Extend => Selection::new(sel.anchor, new_head),  // keep anchor
}
```

This means adding a new motion requires **one function and one registration** —
extend support comes for free from the `MotionMode` parameter.

### The `extendable` flag

Motion and Selection commands are always extendable. Edit commands are never
extendable. EditorCmd and SteelBacked each have an explicit `extendable: bool`
flag set at registration time. This is used by the Ctrl+key guard (see below)
to decide whether a `Ctrl+letter` keypress should trigger extend behaviour.

For Steel-defined commands, use `(define-command-extend! name doc proc)` instead
of `(define-command! …)` to set `extendable = true`. Use this for composite
commands whose last step is a motion or selection — it preserves the `Ctrl+key`
one-shot extend behaviour for any bare letter you rebind to the command.

### Typed commands

The registry also holds typed commands — invoked from the `:` command line, not
from keybindings. They share the same namespace to prevent name collisions and
provide a single source for `:help`.

```rust
struct TypedCommand {
    name, doc, aliases, fun: fn(&mut Editor, Option<&str>, bool),
}
```

## Layer 2: Editor Commands (`commands.rs`)

This file holds the `EditorCmd` implementations as free functions:
`cmd_change`, `cmd_find_forward`, `cmd_open_line_below`, etc. Each is a
`fn(&mut Editor, usize, MotionMode)` registered by name in the registry. This
parallels how `ops/motion.rs` holds pure motion functions.

## Layer 3: Keymap (`keymap.rs`)

The keymap is a trie that maps key sequences to command names. Each binding
resolves to a `KeymapCommand`:

```rust
struct KeymapCommand {
    name: &'static str,
}
```

The keymap stores *names*, not function pointers. This is what the Steel
scripting layer will rewrite to support user keymaps — and it can do so without
touching any execution logic. A user remap is just `key → command-name`.

Three types of trie nodes exist:

- **Leaf**: a complete binding → dispatch the command name.
- **Interior**: more keys needed (e.g. `m` → `i` → `w` for inner-word).
- **WaitChar**: the next keypress is consumed as a character argument (f/t/F/T/r).

The `cmd!` and `wait_char!` macros construct bindings:

```rust
t.bind_leaf(key!('h'), cmd!("move-left"));
t.bind(key!('f'), wait_char!("find-forward"));
```

### Three keymaps

The `Keymap` struct holds three separate tries:

| Trie | Purpose |
|------|---------|
| `normal` | Main keymap for Normal mode |
| `extend` | Sparse overrides for Extend mode (checked first) |
| `insert` | Single-key bindings for Insert mode |

The **extend trie** is small — by default it only contains `o → flip-selections`
(mirrors Helix/Kakoune: `o` in extend mode flips anchor/head instead of opening
a new line). Any key not in the extend trie falls through to the normal trie
with extend mode active, which gives it `MotionMode::Extend` automatically.

This lets Steel customize per-key extend-mode overrides: "when in extend mode
and the user presses this key, run this different command instead."

## Layer 4: Dispatch (`mappings.rs`)

`mappings.rs` is the glue. `execute_keymap_command` takes a command name, an
`extend: bool` flag, and a count. It converts extend to `MotionMode` and calls
the right function pointer:

```
keypress
  → keymap.rs  trie walk  →  KeymapCommand { name: "move-right" }
  → mappings.rs            →  MotionMode = if extend { Extend } else { Move }
  → registry.rs            →  registry.get("move-right") → MappableCommand::Motion { fun }
  → mappings.rs            →  fun(buf, sels, count, MotionMode::Extend)
```

### How extend mode works

There are three ways a command gets `MotionMode::Extend`:

**1. Sticky extend mode.** The user presses `e` to enter Extend mode. All
subsequent commands run with `MotionMode::Extend` until mode is exited. The
extend trie is checked first for per-key overrides (like `o → flip-selections`).

**2. Ctrl+key one-shot extend (kitty keyboard protocol).** When kitty protocol
is enabled, pressing `Ctrl+l` strips the Control modifier, looks up `l` in the
normal trie → `"move-right"`, and dispatches with `MotionMode::Extend`. This
only works on kitty-capable terminals.

**3. Explicit Ctrl+key bindings (works on any terminal).** Some commands have
explicit `Ctrl+letter` bindings in the normal trie:

```rust
t.bind_leaf(key!('x'),        cmd!("select-line"));
t.bind_leaf(key!(Ctrl + 'x'), cmd!("select-line"));
```

Both keys bind to the same command name. The dispatch detects that it's a
`Ctrl+letter` key and the command is extendable, so it sets
`MotionMode::Extend` automatically. No separate extend command name needed.

This is useful for commands that should *always* extend when invoked via
`Ctrl+key`, regardless of terminal capabilities.

### Remapping for users (Steel)

To remap a command with its extend behaviour to a different key, a user binds
the base command name to both the bare key and the Ctrl variant:

```scheme
(keymap-bind! 'normal "f"      "select-line")    ; MotionMode::Move
(keymap-bind! 'normal "C-f"    "select-line")    ; MotionMode::Extend (auto)
```

The user only needs to know the base command name. The extend behaviour comes
from the dispatch layer — any extendable command bound to a `Ctrl+letter` key
automatically gets `MotionMode::Extend`.

### Ctrl+key guard

Not all `Ctrl+letter` keys should extend. `Ctrl+c` is force-quit, `Ctrl+u` is
undo — these must not silently become extend variants. The dispatch checks
`is_extendable()` on the resolved command:

- **Extendable** → dispatch with `MotionMode::Extend`
- **Not extendable** → dispatch normally (non-kitty explicit binding) or
  suppress entirely (kitty strip-CONTROL path)

This is why binding a bare letter to a plain `(define-command! …)` command
silently kills that letter's `Ctrl` variant: `SteelBacked.extendable` is
`false` by default, so the strip-CONTROL path suppresses it. Use
`(define-command-extend! …)` to opt in.

### WaitChar: parameterized commands

Some commands need a character argument: `f` (find), `t` (till), `r` (replace).
The trie stores these as `WaitChar` nodes:

```rust
wait_char!("find-forward")
```

When the trie walk hits a `WaitChar` node, `mappings.rs` stores the pending
command in `Editor.wait_char`. The *next* keypress is consumed as the character
argument, stored in `Editor.pending_char`, and the command is dispatched. The
command function (e.g. `cmd_find_forward` in `commands.rs`) reads the character
via `ed.pending_char.take()`.

Extend resolution for wait-char commands happens at **char-consumption time**
(not when the trigger key is pressed), since extend mode could toggle between
the two keypresses.

## Commands are mode-agnostic

Commands in the registry have no mode affinity. `"flip-selections"` is just a
name that resolves to a function pointer. If Steel binds it to a key in the
insert keymap trie, `handle_insert` walks the trie, gets a `Leaf`, and calls
`execute_keymap_command` — which calls the function. The selection flips, the
editor stays in Insert mode.

Whether that binding is *useful* is the user's responsibility. The editor
doesn't second-guess it.

## Insert mode limitations

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

## Independence of layers

The layering means any of the four files can change independently:

- **New command**: add the function in the appropriate `ops/` file or
  `commands.rs`, register it in `registry.rs`, bind a key in `keymap.rs`.
  `mappings.rs` is unchanged.
- **Rebind a key**: only touch `keymap.rs`.
- **Change dispatch** (e.g. add macro recording): only touch `mappings.rs`.
- **User keymaps via Steel**: rewrite `keymap.rs` trie entries. The registry
  and dispatch layer are unaffected.
