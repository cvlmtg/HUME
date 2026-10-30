# Plugin Attribution: Who Owns What

When several plugins and the user's own `init.scm` all register commands,
HUME needs to know who owns each one: to refuse a conflicting definition, to
roll back what a failed plugin registered, and to answer
`(command-plugin name)`.

## Owners

Every piece of Scheme code HUME runs is credited to one of three owners:

- **HUME** itself: the runtime files that ship with the editor, and every
  built-in command written in Rust.
- **The user**: `init.scm`, and everything it defines.
- **A plugin**: a plugin's files, and everything they define.

A plugin is named in one of three forms:

- `core:<name>`, a plugin bundled with HUME (`core:plum`, `core:lsp`, …);
- `<user>/<repo>`, a third-party plugin, with a single `/`;
- `./<path>.scm`, a single file under `init.scm`'s folder, loaded as a
  plugin of its own.

Each part of a name must be non-empty and safe to use as a folder name: it
is not `.` or `..`, and holds none of `/`, `\`, `"`, `:` or NUL. Names compare
without regard to case, so `ALICE/TOOL` and `alice/tool` are the same plugin,
while the casing as written is kept for display and file paths.

## The attribution stack

Ownership is tracked by a stack of what is running, innermost on top. When
HUME starts evaluating some code, it knows whose code that is: `init.scm` is
the user's, a runtime file is HUME's. When a plugin body starts running, the
plugin is pushed onto the stack, and anything defined while it runs belongs
to it. A body that triggers a second plugin's load pushes that plugin on top;
the innermost one gets the credit. When a body finishes, it is popped.

Ownership is recorded with every procedure a script hands HUME: a command, a
hook handler, a callback. That procedure always runs as the code that defined
it, whoever causes it to run:

- A key or a `:` command runs the bound command as its author, not as the
  user who pressed the key.
- A `call!` from one command to another pushes the called command's author,
  so the called command runs as itself, not as its caller.
- A hook handler or a callback runs as whoever registered it.

Plugins never declare ownership; they run, and the stack credits them.

## Owner, and on whose behalf

Some things more than one party can set, such as key bindings, settings,
language definitions and language-server registrations, rank by who set
them: the user's beat a plugin's, and a plugin's beat HUME's own. That
ranking follows on whose behalf code runs, which is not always its owner. A
command the user runs by hand acts for the user, whoever wrote it, so a key
binding it makes ranks as the user's. Code inside a plugin body always acts
for the plugin.

## `(command-plugin name)`

`command-plugin` answers who owns a command:

```scheme
(command-plugin "move-right")   ; => "hume"            (built in)
(command-plugin "my-cmd")       ; => "alice/my-plugin" (defined by that plugin)
(command-plugin "setup-hook")   ; => "user"            (defined in init.scm)
```

A command defined by one of HUME's runtime files also answers `"hume"`, and so
does a name nothing defines.

A lazy plugin's manifest lists the commands it will provide. Each listed name
answers with the plugin from the moment the plugin is declared, before its
body runs. A listed command the body never defines stops existing once the
plugin loads, and from then on answers `"hume"` like any unknown name.

## Conflicts

A command name has one owner, and the first registration keeps it. There is
no shadowing.

**Declaring a lazy plugin.** `declare-plugin!` refuses any command name that
is already taken, by a built-in command, a defined command or another lazy
plugin's declaration, and any name it lists twice. A refused declaration
raises an error and records nothing. What that error stops depends on where
the declaration is:

- In `init.scm`, it is an ordinary error at the top level, and the rest of
  `init.scm` does not run.
- In a plugin's manifest, the whole manifest fails: every entry it had
  already declared is rolled back, and the plugin is marked failed.

**Defining a command in a plugin body.** When a body calls `define-command!`
for a name that is already taken, the call raises an error that aborts the
body. The only name a body may take over is its own placeholder, the one its
manifest listed. The plugin is marked failed, the commands, hooks and key
bindings it registered are rolled back, and the earlier command stays live.

## Load once

HUME does not unload or hot-reload individual plugins. Once a plugin body has
run, its commands, bindings and hooks stay until `:reload-config` rebuilds
everything from scratch by running `init.scm` again.

---

*See also: [Plugin Architecture](plugin-architecture.md) for how plugins load
and activate; [Runaway-Script Protection](runaway-script-protection.md) for
what a failed plugin leaves behind.*
