# HUME — Fuzzy Finder (Picker) Roadmap

The picker (`core:pickers`, `g f`/`g b`/`g m`) shipped. This file tracks only
what's left — nothing here describes shipped behavior; that lives with the
code and is kept current there, not here. Delete this file once the
remaining work below ships.

## Where the shipped design lives

| Topic | Where |
|---|---|
| User-facing behavior, keys, file-source chain | `user-manual/docs/pickers.md`, `core-plugins.md` |
| Steel API (`picker!`, `picker-push!`, `picker-replace!`, `picker-source-spawn!`) | `user-manual/docs/plugins.md` "Custom pickers" |
| Plugin internals (git/fd probing, config, path resolution) | `runtime/plugins/core/pickers/plugin.scm`, `README.md` |
| Store, ranking, chokepoints | `hume-editor/src/editor/picker.rs` |
| Fuzzy matcher + budget | `hume-editor/src/editor/fuzzy.rs` |
| Panel widget, theme scopes, geometry | `hume-editor/src/ui/picker_panel.rs` |
| Streaming external-command source | `hume-platform/src/process/line_source.rs`, `hume-editor/src/editor/picker_source.rs` |
| Steel builtin semantics (tokens, kill-on-cancel, exactly-once) | `hume-scripting/src/builtins/mod.rs`, `builtins/ui.rs`, `host.rs` |
| Frequency-cut / bulk-data guardrail (architecture this all follows) | `docs/LSP.md` Decisions table |
| Why picker and completion stay separate session types | `hume-editor/src/editor/picker.rs` module doc |

## Remaining work

- **Preview pane** — render a scratch view of the selected item inside the
  panel; touches buffer lifecycle and the render pipeline. The panel width was
  chosen so a preview split can be added to its right without relayouting the
  list half.
- **Native directory walker** — bare directories without `fd` installed;
  would feed the same drain→store path as the spawn source. Build only if the
  fd-fallback posture proves inadequate in practice. Also tracked in
  `docs/ROADMAP.md`.
- **Q-B6 — unify completion's filter onto the picker's `nucleo-matcher`.**
  Completion still uses a hand-rolled subsequence match; revisit once the
  picker's feel is validated, as its own small task with a side-by-side
  comparison. (`docs/COMPLETION-PICKER.md` cites this ID — keep it stable.)

Not planned: multi-select, picker-specific keybinding customization.
