//! Terminal-safety state around `#:inline-output` commands — moved out
//! of `host.rs`'s per-capability split.

/// Terminal-safety state around `#:inline-output` commands — accessed
/// through [`EditorHost::output`](super::EditorHost::output).
pub trait OutputHost {
    /// True while the command currently being dispatched is `#:inline-output`
    /// (raw stdout writes are safe — either the alt-screen has been left for
    /// the duration of its body, or there is no TUI to protect at all).
    fn is_inline_output_command(&self) -> bool;

    /// Called by a builtin just before it writes its first byte of terminal
    /// output (`displayln`, a subprocess with inherited stdio, …). Enters the
    /// inline-output alt-screen bracket lazily — on the first real output,
    /// not eagerly at dispatch — so a command whose body only logs never
    /// flashes an empty screen or blocks on an unnecessary keypress. Safe to
    /// call more than once per command body: only the first call (per
    /// dispatch) does anything.
    fn ensure_inline_output_screen(&mut self) -> Result<(), String>;

    /// Arms the bracket for a `(call! name …)`-dispatched command if `name`
    /// is a Steel command declared `#:inline-output #t`. Returns the depth
    /// to truncate back to at the matching [`Self::truncate_inline_output`]
    /// — the frame count before this call's own frame was pushed, so a
    /// caught error in a nested `call!` that left its own frame unpaired
    /// can't be mistaken for this call's frame when the truncate runs.
    /// `None` — no state touched, no restore to pair — for a native,
    /// unknown, or un-activated `Lazy` command, or a host with no
    /// inline-output authority at all.
    fn arm_inline_output(&mut self, name: &str) -> Option<usize>;

    /// Truncate the bracket's frame stack back to `depth` — the value
    /// [`Self::arm_inline_output`] returned, after a paired `call!` returns,
    /// or `0` once at the tail of every Steel session regardless of outcome.
    /// See `hume_editor`'s `InlineOutput::truncate` for why this truncates to
    /// a depth rather than popping the top frame.
    fn truncate_inline_output(&mut self, depth: usize);
}
