//! Each buffer's LSP document state: which servers it is attached to, and
//! the text changes not yet sent to them. A [`Document`] exists only while
//! the buffer has at least one attachment, so changes are queued only for a
//! buffer some server tracks, and a buffer's queue goes when its last
//! attachment does. Attachments change only through `attach.rs`.

use hume_editing::changeset::ChangeSet;
use hume_editing::edit::TextChange;
use hume_engine::pipeline::BufferId;
use hume_lsp::backend::ServerId;
use hume_scripting::FeatureFilter;
use ropey::Rope;
use rustc_hash::FxHashMap;
use slotmap::SecondaryMap;

use crate::editor::triggers::set_trigger_chars;
use hume_scripting::TriggerKind;

/// One server a buffer is attached to, with the features its language's
/// list lets that server handle for it, and the trigger characters plugins
/// registered for this buffer under it (kind and source name -> chars).
/// The trigger characters go with the attachment, and with a change of its
/// filter. `pull_result_id` is the `resultId` of the last diagnostics report
/// the server gave for this buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::editor) struct Attachment {
    pub(in crate::editor) server: ServerId,
    pub(in crate::editor) filter: FeatureFilter,
    triggers: FxHashMap<(TriggerKind, String), Vec<char>>,
    pull_result_id: Option<String>,
}

impl Attachment {
    pub(in crate::editor) fn new(server: ServerId, filter: FeatureFilter) -> Self {
        Self {
            server,
            filter,
            triggers: FxHashMap::default(),
            pull_result_id: None,
        }
    }
}

/// One text change not yet sent as `didChange`. `before` is the pre-change
/// rope (an O(1) clone via ropey's structural sharing); `version` is the
/// buffer text's generation after the change, the version the
/// notification claims.
pub(in crate::editor) struct PendingChange {
    pub(in crate::editor) cs: ChangeSet,
    pub(in crate::editor) before: Rope,
    pub(in crate::editor) version: u64,
}

/// What every server attached to a document received in its `didOpen`.
/// Every later notification names the document by this `uri`, whatever
/// the buffer's path is now, so a path change is a different `OpenedAs`
/// and closes the document under its old URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::editor) struct OpenedAs {
    pub(in crate::editor) language_id: String,
    pub(in crate::editor) uri: String,
}

/// A buffer's attachments, in its language's planned order, and its queued
/// changes. Never empty of attachments: see the module doc.
struct Document {
    opened: OpenedAs,
    attachments: Vec<Attachment>,
    pending: Vec<PendingChange>,
}

#[derive(Default)]
pub(in crate::editor) struct LspDocuments {
    docs: SecondaryMap<BufferId, Document>,
}

impl LspDocuments {
    /// Queues `change` for `bid`'s servers. A buffer no server is attached
    /// to records nothing, and neither does an identity change, which moves
    /// no text.
    pub(in crate::editor) fn record(&mut self, bid: BufferId, change: &TextChange<'_>) {
        if change.changes().is_identity() {
            return;
        }
        if let Some(doc) = self.docs.get_mut(bid) {
            doc.pending.push(PendingChange {
                cs: change.changes().clone(),
                before: change.before().rope().clone(),
                version: change.after().generation(),
            });
        }
    }

    /// `bid`'s attachments in planned order; empty when it has none.
    pub(in crate::editor) fn attachments(&self, bid: BufferId) -> &[Attachment] {
        self.docs
            .get(bid)
            .map_or(&[], |doc| doc.attachments.as_slice())
    }

