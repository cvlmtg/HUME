//! The session as plain data. Buffers are named by their index in
//! [`SessionSnapshot::buffers`] and panes by their index in their tab, since
//! buffer and pane ids last one run. No type is recursive, so decoding a
//! crafted file cannot exhaust the stack.

use std::ffi::OsString;

use hume_core::{BufferId, PaneId};
use hume_editing::selection::SelectionsSnapshot;
use hume_engine::pane::{WrapMode, WrapOverride};
use hume_engine::pipeline::{Direction, EngineView, LayoutTree};
use hume_ops::register::{MACRO_REGISTERS, Shape};
use serde::{Deserialize, Serialize};
use slotmap::SecondaryMap;

use super::keys::SavedKey;
use crate::editor::EditorState;
use crate::editor::jump_list::kept_with_cursor;
use crate::editor::minibuf::history::HistoryKind;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct SessionSnapshot {
    /// The file buffers, in open order.
    pub(super) buffers: Vec<BufferSnapshot>,
    /// Every buffer index once, least recently shown first.
    pub(super) mru: Vec<usize>,
    /// The tabs in display order.
    pub(super) tabs: Vec<TabSnapshot>,
    pub(super) current_tab: usize,
    pub(super) registers: Vec<(char, RegisterSnapshot)>,
    pub(super) search: Option<String>,
    /// Each prompt's history, oldest entry first.
    pub(super) history: Vec<(HistoryKind, Vec<String>)>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct BufferSnapshot {
    /// The path as the user named it, symlinks kept.
    pub(super) path: OsString,
    /// The MD5 of the text the positions below were taken on.
    pub(super) md5: [u8; 16],
}

/// One tab's layout, rebuilt from its first pane by splitting: split `k`
/// splits pane `splits[k].pane`, which keeps its place as the first half,
/// and creates pane `k + 1` as the second.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct TabSnapshot {
    pub(super) panes: Vec<PaneSnapshot>,
    pub(super) splits: Vec<SplitStep>,
    pub(super) focused: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(super) struct SplitStep {
    pub(super) pane: usize,
    pub(super) direction: SplitDirection,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(super) enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct PaneSnapshot {
    /// The buffer the pane shows, `None` for one no session keeps.
    pub(super) shown: Option<usize>,
    /// The pane's state for each file buffer it has shown.
    pub(super) buffers: Vec<PaneBufferSnapshot>,
    pub(super) jumps: Vec<JumpSnapshot>,
    /// Where the pane stands in `jumps`, `jumps.len()` at the present.
    pub(super) jump_cursor: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct PaneBufferSnapshot {
    pub(super) buffer: usize,
    pub(super) selections: SelectionsSnapshot,
    /// The viewport's top display line, as its buffer line and slot.
    pub(super) top: (usize, usize),
    pub(super) wrap: Option<WrapSnapshot>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct JumpSnapshot {
    pub(super) buffer: usize,
    pub(super) selections: SelectionsSnapshot,
}

/// A pane's wrap override for one buffer.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(super) struct WrapSnapshot {
    pub(super) mode: Option<WrapModeSnapshot>,
    pub(super) saved: Option<WrapModeSnapshot>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(super) enum WrapModeSnapshot {
    None,
    Soft(u16),
    Word(u16),
    Indent(u16),
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) enum RegisterSnapshot {
    /// Each piece's text and shape.
    Text(Vec<(String, ShapeSnapshot)>),
    Macro(Vec<SavedKey>),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(super) enum ShapeSnapshot {
    Charwise,
    Linewise,
}

impl SessionSnapshot {
    pub(super) fn capture(state: &EditorState, view: &EngineView) -> Self {
        let mut index = SecondaryMap::new();
        let buffers = state
            .buffers
            .iter()
            .filter_map(|(bid, buf)| {
                let file = buf.file()?;
                index.insert(bid, index.len());
                Some(BufferSnapshot {
                    path: file.typed().as_os_str().to_owned(),
                    md5: buf.text_md5(),
                })
            })
            .collect();
        let mru = state
            .buffers
            .mru()
            .iter()
            .filter_map(|&bid| index.get(bid).copied())
            .collect();
        let tabs = state
            .tabs
            .layouts(view.focused_layout())
            .map(|(_, layout)| {
                let mut splits = Vec::new();
                let mut leaves = Vec::new();
                grow(layout.tree(), 0, &mut splits, &mut leaves);
                leaves.sort_by_key(|&(n, _)| n);
                TabSnapshot {
                    focused: leaves
                        .iter()
                        .position(|&(_, pid)| pid == layout.focused())
                        .expect("the focused pane is a leaf of its layout"),
                    panes: leaves
                        .iter()
                        .map(|&(_, pid)| capture_pane(state, view, &index, pid))
                        .collect(),
                    splits,
                }
            })
            .collect();
        // The macro registers are the ones only the user writes.
        let registers = MACRO_REGISTERS
            .into_iter()
            .filter_map(|name| {
                let register = state.registers.read(name)?;
                let text = register.as_pieces().map(|pieces| {
                    RegisterSnapshot::Text(
                        pieces
                            .iter()
                            .map(|p| (p.text().to_owned(), p.shape().into()))
                            .collect(),
                    )
                });
                let content = text.or_else(|| {
                    let keys = register.as_macro()?;
                    Some(RegisterSnapshot::Macro(
                        keys.iter().copied().map(SavedKey::from).collect(),
                    ))
                })?;
                Some((name, content))
            })
            .collect();
        Self {
            buffers,
            mru,
            tabs,
            current_tab: state.tabs.current_pos(),
            registers,
            search: state.registers.search_register().map(str::to_owned),
            history: state
                .history
                .entries_by_kind()
                .into_iter()
                .map(|(kind, entries)| (kind, entries.iter().cloned().collect()))
                .collect(),
        }
    }
}

/// Record the split steps that turn pane `at` into `tree`, and each leaf
/// with the number of the pane that ends up as it. A split keeps `at` as
/// its first half and numbers its second half after itself.
fn grow(
    tree: &LayoutTree,
    at: usize,
    splits: &mut Vec<SplitStep>,
    leaves: &mut Vec<(usize, PaneId)>,
) {
    match tree {
        LayoutTree::Leaf(pid) => leaves.push((at, *pid)),
        LayoutTree::Split {
            direction,
            children,
            ..
        } => {
            splits.push(SplitStep {
                pane: at,
                direction: (*direction).into(),
            });
            let second = splits.len();
            grow(&children.0, at, splits, leaves);
            grow(&children.1, second, splits, leaves);
        }
    }
}

fn capture_pane(
    state: &EditorState,
    view: &EngineView,
    index: &SecondaryMap<BufferId, usize>,
    pid: PaneId,
) -> PaneSnapshot {
    let pane = &view.panes[pid];
    let entry = &state.panes.entries[pid];
    let buffers = entry
        .buffers()
        .filter_map(|(bid, buffer_state)| {
            let top = pane.scroll_top(bid);
            Some(PaneBufferSnapshot {
                buffer: *index.get(bid)?,
                selections: SelectionsSnapshot::of(buffer_state.selections()),
                top: (top.line.index(), top.slot),
                wrap: pane.wrap_in(bid).map(WrapSnapshot::from),
            })
        })
        .collect();
    let (entries, cursor) = entry.jumps.history();
    let (jumps, jump_cursor) = kept_with_cursor(
        entries.iter().map(|jump| {
            Some(JumpSnapshot {
                buffer: *index.get(jump.buffer_id)?,
                selections: SelectionsSnapshot::of(&jump.selections),
            })
        }),
        cursor,
    );
    PaneSnapshot {
        shown: index.get(pane.buffer_id()).copied(),
        buffers,
        jumps,
        jump_cursor,
    }
}

mirror!(Direction <=> SplitDirection { Horizontal, Vertical });

impl From<WrapOverride> for WrapSnapshot {
    fn from(wrap: WrapOverride) -> Self {
        Self {
            mode: wrap.mode.map(Into::into),
            saved: wrap.saved.map(Into::into),
        }
    }
}

impl From<WrapSnapshot> for WrapOverride {
    fn from(wrap: WrapSnapshot) -> Self {
        Self {
            mode: wrap.mode.map(Into::into),
            saved: wrap.saved.map(Into::into),
        }
    }
}

impl From<WrapMode> for WrapModeSnapshot {
    fn from(mode: WrapMode) -> Self {
        match mode {
            WrapMode::None => Self::None,
            WrapMode::Soft { width } => Self::Soft(width),
            WrapMode::Word { width } => Self::Word(width),
            WrapMode::Indent { width } => Self::Indent(width),
        }
    }
}

impl From<WrapModeSnapshot> for WrapMode {
    fn from(mode: WrapModeSnapshot) -> Self {
        match mode {
            WrapModeSnapshot::None => Self::None,
            WrapModeSnapshot::Soft(width) => Self::Soft { width },
            WrapModeSnapshot::Word(width) => Self::Word { width },
            WrapModeSnapshot::Indent(width) => Self::Indent { width },
        }
    }
}

mirror!(Shape <=> ShapeSnapshot { Charwise, Linewise });
