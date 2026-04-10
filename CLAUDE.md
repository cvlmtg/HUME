# HUME — Project Instructions

## What is this?
HUME (HUME's Unfinished Modal Editor) is a modal text editor for the terminal, written in Rust. This is an agentic programming / learning project.

## Key files
- `README.md` — Project description
- `ROADMAP.md` — Design decisions, open questions, and milestones
- `LEARNING.md` — Concepts and Rust patterns explained as they arise

## Architectural invariants (quick orientation)
- **Named commands** (`src/ops/edit.rs`, `src/ops/motion.rs`) are pure `(Buffer, SelectionSet) -> (Buffer, SelectionSet)` functions. They have no knowledge of keys.
- **Keymaps** (`src/editor/keymap.rs`) map `KeyEvent` sequences to command names via a trie. Per-mode keymaps (Normal, Insert).
- **Buffer invariant**: every buffer always ends with a structural `\n`. Cursors always satisfy `head < len_chars()`.

## Rules
- **Update ROADMAP.md** when a decision is made (add to decisions table, remove from open questions) or when milestones change
- **Rust idioms**: Write idiomatic Rust. Prefer pattern matching, iterators, and the type system over runtime checks. Use `Result` and `Option` — no `.unwrap()` in non-test code.
- **Terminal compatibility**: Require true color (24-bit) and synchronized output. Prefer kitty keyboard protocol but fall back gracefully to legacy encoding when unavailable (like Helix does). No shims for truly ancient terminals.
- **Cross-platform**: macOS primary, Linux and Windows (Git Bash / WSL) secondary. Use `crossterm` or similar abstractions for platform differences — no platform-specific code unless behind `cfg` gates.
- **Keep it simple**: This is a learning project. Prefer clarity over cleverness, and direct solutions over premature abstraction.
- **Testing**: Every editing command, text object, and selection operation must be tested. No untested commands. Core editing logic uses Helix-style state triples (`initial, op, expected` with cursor/selection markers). Renderer uses `insta` inline snapshots.
- **Editing model**: Select-then-act (Helix/Kakoune). Keys bind to named commands, not to other key sequences. No key-to-key remapping.
- **Scripting**: Steel (Scheme) for plugins and configuration. Rust handles performance-critical paths; Steel handles behavior and customization.

## Day-one architectural invariants
These must be respected from the first line of code — retrofitting is expensive:
- **Selections**: Always `Vec<Selection>`. Single cursor is a vec of length 1. All edit operations iterate over selections. Selections are always inclusive — `anchor == head` is a 1-char selection covering the character at that index, never a zero-width point.
- **Display lines**: The renderer iterates "display lines" (buffer line or virtual line), never buffer lines directly. Initially 1:1, but the abstraction is required for virtual lines later.
- **Grapheme clusters**: All motions, selections, and edit operations work on grapheme clusters (`unicode-segmentation`), never raw bytes or `char`. This is the text boundary abstraction — retrofitting is expensive.
  - **Forbidden**: `pos += 1`, `pos -= 1`, `start += 1`, `start -= 1`, `end += 1`, `end -= 1`, `head += 1`, `head -= 1`, `char_at(pos + 1)`, `char_at(pos - 1)` in any motion or selection code. These step over raw chars and will land mid-cluster on combining sequences (e.g. `é` = U+0065 + U+0301) or ZWJ emoji.
  - **Required**: `next_grapheme_boundary(buf, pos)` and `prev_grapheme_boundary(buf, pos)` from `src/core/grapheme.rs` for all position advances in motion/selection logic.
  - **Allowed**: `line += 1` for line-level iteration, `i += 1` in bracket/delimiter scanning (ASCII only), `len_chars() - 1` for end-of-buffer clamping.
  - **Enforced**: `cargo test no_raw_char_stepping_in_motion_code` (in `src/core/grapheme.rs`) scans `src/ops/motion.rs`, `src/ops/text_object.rs`, and `src/ops/selection_cmd.rs` for forbidden patterns and fails the build if found.

## Rust coding philosophy
This project is both a product and a learning journey. Write the best Rust possible, and teach as you go.
- **Idiomatic first**: Use the type system, iterators, pattern matching, and ownership as intended. Don't fight the borrow checker. Follow current best practices.
- **Performance by design**: Choose the right data structures and algorithms upfront. Avoid allocations in hot paths, use iterators over index loops.
- **No magic**: No macro-heavy abstractions that hide what's happening. Macros only when they genuinely reduce boilerplate.
- **Clean and readable**: Performance and clarity are not at odds in Rust — the compiler optimizes idiomatic patterns well. When in doubt, prefer the version a newcomer can follow.

## Teaching Guidelines
- When writing non-obvious code, add a brief inline comment explaining *why*, not just *what*
- When choosing between multiple valid approaches, briefly note why you picked this one
- Point out when you're using an important Rust concept (ownership, lifetimes, traits, iterators, etc.)
- When using a Rust feature that might be unfamiliar (lifetimes, trait bounds, zero-cost abstractions), explain why it's the right tool — in code comments or conversation.
