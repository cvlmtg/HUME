//! The session as plain data. Buffers are named by their index in
//! [`SessionSnapshot::buffers`] and panes by their index in their tab, since
//! buffer and pane ids last one run. No type is recursive, so decoding a
//! crafted file cannot exhaust the stack.

use std::ffi::OsString;

use hume_core::{BufferId, PaneId};
use hume_editing::selection::SelectionsSnapshot;
use hume_engine::pane::{Pane, WrapMode, WrapOverride};
use hume_engine::pipeline::{Direction, EngineView, LayoutTree};
use hume_ops::register::MACRO_REGISTER;
use serde::{Deserialize, Serialize};
use slotmap::SecondaryMap;

use super::keys::SavedKey;
use crate::editor::EditorState;
use crate::editor::jump_list::JumpEntry;
use crate::editor::minibuf::history::HistoryKind;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct SessionSnapshot {
    /// The file buffers, in open order.
    pub(super) buffers: Vec<BufferSnapshot>,
    /// Buffer indices, least recently shown first.
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
    /// Each piece's text and whether it is linewise.
    Text(Vec<(String, bool)>),
    Macro(Vec<SavedKey>),
}

/// The registers a session keeps: those a macro can be recorded into, which
/// are also the ones only the user writes.
pub(super) fn kept_registers() -> impl Iterator<Item = char> {
    std::iter::once(MACRO_REGISTER).chain('0'..='9')
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
            .map(|layout| {
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
        let registers = kept_registers()
            .filter_map(|name| {
                let register = state.registers.read(name)?;
                let text = register.as_pieces().map(|pieces| {
                    RegisterSnapshot::Text(
                        pieces
                            .iter()
                            .map(|p| (p.text().to_owned(), p.is_linewise()))
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
            Some(PaneBufferSnapshot {
                buffer: *index.get(bid)?,
                selections: SelectionsSnapshot::of(buffer_state.selections()),
                top: scroll_top(pane, bid),
                wrap: pane.wraps.get(bid).copied().map(WrapSnapshot::from),
            })
        })
        .collect();
    let (entries, cursor) = entry.jumps.history();
    let kept = |jump: &JumpEntry| index.contains_key(jump.buffer_id);
    let jumps = entries
        .iter()
        .filter(|jump| kept(jump))
        .map(|jump| JumpSnapshot {
            buffer: index[jump.buffer_id],
            selections: SelectionsSnapshot::of(&jump.selections),
        })
        .collect();
    PaneSnapshot {
        shown: index.get(pane.buffer_id()).copied(),
        buffers,
        jumps,
        jump_cursor: entries
            .iter()
            .take(cursor)
            .filter(|jump| kept(jump))
            .count(),
    }
}

/// `pane`'s top display line for `bid`: the live one for the buffer it
/// shows, the saved one for any other.
fn scroll_top(pane: &Pane, bid: BufferId) -> (usize, usize) {
    let top = if bid == pane.buffer_id() {
        pane.viewport.top()
    } else {
        pane.saved_scrolls
            .get(bid)
            .map(|viewport| viewport.top())
            .unwrap_or_default()
    };
    (top.line.index(), top.slot)
}

impl From<Direction> for SplitDirection {
    fn from(direction: Direction) -> Self {
        match direction {
            Direction::Horizontal => Self::Horizontal,
            Direction::Vertical => Self::Vertical,
        }
    }
}

impl From<SplitDirection> for Direction {
    fn from(direction: SplitDirection) -> Self {
        match direction {
            SplitDirection::Horizontal => Self::Horizontal,
            SplitDirection::Vertical => Self::Vertical,
        }
    }
}

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
