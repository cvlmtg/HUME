use std::os::unix::fs::PermissionsExt;

use hume::cli::{ConfigSource, FileArg};

/// A directory that canonicalizes and lists as a directory but cannot be
/// entered: no search permission. `None` when the process can enter it anyway
/// (root).
fn unenterable_dir(parent: &std::path::Path) -> Option<std::path::PathBuf> {
    let dir = parent.join("locked");
    std::fs::create_dir(&dir).expect("create the locked dir");
    std::fs::set_permissions(&dir, PermissionsExt::from_mode(0o600))
        .expect("drop the dir's search permission");
    std::fs::metadata(dir.join("probe"))
        .is_err_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
        .then_some(dir)
}

#[test]
fn set_cwd_into_a_directory_the_process_cannot_enter_raises_and_moves_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(locked) = unenterable_dir(tmp.path()) else {
        return;
    };
    let input = tmp.path().join("in.txt");
    let output = tmp.path().join("out.txt");
    let marker = tmp.path().join("marker");
    std::fs::write(&input, "hello\n").unwrap();
    let config = tmp.path().join("init.scm");
    std::fs::write(
        &config,
        format!(
            r#"(define-command! "probe" ""
                 (lambda ()
                   (let ((outcome (with-handler (lambda (err) "raised")
                                    (begin (set-cwd! "{}") "moved"))))
                     (call-with-output-file "{}"
                       (lambda (port) (write-string (string-append outcome "|" (cwd)) port))))))
               (bind-key! 'normal "Z" "probe")"#,
            locked.display(),
            marker.display()
        ),
    )
    .unwrap();
    let start = std::env::current_dir().unwrap();

    hume::run_keys(
        FileArg {
            path: input,
            pos: None,
        },
        "Z",
        output,
        ConfigSource::File(config),
        start.clone(),
    )
    .unwrap();

    let got = std::fs::read_to_string(&marker).unwrap();
    assert_eq!(got, format!("raised|{}", start.display()));
}
