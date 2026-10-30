<div align="center">
  <img src="/HUME-logo.svg" alt="HUME logo" width="200" />
</div>

## The pragmatic modal editor.

Zero friction, maximum output. **HUME** rests on a simple premise: the common case should be the short case, and the classic papercuts of text editing should be designed out from the start.

**HUME** is a modal text editor for the terminal. It follows the select-then-act model: first select what you want to change, then say what to do with it.

This manual takes you from first launch to advanced configuration. No experience with modal editors is required.

::: warning Early stage
HUME is at an early stage: some features are missing and there may be rough edges.
:::

<img src="/HUME.png" alt="HUME editing a source file" />

## Why try it
- **Less typing for what you do most**. Selections come first, so acting on text is short and direct.
- **Usable immediately, yours to shape**. Sensible defaults on day one; customize everything in a single language when you're ready.
- **Bundled, one line away**. Fuzzy pickers, language servers, live git diff, and 300+ language definitions ship with the editor, each one line of config away from being on.
- **Paste that does the obvious thing**. `p` reaches for what you most likely meant, and a kill ring keeps your recent deletes and yanks within reach.
- **Comfortable with real text**. Emoji, accents and other multi-byte characters count as single characters.

## Quickstart

1. **Install.** Download the archive for your platform and extract it. See [Installation](installation.md).
2. **Turn on the extras.** HUME starts lean. Copy the [starter config](configuration.md#example-init-scm) to `~/.config/hume/init.scm` (`%APPDATA%\hume\init.scm` on Windows) to turn on the fuzzy pickers, language servers, live git diff, the undo-history graph and the plugin manager.
3. **Add your languages.** Open a file and run `:plum-install-grammar` for syntax highlighting and `:lsp-install` for a language server. Both act on the open file's language, once per language. Both need [the plugins from step 2](core-plugins.md), and highlighting needs a few [tools on your `PATH`](syntax-highlighting.md#prerequisites).
4. **Learn the keys.** Type `:tutor` and press `Enter` for the interactive tutorial.

The question every modal editor must answer up front: **to quit, press `Esc`, then type `:q` and press `Enter`.** Use `:q!` to quit without saving, `:wq` to save and quit.

## Where to go next

- New to modal editors? Start with [Getting Started](getting-started.md) and [Modes](modes.md).
- Coming from Vim or Neovim? See [Coming From Vim](from-vim.md).
- Coming from Helix? See [Coming From Helix](from-helix.md).
- Coming from Kakoune? See [Coming From Kakoune](from-kakoune.md).
- Looking for a specific key? Jump to the [Default Keys](default-keys.md).