    pub(in crate::editor) fn servers(&self, bid: BufferId) -> impl Iterator<Item = ServerId> + '_ {
        self.attachments(bid).iter().map(|a| a.server)
    }

    pub(in crate::editor) fn is_attached(&self, bid: BufferId, sid: ServerId) -> bool {
        self.attachments(bid).iter().any(|a| a.server == sid)
    }

    /// The filter `sid`'s attachment to `bid` carries, or `None` when `sid`
    /// is not attached to it.
    pub(in crate::editor) fn filter_of(
        &self,
        bid: BufferId,
        sid: ServerId,
    ) -> Option<FeatureFilter> {
        self.attachments(bid)
            .iter()
            .find(|a| a.server == sid)
            .map(|a| a.filter)
    }

    /// The `resultId` of `sid`'s last diagnostics report for `bid`.
    pub(in crate::editor::lsp) fn pull_result_id(
        &self,
        bid: BufferId,
        sid: ServerId,
    ) -> Option<&str> {
        self.attachments(bid)
            .iter()
            .find(|a| a.server == sid)?
            .pull_result_id
            .as_deref()
    }

    /// Replaces the `resultId` of `sid`'s last diagnostics report for `bid`.
    /// Does nothing when `sid` is not attached to `bid`.
    pub(in crate::editor::lsp) fn set_pull_result_id(
        &mut self,
        bid: BufferId,
        sid: ServerId,
        result_id: Option<String>,
    ) {
        if let Some(att) = self.attachment_mut(bid, sid) {
            att.pull_result_id = result_id;
        }
    }

    /// Every `(source, chars)` of `kind` registered on any of `bid`'s
    /// attachments.
    pub(in crate::editor) fn triggers(
        &self,
        bid: BufferId,
        kind: TriggerKind,
    ) -> impl Iterator<Item = (&str, &[char])> {
        self.attachments(bid)
            .iter()
            .flat_map(|a| a.triggers.iter())
            .filter(move |((k, _), _)| *k == kind)
            .map(|((_, source), chars)| (source.as_str(), chars.as_slice()))
    }

    /// Sets `source`'s characters of `kind` on `sid`'s attachment to `bid`,
    /// replacing its previous set; empty `chars` removes it. Does nothing
    /// when `sid` is not attached to `bid`.
    pub(in crate::editor) fn set_triggers(
        &mut self,
        bid: BufferId,
        sid: ServerId,
        kind: TriggerKind,
        source: String,
        chars: Vec<char>,
    ) {
        if let Some(att) = self.attachment_mut(bid, sid) {
            set_trigger_chars(&mut att.triggers, (kind, source), chars);
        }
    }

    /// Empties every attachment's trigger characters: the plugins that set them
    /// belong to an engine that is going away.
    pub(in crate::editor) fn clear_all_triggers(&mut self) {
        for att in self
            .docs
            .values_mut()
            .flat_map(|d| d.attachments.iter_mut())
        {
            att.triggers.clear();
        }
    }

    fn attachment_mut(&mut self, bid: BufferId, sid: ServerId) -> Option<&mut Attachment> {
        self.docs
            .get_mut(bid)?
            .attachments
            .iter_mut()
            .find(|a| a.server == sid)
    }

    /// Every buffer attached to `sid`.
    pub(in crate::editor) fn buffers_of(&self, sid: ServerId) -> Vec<BufferId> {
        self.docs
            .iter()
            .filter(|(_, doc)| doc.attachments.iter().any(|a| a.server == sid))
            .map(|(bid, _)| bid)
            .collect()
    }

    /// Whether any buffer is attached to `sid`.
    pub(in crate::editor) fn referenced(&self, sid: ServerId) -> bool {
        self.docs
            .values()
            .any(|doc| doc.attachments.iter().any(|a| a.server == sid))
    }

    pub(in crate::editor) fn has_doc(&self, bid: BufferId) -> bool {
        self.docs.contains_key(bid)
    }

    /// What `bid`'s servers opened it as, or `None` when it has no
    /// attachment.
    pub(in crate::editor::lsp) fn opened_as(&self, bid: BufferId) -> Option<&OpenedAs> {
        self.docs.get(bid).map(|doc| &doc.opened)
    }

    /// Every buffer with changes not yet sent.
    pub(in crate::editor::lsp) fn with_pending(&self) -> Vec<BufferId> {
        self.docs
            .iter()
            .filter(|(_, doc)| !doc.pending.is_empty())
            .map(|(bid, _)| bid)
            .collect()
    }

    pub(in crate::editor::lsp) fn take_pending(&mut self, bid: BufferId) -> Vec<PendingChange> {
        self.docs
            .get_mut(bid)
            .map(|doc| std::mem::take(&mut doc.pending))
            .unwrap_or_default()
    }

    /// Appends `attachment`, opened as `opened`. The caller has flushed
    /// `bid`'s queue first, so the new server never receives a change older
    /// than its `didOpen`.
    pub(in crate::editor::lsp) fn push(
        &mut self,
        bid: BufferId,
        attachment: Attachment,
        opened: &OpenedAs,
    ) {
        match self.docs.get_mut(bid) {
            Some(doc) => {
                debug_assert_eq!(&doc.opened, opened);
                doc.attachments.push(attachment);
            }
            None => {
                self.docs.insert(
                    bid,
                    Document {
                        opened: opened.clone(),
                        attachments: vec![attachment],
                        pending: Vec::new(),
                    },
                );
            }
        }
    }

    /// Removes `sid`'s attachment to `bid`; the document goes with its last
    /// attachment. `false` when `sid` was not attached to `bid`.
    pub(in crate::editor::lsp) fn remove(&mut self, bid: BufferId, sid: ServerId) -> bool {
        let Some(doc) = self.docs.get_mut(bid) else {
            return false;
        };
        let Some(index) = doc.attachments.iter().position(|a| a.server == sid) else {
            return false;
        };
        doc.attachments.remove(index);
        if doc.attachments.is_empty() {
            self.docs.remove(bid);
        }
        true
    }

    /// Replaces `sid`'s filter on `bid`. The trigger tables depend on what
    /// the filter admits, so they are emptied for the attach hook to
    /// register again.
    pub(in crate::editor::lsp) fn set_filter(
        &mut self,
        bid: BufferId,
        sid: ServerId,
        filter: FeatureFilter,
    ) {
        if let Some(att) = self.attachment_mut(bid, sid) {
            att.filter = filter;
            att.triggers.clear();
        }
    }

    /// Orders `bid`'s attachments as `order` lists them. `order` names
    /// the attached servers and no others.
    pub(in crate::editor::lsp) fn reorder(&mut self, bid: BufferId, order: &[ServerId]) {
        if let Some(doc) = self.docs.get_mut(bid) {
            debug_assert_eq!(doc.attachments.len(), order.len());
            doc.attachments
                .sort_by_key(|a| order.iter().position(|&s| s == a.server));
        }
    }
}

#[cfg(test)]
mod tests;
