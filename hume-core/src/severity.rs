//! The two severity scales HUME shares across crates: how a message is
//! logged and shown, and how serious a language server's diagnostic is.

named_enum! {
    /// Severity level for a message, controlling both logging and display.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Severity {
        /// Ephemeral confirmation (e.g. "Written 42 lines"). Shown, not logged.
        Info => "info",
        /// Something the user should review (e.g. unknown config key). Logged and shown.
        Warning => "warning",
        /// A failure the user must address (e.g. script error). Logged and shown.
        Error => "error",
        /// Verbose diagnostic detail (e.g. stack trace). Logged only, not shown in statusline.
        Trace => "trace",
    }
}

named_enum! {
    /// How serious a diagnostic is. Ordered least-to-most-lenient so
    /// `severity <= floor` means "at least as severe as floor": e.g. `floor =
    /// Warning` keeps `Error` and `Warning`, drops `Info`/`Hint`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub enum DiagSeverity {
        Error => "error",
        Warning => "warning",
        Info => "info",
        Hint => "hint",
    }
}

impl DiagSeverity {
    /// Place on the scale, `0` for `Error` up to `3` for `Hint`: lower is
    /// more severe, and it indexes [`Self::ALL`].
    pub const fn rank(self) -> u8 {
        self as u8
    }
}
