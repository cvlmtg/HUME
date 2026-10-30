# Themes

HUME colors its interface and your code from a theme: a TOML file in the Helix theme format. Set the active one with the `theme` option (see [Settings](settings.md#global-options)) or `:theme <name>`.

```scheme
(set-option! "theme" "sand")
```

To see which themes are available, type `:theme ` and press `Tab`.

## Bundled themes

HUME ships 35 themes. `sand` is the default.

| Family | Themes |
|--------|--------|
| Catppuccin | `catppuccin_frappe`, `catppuccin_latte`, `catppuccin_macchiato`, `catppuccin_mocha` |
| Dracula | `dracula` |
| GitHub | `github_dark`, `github_dark_colorblind`, `github_dark_dimmed`, `github_dark_high_contrast`, `github_dark_tritanopia`, `github_light`, `github_light_colorblind`, `github_light_high_contrast`, `github_light_tritanopia` |
| Gruvbox | `gruvbox`, `gruvbox_light` |
| Kanagawa | `kanagawa`, `kanagawa-dragon`, `kanagawa-lotus` |
| Nord | `nord`, `nord-night`, `nord_light` |
| One Dark | `onedark`, `onedarker`, `onelight` |
| Rosé Pine | `rose_pine`, `rose_pine_dawn`, `rose_pine_moon` |
| Sand | `sand` |
| Solarized | `solarized_dark`, `solarized_light` |
| Tokyo Night | `tokyonight_day`, `tokyonight_moon`, `tokyonight_night`, `tokyonight_storm` |

To change a bundled theme, copy its file into the `themes/` subdirectory of your config directory and edit the copy there: a theme in your config directory wins over a bundled one of the same name. HUME falls back to a built-in theme when no `sand` theme is found or when the theme set in `init.scm` fails to load.

## Where themes live

Hand-authored themes go in the `themes/` subdirectory of your HUME config directory, alongside `init.scm`. A theme installed by a tool goes in the `themes/` subdirectory of your HUME data directory instead (see [File locations](configuration.md#file-locations)); a config-dir theme of the same name wins.

## Theme files

Most Helix themes load and render unchanged. A scope can be written as a flat key (`"ui.cursor" = { fg = "..." }`) or as a TOML section header (`[ui.cursor]` / `fg = "..."`). HUME treats the two the same, but Helix reads only the flat form, so a section-header theme won't work in Helix. The theme editor below exports the flat form.

A color can be a hex literal, a palette name you define, or one of the sixteen terminal color names Helix themes use (`red`, `light-gray`, and so on). The names resolve to fixed colors from the standard terminal palette, not to whatever your terminal has those colors set to. A color value in any other form leaves that one entry unstyled instead of failing the whole load, and `:messages` names it.

HUME has no rainbow-bracket highlighting, so it ignores a Helix theme's top-level `rainbow` array. The rest of the theme still loads, and `:messages` reports the entry like any other entry HUME couldn't use.

A theme fails to load outright only when the problem is with the document rather than one entry in it:

- invalid TOML syntax;
- an `inherits` parent that doesn't exist, forms a cycle, or nests more than eight deep;
- an `inherits`/`palette` key that isn't a string/table.

Loading then keeps your current theme.

## Installing themes

To install a third-party theme repository, run `:plum-install-theme <user/repo>` (see [Core Plugins → core:plum](core-plugins.md#core-plum)), for example:

```
:plum-install-theme cvlmtg/everforest.hume
```

::: info
[cvlmtg/everforest.hume](https://github.com/cvlmtg/everforest.hume) is Everforest, ported from Helix: a green-based, low-contrast color scheme inspired by forest colors in fall.
:::

`:theme <Tab>` picks it up right away, with no restart.

A theme editor is available online: a single-file HTML tool you download and open in a browser to edit themes visually and export them as TOML: https://raw.githubusercontent.com/cvlmtg/HUME/main/tools/theme-editor/index.html

## Theme scopes

A scope not listed below behaves as [Helix's own theme reference](https://docs.helix-editor.com/themes.html) describes it. Every syntax-highlighting scope works this way, so the part of a theme that colors your code carries over as-is.

### Scopes HUME adds

These have no Helix equivalent:

- `ui.cursor.match.search`: every visible search match, falling back to `ui.cursor.match` when unset
- `ui.popup.scroll`: scrollbar thumb on a scrolled hover popup (Helix only themes a scrollbar for `ui.menu`)
- `ui.window.focused`: seam divider segments next to the focused pane, falling back to `ui.window`
- `ui.drawer`: background and border of the bottom drawer, the list panel that shows diagnostics, references and similar lists
- `ui.tabline` / `ui.tabline.active`: the tab bar's row and its active tab. An unset `.active` falls back to the base `ui.tabline` style, unlike the statusline separator below
- `ui.statusline.search` / `.command` / `.sift`: one more mode-tinted statusline scope per HUME mode Helix doesn't have, alongside Helix's own `ui.statusline.normal`/`.insert`/`.select` (`.select` colors Extend mode, HUME's name for what Helix calls Select mode; `.sift` colors Sift mode, the `s` regex prompt)
- `ui.virtual.invisible`: the `<200b>`-style stand-in for a character the terminal must not be shown as itself (see [Buffer options](settings.md#buffer-options))
- `diff.plus.line` / `diff.minus.line` / `diff.delta.line`: the whole-line background tint `core:git-diff` paints for an added, deleted, or changed line (`diff.minus.line` also colors the text of a deleted line). Left undefined, they paint nothing: an unmodified Helix theme colors the gutter marker (through `diff.plus`, `diff.minus` and `diff.delta`) but paints no line tint
- `diff.plus.word` / `diff.minus.word`: word-level highlight inside a changed line (`core:git-diff`'s inline diff), inside the line-level `.line` tint above
- `diagnostic.error.message` / `.warning.message` / `.info.message` / `.hint.message` and their `.message-text` counterparts: the `:messages` log's severity badge and body text
- `error.diagnostic.inline` / `warning.diagnostic.inline` / `info.diagnostic.inline` / `hint.diagnostic.inline`: the diagnostic summary shown at the end of an offending line. Separate from `diagnostic.error` and friends, which style the squiggle under the code itself, so the summary doesn't pick up that scope's underline. Each falls back to the matching `error`/`warning`/`info`/`hint` gutter color when unset

### Helix scopes HUME doesn't read

Declaring any of these has no effect.

HUME doesn't have the feature these color:

- debugger (DAP) support: `ui.debug`, `ui.debug.breakpoint`, `ui.debug.active`
- which-key-style prompts: `ui.popup.info`, `ui.help`, `ui.text.info`
- picker preview: `ui.highlight`, `ui.highlight.frameline`
- a cursor-column highlight: `ui.cursorcolumn`, `ui.cursorcolumn.primary`, `ui.cursorcolumn.secondary`
- per-kind completion-entry styling: `ui.text.directory`, `ui.text.symlink`
- LSP deprecated/unnecessary diagnostic tags: `diagnostic.deprecated`, `diagnostic.unnecessary`
- move- or conflict-specific diff styling: `diff.delta.moved`, `diff.delta.conflict`
- snippets: `tabstop`
- ruler columns, the soft-wrap indicator, virtual jump labels, and per-kind inlay hints: `ui.virtual.ruler`, `ui.virtual.wrap`, `ui.virtual.jump-label`, `ui.virtual.inlay-hint.parameter`, `ui.virtual.inlay-hint.type`

HUME's interface works differently for these, so put the color in the listed alternative instead:

- `ui.picker.header`, `ui.picker.header.column`, `ui.picker.header.column.active`: HUME's picker has no column headers to style
- `ui.gutter`, `ui.gutter.selected`: the gutter takes no background of its own; style it with `ui.linenr` and `ui.linenr.selected`
- `ui.statusline.inactive`, `ui.text.inactive`: HUME tints the whole statusline row by mode rather than dimming an unfocused one, and dims an unfocused pane as a whole instead of theming an inactive state
- `ui.cursorline.secondary`: only the primary selection's line is tinted, via `ui.cursorline.primary`
- `ui.background.separator`: HUME's prompt line has no separator rule beneath it
- `ui.bufferline.background`: the tab bar's background comes from `ui.tabline` (or `ui.bufferline`, see below), not a separate background layer

### Scopes HUME reads differently

`ui.window` is the seam between split panes. HUME reads that scope's foreground for the divider glyph. A theme that sets only a background gets the theme's base text color (`ui.text`) for it, the same fallback every other undecorated element uses.

`ui.statusline.separator` divides the statusline's segments. HUME tints the whole statusline row by mode, so an undefined separator takes the row's current color rather than the untinted `ui.statusline`. A separator you set explicitly wins, in every mode.

HUME's tab bar shows one tab per saved pane layout, not one per buffer, so it has its own `ui.tabline` / `ui.tabline.active` scopes rather than Helix's `ui.bufferline` / `ui.bufferline.active`. A theme that sets only the latter (every Helix theme) still renders correctly: `ui.tabline` falls back to `ui.bufferline` when unset, and `ui.tabline.active` to `ui.bufferline.active`.
