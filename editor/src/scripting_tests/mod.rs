// Scripting integration tests — live in the editor crate so they can use
// both editor types (EditorSettings, Keymap, etc.) and the scripting crate.
pub(crate) mod test_harness;
mod tests;
