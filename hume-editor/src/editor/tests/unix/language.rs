use super::super::language::attach_host;
use super::*;

/// An invalid glob pattern in `define-language!` is warned and silently skipped;
/// valid patterns and other languages still register correctly.
///
/// Flip: without validation, a bad glob would silently drop at compile time with
/// no message, making it undetectable to the user.
#[test]
fn invalid_glob_in_define_language_warns_and_skips() {
    use hume_scripting::PendingLanguageReg;
    let mut ed = editor_from("-[a]>b\n");
    attach_host(&mut ed, "");
    let regs = vec![PendingLanguageReg::Identity {
        name: "test-lang".to_owned(),
        extensions: vec!["xyz".to_owned()],
        globs: vec!["valid/*.xyz".to_owned(), "[invalid-glob".to_owned()],
        shebangs: vec![],
    }];
    ed.apply_pending_language_regs(regs);
    // Valid glob must be registered; extension lookup must work.
    assert!(
        ed.state.config.languages.by_extension("xyz").is_some(),
        "extension must register despite bad glob"
    );
    // At least one warning must mention the bad pattern.
    let has_warning = ed
        .state
        .message_log
        .entries()
        .any(|e| e.text.contains("[invalid-glob") || e.text.contains("invalid-glob"));
    assert!(
        has_warning,
        "invalid glob must produce a warning; log: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| &e.text)
            .collect::<Vec<_>>()
    );
}
