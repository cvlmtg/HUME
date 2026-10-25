//! Terminal-safety state around `#:inline-output` commands.

/// Why [`OutputHost::claim_terminal`] refused the terminal.
#[derive(Debug, PartialEq, Eq)]
pub enum ClaimRefused {
    /// No `#:inline-output` command's frame is open.
    NoFrame,
    /// Leaving the editor's screen failed, with this reason.
    Enter(String),
}

/// Terminal-safety state around `#:inline-output` commands, accessed
/// through [`EditorHost::output`](super::EditorHost::output).
pub trait OutputHost {
    /// Claims the terminal for a builtin about to write to it (`print!`, a
    /// subprocess with inherited stdio). Refused with
    /// [`ClaimRefused::NoFrame`] unless an `#:inline-output` command's frame
    /// is open, the one state in which a script may write to the terminal.
    /// Asked on every write, not once per session: a `call!` can open a
    /// frame mid-body. The first claim in a dispatch enters the inline-output
    /// alt-screen bracket, so the screen is entered lazily, on the first real
    /// output: a command whose body only logs never flashes an empty screen
    /// or blocks on an unnecessary keypress.
    fn claim_terminal(&mut self) -> Result<(), ClaimRefused>;

    /// Arms the bracket for a `(call! name …)`-dispatched command if `name`
    /// is a Steel command declared `#:inline-output #t`. Returns the depth
    /// to truncate back to at the matching [`Self::truncate_inline_output`]:
    /// the frame count before this call's own frame was pushed, so a
    /// caught error in a nested `call!` that left its own frame unpaired
    /// can't be mistaken for this call's frame when the truncate runs.
    /// `None` (no state touched, no restore to pair) for a native,
    /// unknown, or un-activated `Lazy` command, or a host with no
    /// inline-output authority at all.
    fn arm_inline_output(&mut self, name: &str) -> Option<usize>;

    /// Truncate the bracket's frame stack back to `depth`, the value
    /// [`Self::arm_inline_output`] returned, after a paired `call!` returns,
    /// or `0` once at the tail of every Steel session regardless of outcome.
    /// See `hume_editor`'s `InlineOutput::truncate` for why this truncates to
    /// a depth rather than popping the top frame.
    fn truncate_inline_output(&mut self, depth: usize);
}
