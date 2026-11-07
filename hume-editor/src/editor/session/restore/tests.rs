use hume_editing::selection::{SelectionSet, SelectionsSnapshot};
use hume_editing::text::BufferText;

use super::super::snapshot::{
    BufferSnapshot, JumpSnapshot, PaneBufferSnapshot, PaneSnapshot, RegisterSnapshot,
    SessionSnapshot, SplitDirection, SplitStep, TabSnapshot,
};
use super::check;
use crate::editor::stamped_file::decode_whole;

/// A session of one buffer shown in two side-by-side panes.
fn valid() -> SessionSnapshot {
    let text = BufferText::from("a\n");
    let selections = || SelectionsSnapshot::of(&SelectionSet::at_text_start(&text));
    let pane = || PaneSnapshot {
        shown: Some(0),
        buffers: vec![PaneBufferSnapshot {
            buffer: 0,
            selections: selections(),
            top: (0, 0),
            wrap: None,
        }],
        jumps: vec![JumpSnapshot {
            buffer: 0,
            selections: selections(),
        }],
        jump_cursor: 1,
    };
    SessionSnapshot {
        buffers: vec![BufferSnapshot {
            path: "/a.txt".into(),
            md5: [0; 16],
        }],
        mru: vec![0],
        tabs: vec![TabSnapshot {
            panes: vec![pane(), pane()],
            splits: vec![SplitStep {
                pane: 0,
                direction: SplitDirection::Horizontal,
                at: None,
            }],
            focused: 1,
        }],
        current_tab: 0,
        registers: vec![],
        search: None,
        history: vec![],
    }
}

/// Whether [`check`] refuses [`valid`] once `break_it` has changed it.
fn refused(break_it: fn(&mut SessionSnapshot)) -> bool {
    let mut snapshot = valid();
    break_it(&mut snapshot);
    check(&snapshot).is_err()
}

#[test]
fn a_session_whose_indices_all_hold_passes() {
    assert_eq!(check(&valid()), Ok(()));
}

#[test]
fn every_index_past_what_the_session_holds_is_refused() {
    let broken: [fn(&mut SessionSnapshot); 10] = [
        |s| s.mru = vec![1],
        |s| s.current_tab = 1,
        |s| s.tabs.clear(),
        |s| s.tabs[0].splits[0].pane = 1,
        |s| {
            s.tabs[0].panes.pop();
        },
        |s| s.tabs[0].focused = 2,
        |s| s.tabs[0].panes[1].shown = Some(1),
        |s| s.tabs[0].panes[0].buffers[0].buffer = 3,
        |s| s.tabs[0].panes[1].jumps[0].buffer = 1,
        |s| s.tabs[0].panes[0].jump_cursor = 2,
    ];
    for (n, break_it) in broken.into_iter().enumerate() {
        assert!(refused(break_it), "case {n}");
    }
}

#[test]
fn an_mru_that_does_not_list_each_buffer_once_is_refused() {
    let broken: [fn(&mut SessionSnapshot); 2] = [
        |s| s.mru.clear(),
        |s| {
            s.buffers.push(BufferSnapshot {
                path: "/b.txt".into(),
                md5: [0; 16],
            });
            s.mru = vec![0, 0];
        },
    ];
    for (n, break_it) in broken.into_iter().enumerate() {
        assert!(refused(break_it), "case {n}");
    }
}

#[test]
fn a_register_no_session_keeps_is_refused() {
    assert!(refused(|s| s
        .registers
        .push(('c', RegisterSnapshot::Text(vec![])))));
}

#[test]
fn a_split_share_of_zero_does_not_decode() {
    let step = |share: u32| {
        postcard::to_allocvec(&(0usize, SplitDirection::Horizontal, Some(share))).expect("encodes")
    };
    assert!(decode_whole::<SplitStep>(&step(1)).is_ok());
    assert!(decode_whole::<SplitStep>(&step(0)).is_err());
}
