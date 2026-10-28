# Themes

HUME colors its interface and your code from a theme: a TOML file in the Helix theme format. Set the active one with the `theme` option (see [Configuration](settings.md#global-options)) or `:theme <name>`.

```scheme
(set-option! "theme" "sand")
```

To see which themes are available, type `:theme ` and press `Tab`.

HUME reads `sand` from the `themes/` directories like any other theme, so editing your copy changes what you see. HUME falls back to a built-in theme when no `sand` theme is found or when the theme set in `init.scm` fails to load.

Hand-authored themes go in the `themes/` subdirectory of your HUME config directory, alongside `init.scm`. A theme installed by a tool goes in the `themes/` subdirectory of your HUME data directory instead (see [File locations](configuration.md#file-locations)); a config-dir theme of the same name wins.

HUME aims to support Helix themes as they are written. It isn't there in every detail, but it is close: most Helix themes load and render unchanged. A scope can be written as a flat key (`"ui.cursor" = { fg = "..." }`) or as a TOML section header (`[ui.cursor]` / `fg = "..."`). HUME treats the two the same, but Helix reads only the flat form, so a section-header theme won't travel back. The theme editor below exports the flat form.

A color can be a hex literal, a palette name you define, or one of the sixteen terminal color names Helix themes use (`red`, `light-gray`, and so on). The names resolve to fixed colors from the standard terminal palette, not to whatever your terminal has those colors set to, so a theme looks the same everywhere and unfocused-pane dimming has a real color to blend toward. A color value in any other form leaves that one entry unstyled instead of failing the whole load, and `:messages` names it.

HUME doesn't support a Helix theme's top-level `rainbow` array, since it has no rainbow-bracket highlighting to drive. The rest of the theme still loads, and `:messages` reports the entry like any other entry HUME couldn't use.

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
[cvlmtg/everforest.hume](https://github.com/cvlmtg/everforest.hume) is Everforest, ported from Helix: a green-based, low-contrast color scheme designed to feel warm and comfortable on the eyes, inspired by forest colors in fall.
:::

`:theme <Tab>` picks it up right away, no restart needed.

A theme editor is available online: a single-file HTML tool you download and open in a browser to edit themes visually and export them as TOML: https://raw.githubusercontent.com/cvlmtg/HUME/main/tools/theme-editor/index.html

## Theme scopes

A scope not listed below behaves as
[Helix's own theme reference](https://docs.helix-editor.com/themes.html) describes it.
Every syntax-highlighting scope works this way, so the part of a theme that colors your
code carries over as-is.

### Scopes HUME adds

These have no Helix equivalent:

- `ui.cursor.match.search`: every visible search match, falling back to
  `ui.cursor.match` when unset
- `ui.popup.scroll`: scrollbar thumb on a scrolled hover popup (Helix only themes a
  scrollbar for `ui.menu`)
- `ui.window.focused`: seam divider segments next to the focused pane, falling back
  to `ui.window`
- `ui.drawer`: background and border of the bottom drawer (`show-drawer-list!`), a
  generic pick-list panel Helix doesn't have
- `ui.tabline` / `ui.tabline.active`: the tab bar's row and its active tab. An unset
  `.active` falls back to the base `ui.tabline` style, unlike the statusline separator below
- `ui.statusline.search` / `.command` / `.sift`: one more mode-tinted statusline scope
  per HUME mode Helix doesn't have, alongside Helix's own
  `ui.statusline.normal`/`.insert`/`.select` (`.select` colors **Extend**, HUME's name for
  what Helix calls Select mode; `.sift` colors HUME's own Sift mode, the `s` regex prompt)
- `ui.virtual.invisible`: the `<200b>`-style stand-in for a character the terminal must
  not be shown as itself (see [Buffer options](settings.md#buffer-options))
- `diff.plus.line` / `diff.minus.line` / `diff.delta.line`: the whole-line background tint
  `core:git-diff` paints for an added, deleted, or changed line (`diff.minus.line` also
  colors the ghost text of a deleted line, since nothing is left in the buffer to color).
  Left undefined, they paint nothing: an unmodified Helix theme colors the gutter marker
  (through `diff.plus`, `diff.minus` and `diff.delta`) but paints no line tint. This is intended, not a bug
- `diff.plus.word` / `diff.minus.word`: word-level highlight inside a changed line
  (`core:git-diff`'s inline diff), inside the line-level `.line` tint above
- `diagnostic.error.message` / `.warning.message` / `.info.message` / `.hint.message` and
  their `.message-text` counterparts: the `:messages` log's severity badge and body text,
  a HUME-only feature
- `error.diagnostic.inline` / `warning.diagnostic.inline` / `info.diagnostic.inline` /
  `hint.diagnostic.inline`: the diagnostic summary shown at the end of an offending line.
  Separate from `diagnostic.error` and friends, which style the squiggle under the code
  itself, so the summary doesn't pick up that scope's underline. Each falls back to the
  matching `error`/`warning`/`info`/`hint` gutter color when unset

### Helix scopes HUME doesn't read

Declaring any of these has no effect today. They fall into two groups, which matters if
you're deciding whether to keep them in a theme you maintain.

**Waiting on a feature.** HUME doesn't have the thing these color yet. When it does, these
scopes are the natural way to theme it, so leaving them in a theme costs nothing:

- No debugger (DAP) support: `ui.debug`, `ui.debug.breakpoint`, `ui.debug.active`
- No which-key-style prompts: `ui.popup.info`, `ui.help`, `ui.text.info`
- No picker-preview highlighting: `ui.highlight`, `ui.highlight.frameline`
- No cursor-column ruler: `ui.cursorcolumn`, `ui.cursorcolumn.primary`,
  `ui.cursorcolumn.secondary`
- No per-kind completion-entry styling: `ui.text.directory`, `ui.text.symlink`
- No LSP deprecated/unnecessary diagnostic tags: `diagnostic.deprecated`,
  `diagnostic.unnecessary`
- No move- or conflict-specific diff styling: `diff.delta.moved`, `diff.delta.conflict`
- No snippet support: `tabstop`
- Ruler columns, the soft-wrap indicator, virtual jump labels, and per-kind inlay hints:
  `ui.virtual.ruler`, `ui.virtual.wrap`, `ui.virtual.jump-label`,
  `ui.virtual.inlay-hint.parameter`, `ui.virtual.inlay-hint.type`

**HUME's interface works differently.** These color a piece of Helix's UI that HUME
either doesn't present the same way or styles from another scope. They may never apply, so put
the color in the listed alternative instead:

- `ui.picker.header`, `ui.picker.header.column`, `ui.picker.header.column.active`: HUME's
  picker has no column headers to style
- `ui.gutter`, `ui.gutter.selected`: the gutter takes no background of its own; style it
  with `ui.linenr` and `ui.linenr.selected`
- `ui.statusline.inactive`, `ui.text.inactive`: HUME tints the whole statusline row by
  mode rather than dimming an unfocused one, and dims an unfocused pane wholesale instead
  of theming an inactive state
- `ui.cursorline.secondary`: only the primary selection's line is tinted, via
  `ui.cursorline.primary`
- `ui.background.separator`: HUME's prompt line has no separator rule beneath it
- `ui.bufferline.background`: the tab bar's ground comes from `ui.tabline` (or
  `ui.bufferline`, see below), not a separate background layer

### Scopes HUME reads differently

`ui.window` is the seam between split panes. HUME draws the divider glyph itself, so it
reads that scope's foreground. A theme that sets only a background gets the theme's base
text color (`ui.text`) for it, the same fallback every other undecorated element uses.

`ui.statusline.separator` divides the statusline's segments. HUME tints the whole
statusline row by mode, so an undefined separator takes the row's current color rather
than the untinted `ui.statusline`, which would show through a mode-tinted row as a
stripe of the wrong color. A separator you set explicitly wins, in every mode.

HUME's tab bar is a saved window layout per tab (Vim's tab pages), not a per-buffer strip,
so it has its own `ui.tabline` / `ui.tabline.active` scopes rather than Helix's
`ui.bufferline` / `ui.bufferline.active`. A theme that sets only the latter (every Helix
theme, since `ui.tabline` is HUME's own addition) still renders correctly: `ui.tabline`
falls back to `ui.bufferline` when unset, and `ui.tabline.active` to `ui.bufferline.active`.
A ported theme's colors carry over without naming HUME's scopes at all.
