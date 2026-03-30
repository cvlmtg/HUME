# The Command/Keymap/Dispatch Architecture

HUME's key handling is split across four files, each owning one responsibility.
Understanding the split — and what each layer does *not* know — is the key to
extending the editor safely.

## The four files

| File | Role | Knows about keys? | Knows about `&mut Editor`? |
|---|---|---|---|
| `registry.rs` | Name → function pointer | No | No |
| `commands.rs` | Editor-level command implementations | No | Yes (via `&mut Editor` param) |
| `keymap.rs` | Key sequence → command name | Yes | No |
| `mappings.rs` | Resolve name, call function | No | Yes (`&mut self`) |

**`registry.rs`** is the command registry. Every user-facing operation is a
named `MappableCommand` — a function pointer wrapped with a `&'static str` name.
Four variants exist:

```rust
enum MappableCommand {
    Motion    { name, fun: fn(&Buffer, SelectionSet, usize) -> SelectionSet },
    Selection { name, fun: fn(&Buffer, SelectionSet) -> SelectionSet },
    Edit      { name, fun: fn(Buffer, SelectionSet) -> (Buffer, SelectionSet, ChangeSet) },
    EditorCmd { name, fun: fn(&mut Editor, usize) },
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

## Extend-mode duality

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

## WaitChar: parameterized commands

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
