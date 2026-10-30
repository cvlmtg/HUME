use super::*;
use crate::null_host::RecordingInlineOutputHost;
use crate::test_support::SteelCtxTestHarness;

/// `print!`'s result on a host whose inline-output frame is open when
/// `frame_open`, and how many times it left the editor's screen.
fn print_with_frame_open(frame_open: bool) -> (Result<SteelVal, String>, usize) {
    let mut host = RecordingInlineOutputHost::with_frame_open(frame_open);
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_with_host(&mut host);
    let result = print(&mut ctx, String::new()).map_err(|e| e.to_string());
    drop(ctx);
    (result, host.claims)
}

#[test]
fn print_leaves_the_editors_screen_inside_an_inline_output_command() {
    assert_eq!(print_with_frame_open(true), (Ok(SteelVal::Void), 1));
}

/// Outside an `#:inline-output` command `print!` raises before it writes.
#[test]
fn print_raises_outside_an_inline_output_command() {
    let (result, claims) = print_with_frame_open(false);
    let err = result.expect_err("print! outside an #:inline-output command");
    assert!(
        err.contains("runs only inside an #:inline-output command"),
        "{err}"
    );
    assert_eq!(claims, 0);
}

/// `print!` is reachable from a required module, the shape of every plugin
/// command.
#[test]
fn print_reaches_the_terminal_from_a_required_module() {
    use crate::ScriptingHost;
    use crate::null_host::CommandStoreHost;
    use hume_platform::dirs::Dirs;

    let tmp = tempfile::tempdir().unwrap();
    let plugin_path = tmp.path().join("probe.scm");
    std::fs::write(
        &plugin_path,
        r#"(define-command! "probe" "doc" (lambda () (print! "")) #:inline-output #t)"#,
    )
    .unwrap();
    let mut host = ScriptingHost::for_testing(&Dirs::none());
    let mut store = CommandStoreHost::default();
    let escaped_path =
        crate::steel_path_literal(&plugin_path).expect("plugin path must not contain '\"'");
    host.eval_source(&format!(r#"(require {escaped_path})"#), &mut store)
        .expect("requiring the plugin file must not error");
    let mut recording_host = RecordingInlineOutputHost::default();
    host.call_steel_cmd(store.proc("probe"), vec![], &mut recording_host)
        .unwrap_or_else(|e| panic!("{}", e.message));
    assert_eq!(recording_host.claims, 1);
}
