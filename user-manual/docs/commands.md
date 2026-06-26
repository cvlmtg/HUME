# Commands

Press `:` in Normal mode to open the command line. Type a command name and press `Enter` to run it. `Esc` dismisses without running. This page lists the built-in `:` commands; for launching HUME from a shell, see [Command-line Flags](cli.md).

Tab completion is available for command names and, where applicable, their arguments.

## File commands

| Command | Effect |
|---------|--------|
| `:e <path>` | Open file (or reload current file if no path given) |
| `:w` | Save |
| `:w <path>` | Save as |
| `:w!` | Force save (retry with chmod on permission errors) |
| `:wa` | Write all modified buffers to disk |
| `:q` | Quit |
| `:q!` | Force quit |
| `:wq` | Save and quit |
| `:qa` | Quit all buffers |

## Buffer commands

| Command | Aliases | Effect |
|---------|---------|--------|
| `:ls` | — | List buffers |
| `:b <name>` | — | Switch to buffer |
| `:bnext` | `:bn` | Next buffer |
| `:bprev` | `:bp` | Previous buffer |
| `:bd` | — | Close buffer (blocked if there are unsaved changes) |
| `:bd!` | — | Force close buffer (discard unsaved changes) |

## Panes

| Command | Aliases | Effect |
|---------|---------|--------|
| `:split` | `:sp` | Split the current pane horizontally *(not yet implemented)* |
| `:vsplit` | `:vsp` | Split the current pane vertically *(not yet implemented)* |

Pane focus uses the `Ctrl+p` prefix (`Ctrl+p h`/`j`/`k`/`l`/`w`) — see the [Key Reference](key-reference.md).

## Settings

| Command | Effect |
|---------|--------|
| `:set <option>` | Show current value of an option |
| `:set global <option>=<value>` | Set a global option |
| `:set buffer <option>=<value>` | Set an option for the current buffer only |

See [Configuration](configuration.md) for all available options.

## Display

| Command | Aliases | Effect |
|---------|---------|--------|
| `:theme <name>` | — | Load a theme by name; no arg shows current |
| `:theme-debug` | — | Show resolved styles for key UI scopes |
| `:toggle-soft-wrap` | `:wrap` | Toggle soft line wrapping |
| `:messages` | `:mes` | Show message log in a read-only buffer |
| `:clear-search` | — | Clear search highlights (also clears automatically on `Esc`) |

Search highlights clear automatically when you press `Esc`. `:clear-search` clears them on demand.

## Navigation

| Command | Effect |
|---------|--------|
| `:cd <path>` | Change the working directory |
| `:pwd` | Print the current working directory |

## Plugins

| Command | Aliases | Effect |
|---------|---------|--------|
| `:plugin-status` | `:plugins` | Show declared plugins and their load state |
| `:reload-config` | — | Reload `init.scm` from scratch |

The `:plum-*` commands (plugin and grammar installation, updates, cleanup) are provided by the bundled `core:plum` plugin — see [Plugins](plugins.md#plum-plugin-and-grammar-management) for the full list.

## Other

| Command | Aliases | Effect |
|---------|---------|--------|
| `:tutor` | — | Open the interactive tutorial |
| `:version` | `:ver` | Show editor version |

## Discovering commands

There is no `:commands` listing command. To discover available commands, open the command line with `:` and press `Tab` — completion lists every registered name and alias. The list reflects both built-in commands and stubs registered by lazy plugins (see [Plugins](plugins.md)).

## Mappable commands from the command line

In addition to the typed commands above, **any mappable editor command** can be invoked from `:`. Commands like `:clear-search`, `:undo`, `:redo`, and `:select-all-matches` work this way without dedicated typed-command wrappers. Mappable commands take no aliases and accept an implicit count of 1.