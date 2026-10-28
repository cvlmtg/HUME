//! A well-formed call to every builtin that checks a pane value: one list
//! hume-scripting checks against the `builtins!` declarations and a closed
//! buffer, and hume-editor checks against a real editor's panes.

use std::collections::BTreeMap;

pub use crate::builtins::args::pane::PaneNeed;

/// A call to each builtin that checks a pane value, on a pane value bound to
/// `p`, so only the pane can make it raise a pane error.
const PANE_PROBES: &[&str] = &[
    "(buffer-path p)",
    "(buffer-display-path p)",
    "(buffer-name p)",
    "(buffer-dirty? p)",
    "(buffer-text p)",
    "(buffer-line-count p)",
    "(buffer-char-count p)",
    "(buffer-lines p)",
    "(buffer-generation p)",
    "(buffer-undo-tree p)",
    "(buffer-panes p)",
    "(char-offset->line p 0)",
    "(line->char-offset p 0)",
    "(close-buffer! p)",
    r#"(set-buffer-option! p "indent-width" 4)"#,
    r#"(get-buffer-option p "indent-width")"#,
    r#"(attach-keymap! p "keys")"#,
    r#"(detach-keymap! p "keys")"#,
    r#"(diff-buffer-lines p "")"#,
    "(buffer-revision-diff p 0)",
    "(buffer-changes-since p #f)",
    r#"(lsp-notify! p "m" (hash))"#,
    "(lsp-servers p)",
    "(lsp-document-params p)",
    r#"(lsp-position->char-offset p (hash "line" 0 "character" 0))"#,
    r#"(lsp-range->char-range p
             (hash "start" (hash "line" 0 "character" 0) "end" (hash "line" 0 "character" 0)))"#,
    "(lsp-stop! p)",
    "(lsp-restart! p)",
    r#"(set-attachment-hook-triggers! p "src" "server" 'completion (list #\.))"#,
    r#"(set-attachment-completion-triggers! p "src" "server" 'completion (list #\.))"#,
    r#"(set-inlay-hints! p "src" (list))"#,
    r#"(register-sign-source! p "src" 0)"#,
    r#"(set-signs! p "src" (list))"#,
    r#"(set-virtual-lines! p "src" (list))"#,
    r#"(set-eol-text! p "src" (list))"#,
    r#"(set-extra-highlights! p "src" (list))"#,
    r#"(set-line-backgrounds! p "src" (list))"#,
    r#"(set-statusline-text! p "src" "text")"#,
    "(apply-text-edits! p (list))",
    "(apply-workspace-edit! p (hash))",
    "(edit-text! p (list))",
    r#"(lsp-request! p "m" (hash) (lambda (e r) (begin)))"#,
    r#"(lsp-request-all! p "m" (hash) (lambda (e r) (begin)))"#,
    "(buffer-cursor-line p)",
    "(buffer-selections p)",
    "(symbol-under-cursor p)",
    "(selections-linewise? p)",
    "(selections-charwise? p)",
    "(viewport-range p)",
    "(switch-to-buffer! p p)",
    r#"(show-read-only-buffer! p "name" "text")"#,
    "(goto-location! p (hash 'target p 'char-offset 0))",
    "(goto-revision! p 0)",
    "(lsp-position-params p)",
    "(track-position! p)",
    "(lsp-primary-range-params p)",
    "(lsp-linewise-ranges-params p)",
    r#"(prompt! p "label" (lambda (s) s))"#,
    r#"(show-popup! p "text")"#,
    r#"(show-menu! p (list (cons "a" 'a)) (lambda (i) i))"#,
    r#"(show-drawer-list! p (list (cons "a" 'a)) (lambda (i t) i))"#,
    r#"(picker! p (list) (lambda (i) i))"#,
    "(live-picker! p (lambda (i) i) #:source (lambda (t q) q))",
    r#"(insert-key! p "tab")"#,
    r#"(paste-text! p "x")"#,
    "(lsp-show-status! p)",
];

/// Each builtin `builtins!` declares with a pane-checking argument type,
/// with the most demanding need among its arguments.
pub fn declared_pane_needs() -> BTreeMap<&'static str, PaneNeed> {
    crate::builtins::pane_needs().into_iter().collect()
}

/// Every probe as the builtin it calls, that builtin's declared need, and
/// the call.
pub fn pane_probes() -> Vec<(&'static str, PaneNeed, &'static str)> {
    let declared = declared_pane_needs();
    PANE_PROBES
        .iter()
        .map(|expr| {
            let name = expr[1..]
                .split_whitespace()
                .next()
                .expect("a probe calls a builtin");
            let need = *declared
                .get(name)
                .unwrap_or_else(|| panic!("{name} declares no pane-checking argument"));
            (name, need, *expr)
        })
        .collect()
}
