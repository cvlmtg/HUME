<div align="center">
  <img src="/HUME-logo.svg" alt="HUME logo" width="200" />
</div>

## The pragmatic modal editor

**A modal editor for the terminal, extensible in Scheme.**

HUME follows the select-then-act model: first select what you want to change, then say what to do with it. On top of that, it is extensible thanks to its embedded Scheme interpreter and runs on *Windows*, *macOS* and *Linux*.

<img src="/HUME.png" alt="HUME editing a source file" />

## Why try it

- **Extensible to the core.** Fuzzy pickers, language server support, live git diff and the undo history graph are plugins written in Scheme, the same language as your config. They ship with the editor: enable them with one line, customize them, or rewrite them to your taste.
- **Installs from inside the editor.** Language servers, tree-sitter grammars, themes and plugins: each is a single command away, no manual downloads.
- **See it before you change it.** Selections come first, so every edit starts from something visible on screen. Common changes take fewer keys and cause fewer surprises.
- **Paste that does the obvious thing.** `p` reaches for what you most likely meant, and a kill ring keeps your recent deletes and yanks within reach.
- **Comfortable with real text.** Emoji, accents and other multi-byte characters count as single characters.
- **Easy to pick up.** HUME ships with an interactive `:tutor` and the manual teaches you how to migrate from your editor.

## Quickstart

1. **Install.** Download the archive for your platform and extract it. See [Installation](./installation.md).
2. **Turn on the extras.** HUME starts lean. Copy the [starter config](configuration.md#example-init-scm) to `~/.config/hume/init.scm` (`%APPDATA%\hume\init.scm` on Windows) to turn on the fuzzy pickers, language servers, live git diff, the undo-history graph and the plugin manager.
3. **Add your languages.** Open a file and run `:plum-install-grammar` for syntax highlighting and `:lsp-install` for a language server. Both act on the open file's language, once per language. Both need [the plugins from step 2](./core-plugins.md).
4. **Learn the keys.** Type `:tutor` and press `Enter` for the interactive tutorial.

The question every modal editor must answer up front: **to quit, press `Esc`, then type `:q` and press `Enter`.** Use `:q!` to quit without saving, `:wq` to save and quit.

## Where to go next

- New to modal editors? Start with [Getting Started](./getting-started.md) and [Modes](./modes.md).
- Coming from Vim or Neovim? See [Coming From Vim](./from-vim.md).
- Coming from Helix? See [Coming From Helix](./from-helix.md).
- Coming from Kakoune? See [Coming From Kakoune](./from-kakoune.md).
- Looking for a specific key? Jump to the [Default Keys](./default-keys.md).

## Credits

- [Helix](https://helix-editor.com): the language definitions, the language servers each language uses and their default settings, and the tree-sitter queries behind syntax highlighting, indentation and text objects.
- [Mason](https://github.com/mason-org/mason-registry): the registry `:lsp-install` uses to download and install language servers.
