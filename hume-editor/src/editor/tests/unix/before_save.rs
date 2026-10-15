// Hard write failures during `:wa` with `on-before-buffer-save` handlers.
// A directory in place of a file makes the write fail on unix.

use super::*;
use crate::editor::tests::before_save::{errors, two_dirty_buffers};

fn replace_with_directory(file: &tempfile::TempPath) {
    std::fs::remove_file(file).unwrap();
    std::fs::create_dir(file).unwrap();
}

/// Every member of the batch fails, so a second error means the batch went
/// on after the first.
#[test]
fn write_all_hard_error_drops_the_members_not_yet_written() {
    let (mut ed, file_a, file_b, dir) = two_dirty_buffers();
    run(
        &mut ed,
        dir.path(),
        r#"(define dones (list))
           (register-hook! 'on-before-buffer-save
             (lambda (pane done) (set! dones (append dones (list done)))))
           (define-typed-command! "release" ""
             (lambda ()
               (let ((d (car dones)))
                 (set! dones (cdr dones))
                 (d))))"#,
    );
    type_cmd(&mut ed, ":wa!");
    ed.settle();
    replace_with_directory(&file_a);
    replace_with_directory(&file_b);

    type_cmd(&mut ed, ":release");
    ed.settle();
    type_cmd(&mut ed, ":release");
    ed.settle();

    assert_eq!(errors(&ed), 1);
    std::fs::remove_dir(&file_a).unwrap();
    std::fs::remove_dir(&file_b).unwrap();
}

#[test]
fn write_all_hard_error_stops_before_later_buffers() {
    let (mut ed, file_a, file_b, _dir) = two_dirty_buffers();
    replace_with_directory(&file_a);
    replace_with_directory(&file_b);

    type_cmd(&mut ed, ":wa!");
    ed.settle();

    assert_eq!(errors(&ed), 1);
    std::fs::remove_dir(&file_a).unwrap();
    std::fs::remove_dir(&file_b).unwrap();
}
