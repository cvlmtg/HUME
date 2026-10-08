use hume_engine::pipeline::RenderContext;

use super::*;

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
