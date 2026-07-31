# HUME — Project Instructions

## What is this?
HUME (HUME's Unfinished Modal Editor) is a modal text editor for the terminal, written in Rust. This is an agentic programming / learning project.

## Key files
- `README.md` — Project description
- `docs/ROADMAP.md` — Design decisions, open questions, and milestones
- `docs/LSP.md` — LSP design, prerequisites, and task breakdown
- `docs/LEARNING.md` — Concepts and Rust patterns explained as they arise

## Architectural invariants (quick orientation)
- **Workspace**: `hume-engine/` (rendering pipeline, pane geometry), `hume-editor/` (editor state, scripting glue, keymaps, everything else; builds the `hume` binary), `hume-ops/` (named commands — pure functions of buffer + selections), `hume-editing/` (text model, selections, grapheme utils), `hume-platform/` (terminal I/O, filesystem helpers), `hume-scripting/` (Steel scripting host), `hume-treesitter/` (grammar loading, incremental parse), `hume-lsp/` (LSP client transport), plus `hume-test-fixtures/` (shared test DSL and grammar fixtures, dev-only).
- **Named commands** (`hume-ops/src/edit/`, `hume-ops/src/motion/`) are pure functions of buffer + selections (plus command-specific params like `count: usize`, `MotionMode`). Edits also return a `ChangeSet`. They have no knowledge of keys — `hume-ops` doesn't depend on `hume-editor`, so this is compiler-enforced, not just discipline.
- **Keymaps** (`hume-editor/src/editor/keymap/`) map `KeyEvent` sequences to command names via a trie. Per-mode keymaps (Normal, Extend, Insert).
- **Buffer invariant**: every buffer always ends with a structural `\n`. Cursors always satisfy `head < len_chars()`.

## Rules
- **Update docs/ROADMAP.md** when a decision is made (add to decisions table, remove from open questions) or when milestones change
- **Rust idioms**: Write idiomatic Rust. Prefer pattern matching, iterators, and the type system over runtime checks. Use `Result` and `Option` — no `.unwrap()` in non-test code.
- **Terminal compatibility**: Require true color (24-bit) and synchronized output. Prefer kitty keyboard protocol but fall back gracefully to legacy encoding when unavailable. No shims for truly ancient terminals.
- **Cross-platform**: macOS primary, Linux and Windows (Git Bash / WSL) secondary. Use `termina` or similar abstractions for platform differences — no platform-specific code unless behind `cfg` gates.
- **Keep it simple**: This is a learning project. Prefer clarity over cleverness, and direct solutions over premature abstraction.
- **Testing**: Every editing command, text object, and selection operation must be tested. No untested commands. Core editing logic uses state triples (`initial, op, expected` with cursor/selection markers). Appearance (glyphs, spacing, exact rendered strings) is tested with `insta` snapshots — inline for short one-line element strings, file snapshots for full-frame renders — never with hardcoded string assertions in unit tests; unit/integration tests assert data/semantics only. Verification sequence: `cargo fmt`, then `scripts/test-all.sh` — once — before pushing. `test-all.sh` matches CI exactly, including the network-gated live-grammar e2e tests (`HUME_REQUIRE_LIVE_GRAMMAR_E2E=1`) that a plain `cargo test` silently skips, so it supersedes a bare `cargo test` — never run a full `cargo test` before it, and never rerun the full suite after `fmt` (whitespace-only, doesn't invalidate a green run). Narrow `cargo test <filter>` runs while iterating on a specific failure are fine.
- **Editing model**: Select-then-act. Keys bind to named commands, not to other key sequences. No key-to-key remapping.
- **Scripting**: Steel (Scheme) for plugins and configuration. Rust handles performance-critical paths; Steel handles behavior and customization.

## Day-one architectural invariants
These must be respected from the first line of code — retrofitting is expensive:
- **Selections**: Selections live in a `SelectionSet` (`Vec<Selection>` + `primary: usize` index, kept sorted by start, non-overlapping, non-empty). All edit operations iterate over selections. Selections are always inclusive — `anchor == head` is a 1-char selection covering the character at that index, never a zero-width point.
- **Grapheme clusters**: All motions, selections, and edit operations work on grapheme clusters (`unicode-segmentation`), never raw bytes or `char`. This is the text boundary abstraction — retrofitting is expensive.
  - **Forbidden**: `pos += 1`, `pos -= 1`, `start += 1`, `start -= 1`, `end += 1`, `end -= 1`, `head += 1`, `head -= 1`, `char_at(pos + 1)`, `char_at(pos - 1)` in any motion or selection code. These step over raw chars and will land mid-cluster on combining sequences (e.g. `é` = U+0065 + U+0301) or ZWJ emoji.
  - **Required**: `next_grapheme_boundary(buf, pos)` and `prev_grapheme_boundary(buf, pos)` from `hume-editing/src/grapheme.rs` for all position advances in motion/selection logic.
  - **Allowed**: `line += 1` for line-level iteration, `i += 1` in bracket/delimiter scanning (ASCII only), `len_chars() - 1` for end-of-buffer clamping.
  - **Enforced**: `cargo test no_raw_char_stepping_in_motion_code` (in `hume-editor/src/editor/lints/grapheme.rs`) recursively scans `hume-ops/src/` and `hume-editing/src/lines.rs` + `hume-editing/src/word.rs` for forbidden patterns and fails the build if found.

## Rust coding philosophy
This project is both a product and a learning journey. Write the best Rust possible, and teach as you go.
- **Idiomatic first**: Use the type system, iterators, pattern matching, and ownership as intended. Don't fight the borrow checker. Follow current best practices.
- **Performance by design**: Choose the right data structures and algorithms upfront. Avoid allocations in hot paths, use iterators over index loops.
- **No magic**: No macro-heavy abstractions that hide what's happening. Macros only when they genuinely reduce boilerplate.
- **Clean and readable**: Performance and clarity are not at odds in Rust — the compiler optimizes idiomatic patterns well. When in doubt, prefer the version a newcomer can follow.

## Documentation audiences
Every piece of writing in this repo targets one of three audiences. Know which one before you write, and don't mix them.

1. **End users** — people running HUME who want to use it. Lives in `README.md`, `user-manual/docs/*.md`, `runtime/tutor.rst`, `runtime/init.scm.example`, and any `:help`-style content surfaced inside the editor.
   - No internal names. Don't reference Rust types, Steel builtins used only by the implementation, or module paths. ❌ "the next key pressed is passed as `(pending-char)`" — `pending-char` is a code internal; describe the *behaviour* instead.
   - No babysitting. Assume the reader can follow a short instruction. ❌ "These are absent on a fresh setup" — say what to do, not what the reader will or won't see.
   - Describe what the editor does and how to drive it. Nothing about why it's built that way.

2. **Learners / curious developers** — people who want to understand HUME's *concepts*, not its code yet. Lives in `docs/LEARNING.md` and `docs/learning/*.md`.
   - High-level explanations of ideas: the text model, motions vs text objects, the undo tree, etc.
   - No source-file paths, no `editor/src/...` references, no function names. If the explanation needs them to land, it belongs in a source comment instead.
   - Code snippets are fine when they illustrate an *idea*; they should read as pseudocode-with-Rust-syntax, not as a tour of the actual implementation.

3. **Source readers (contributors)** — people with the file open. The only doc surface for this audience is inline source-code comments. Conversely, source-code comments target only this audience — never an end user, never a learner.
   - Add a brief comment when the *why* is non-obvious; never narrate the *what* (well-named identifiers handle that).
   - When choosing between multiple valid approaches, briefly note why this one.
   - Point out important Rust concepts in use (ownership, lifetimes, traits, iterators), especially when the feature might be unfamiliar.
