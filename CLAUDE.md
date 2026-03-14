# HUME — Project Instructions

## What is this?
HUME (HUME's Unfinished Modal Editor) is a modal text editor for the terminal, written in Rust. This is a vibe coding / learning project — not a production product.

## Key files
- `README.md` — Project description
- `GOALS.md` — Vision, decisions, and open questions
- `PLAN.md` — Tech stack, architecture, and milestones
- `LEARNING.md` — Concepts and Rust patterns explained as they arise

## Current state (as of 2026-03-14)
- M1 core engine in progress. Completed: Buffer, Selection, ChangeSet, Transaction, edit operations, motions.
- **Named commands** (`src/edit.rs`, `src/motion.rs`) are pure `(Buffer, SelectionSet) -> (Buffer, SelectionSet)` functions in the core layer. They have no knowledge of keys.
- **Keymaps** (wiring key events to named commands) are an Editor-layer concern — not yet implemented. They belong to M3.
- 222 tests passing (`cargo test`).

## Rules
- **Update GOALS.md** when a decision is made (add to the decisions table, remove from open questions)
- **Update PLAN.md** when the plan changes (milestones, tech stack, architecture)
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
