# Helix Theme Editor

React app for building Helix-format theme TOML files, previewed live against a mock editor pane.

`index.html` at the top of this directory is **generated output** — edit files under `src/` instead, then rebuild. It's committed as a single self-contained file because the user manual links to it directly as a standalone download.

```sh
npm install
npm run dev     # local dev server with HMR
npm run build   # rebuilds tools/theme-editor/index.html
npm test        # runs tests/*.test.js against the pure-logic modules
```

## Known limitations

- Triple-quoted (multi-line) TOML strings (`"""..."""`, `'''...'''`) aren't supported by the parser.
- Scopes outside the catalog in `src/data.js` (e.g. from an imported theme with its own invented scopes) are preserved on export but aren't editable in the UI. The catalog is meant to track what HUME resolves, but it's maintained by hand — `tests/coverage.test.js` only checks that every catalog entry is rendered somewhere in the preview and vice versa, never that the catalog matches HUME's own scope names.
- Exporting while the "import the parent" banner is still up (an `inherits` child was imported alone) writes just that child's overrides plus `inherits`, not a flattened theme — import the named parent first if you want a self-contained export.
- The preview can still show a different colour than HUME actually renders for a value that is neither a palette name nor one of the sixteen ANSI names `resolveColor` (`src/lib/theme.js`) knows. Such a value passes straight through to CSS, so a genuinely unsupported name (Helix accepts none beyond those sixteen, but a hand-edited theme could contain anything) previews as that CSS named colour — but HUME's loader leaves the same entry unstyled and logs a warning instead of colouring it. A top-level `rainbow` array is preserved and re-exported unchanged, since neither tool implements rainbow-bracket highlighting. Nothing in the tool surfaces either difference — check the manual's Themes section for what HUME actually does with an entry like this.
