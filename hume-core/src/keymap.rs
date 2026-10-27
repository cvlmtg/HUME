//! Key-binding vocabulary shared by the scripting builtins that bind keys
//! and the keymaps that hold them.

named_enum! {
    /// Which keymap a binding belongs to, as `bind-key!` and `unbind-key!`
    /// name it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BindMode {
        Normal => "normal",
        /// Sparse extend-mode overrides. In extend mode a keymap's overrides
        /// are checked before its normal trie, whose match runs with
        /// `extend = true`.
        Extend => "extend",
        Insert => "insert",
        /// The command mode prompt: the `:` line and `prompt!` prompts.
        Command => "command",
    }
}
