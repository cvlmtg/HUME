// `hume_platform::QUIT_GRACE`'s own doc comment states its budget as
// `Editor::SHUTDOWN_GRACE` plus `hume_lsp::transport`'s `WRITER_FLUSH_GRACE`
// per still-live LSP server, kept in step across three crates by comment
// alone — a chokepoint invariant enforced only by a comment is no invariant
// at all. This test ties the three real
// constants together so a change that breaks the promised relationship
// fails loudly instead of silently drifting.

use super::*;

#[test]
fn quit_grace_covers_shutdown_grace_plus_a_handful_of_lsp_servers() {
    let quit_grace = hume_platform::quit_grace();
    let shutdown_grace = Editor::SHUTDOWN_GRACE;
    let writer_flush_grace = hume_lsp::transport::writer_flush_grace();

    assert!(
        quit_grace > shutdown_grace,
        "QUIT_GRACE ({quit_grace:?}) must exceed SHUTDOWN_GRACE ({shutdown_grace:?}) on its \
         own, before any LSP servers are even in the picture"
    );

    // "A handful" per QUIT_GRACE's own doc comment — pinned to a concrete
    // number so a shrinking margin fails loudly instead of drifting
    // silently as long as the `>` above still holds.
    const SERVERS_THE_BUDGET_MUST_COVER: u32 = 5;
    let margin = quit_grace - shutdown_grace;
    assert!(
        margin >= writer_flush_grace * SERVERS_THE_BUDGET_MUST_COVER,
        "QUIT_GRACE's margin over SHUTDOWN_GRACE ({margin:?}) must cover at least \
         {SERVERS_THE_BUDGET_MUST_COVER} live LSP servers' WRITER_FLUSH_GRACE \
         ({writer_flush_grace:?} each) — got headroom for only {} whole servers",
        margin.as_millis() / writer_flush_grace.as_millis().max(1)
    );
}
