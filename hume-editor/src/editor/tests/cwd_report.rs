use std::path::Path;

use hume_engine::pipeline::RenderContext;

use super::*;
use crate::editor::cwd_report::working_directory_url;

#[cfg(not(windows))]
#[test]
fn url_names_this_machine_and_percent_encodes_the_path() {
    let url = working_directory_url(Path::new("/tmp/a b")).expect("absolute path has a URL");
    assert!(url.starts_with("file://"), "{url}");
    assert!(url.ends_with("/tmp/a%20b"), "{url}");
}

#[test]
fn relative_path_has_no_url() {
    assert_eq!(working_directory_url(Path::new("rel/dir")), None);
}

#[test]
fn prepare_frame_records_the_reported_cwd_without_a_terminal() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    assert_eq!(ed.applied_cwd, None);

    ed.state.cwd = tmp.path().to_path_buf();
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    assert_eq!(ed.applied_cwd.as_deref(), Some(tmp.path()));
}
