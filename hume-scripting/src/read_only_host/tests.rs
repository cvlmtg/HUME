use std::rc::Rc;

use super::{ReadOnly, ReadOnlySession, SessionHost};
use crate::attribution::{AttributionStack, Owner};
use crate::testing::null_host::EmptyLspHost;
use crate::testing::test_support::SteelCtxTestHarness;
use crate::types::Effect;

fn error_text(result: Result<impl Sized, steel::rerrs::SteelErr>) -> String {
    match result {
        Ok(_) => String::new(),
        Err(e) => e.to_string(),
    }
}

#[test]
fn a_capability_is_reached_while_the_session_may_change_the_editor() {
    let mut h = SteelCtxTestHarness::new();
    let mut host = EmptyLspHost::default();
    let mut ctx = h.ctx_with_host(&mut host);
    assert!(ctx.host.require(|h| h.lsp()).is_ok());
}

#[test]
fn a_capability_the_host_has_is_reached_inside_a_read_only_frame() {
    let mut h = SteelCtxTestHarness::new();
    let mut host = EmptyLspHost::default();
    let mut ctx = h.ctx_with_host(&mut host);
    ctx.host.enter_paste_handler_call(Rc::new(Owner::User));
    assert!(ctx.host.require(|h| h.lsp()).is_ok());
}

#[test]
fn a_capability_of_writes_only_is_refused_inside_a_read_only_frame() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.host.enter_paste_handler_call(Rc::new(Owner::User));
    assert!(
        error_text(ctx.host.require(|h| h.timers()))
            .contains("not available while an on-paste handler runs")
    );
    assert_eq!(
        ctx.host.take_refusal().as_deref(),
        Some("the editor cannot be changed while an on-paste handler runs")
    );
}

#[test]
fn a_missing_capability_is_not_supported_outside_a_read_only_frame() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    assert!(error_text(ctx.host.require(|h| h.edits())).contains("not supported by this host"));
}

#[test]
fn a_read_only_frame_outlives_a_deeper_frame_and_ends_at_its_own() {
    let mut h = SteelCtxTestHarness::new();
    let mut host = EmptyLspHost::default();
    let mut ctx = h.ctx_with_host(&mut host);
    let depth = ctx.host.enter_paste_handler_call(Rc::new(Owner::User));
    let deeper = ctx.host.enter_call(Rc::new(Owner::User));
    ctx.host.leave_frames_above(deeper);
    assert_eq!(
        ctx.host.read_only_reason(),
        Some(ReadOnly::PasteHandler { depth })
    );
    ctx.host.leave_frames_above(depth);
    assert_eq!(ctx.host.read_only_reason(), None);
    assert!(ctx.host.require(|h| h.lsp()).is_ok());
}

#[test]
fn leaving_frames_keeps_a_session_read_only_from_its_start() {
    let mut host = EmptyLspHost::default();
    let mut frames = AttributionStack::default();
    let mut session = SessionHost::read_only(&mut host, &mut frames, ReadOnlySession::Rendering);
    session.leave_frames_above(0);
    assert_eq!(
        session.read_only_reason(),
        Some(ReadOnly::Session(ReadOnlySession::Rendering))
    );
}

#[test]
fn a_language_config_session_names_its_reason_in_a_refusal() {
    let mut host = EmptyLspHost::default();
    let mut frames = AttributionStack::default();
    let session = SessionHost::read_only(&mut host, &mut frames, ReadOnlySession::LanguageConfig);
    assert_eq!(
        session.check_write(),
        Err("the editor cannot be changed while a language's configuration loads".to_owned())
    );
}

#[test]
fn an_effect_is_refused_inside_a_read_only_frame() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.host.enter_paste_handler_call(Rc::new(Owner::User));
    let refused = error_text(ctx.push_effect(Effect::GrammarSweep("rust".to_string())));
    assert!(
        refused.contains("the editor cannot be changed while an on-paste handler runs"),
        "{refused}"
    );
    assert!(ctx.queued_effects().is_empty());
}
