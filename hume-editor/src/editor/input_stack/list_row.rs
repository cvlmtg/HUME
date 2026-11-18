//! A script's row display as a list row, and the rows a list's `#:render`
//! proc makes, shared by every list a script fills.

use std::sync::Arc;

use hume_core::{NonEmpty, TruncateEnd};
use hume_engine::theme::ScopeRegistry;
use hume_scripting::host::{RowDisplay, SetError, each_entry};
use hume_ui::list_row::{ListRow, SpanScopes};
use steel::rvals::SteelVal;

use crate::editor::Editor;
use crate::editor::host_impl::{EditorHostImpl, script_scope};

use super::stack::Layer;

/// A script's row display as a list row, its scopes interned in `registry`
/// through [`script_scope`]. Every row a script gives a drawer, menu or
/// picker, eager or rendered, is built here.
fn list_row(display: RowDisplay, registry: &mut ScopeRegistry) -> Result<ListRow, String> {
    let content = display.content.try_map_payload(|scopes| {
        Ok::<_, String>(SpanScopes {
            scope: script_scope(registry, &scopes.scope)?,
            selected: scopes
                .selected
                .map(|selected| script_scope(registry, &selected))
                .transpose()?,
        })
    })?;
    let selected_scope = display
        .selected_scope
        .map(|scope| script_scope(registry, &scope))
        .transpose()?;
    Ok(ListRow::styled(content).with_selected_scope(selected_scope))
}

/// The rows of a script's `(display . payload)` items, each display built by
/// [`list_row`]. The error names the first row that cannot be built by its
/// place in `items`.
pub(in crate::editor) fn script_rows<P>(
    items: NonEmpty<(RowDisplay, P)>,
    registry: &mut ScopeRegistry,
) -> Result<NonEmpty<(ListRow, P)>, SetError> {
    let rows = each_entry(items.into_vec(), |(display, payload)| {
        Ok((list_row(display, registry)?, payload))
    })?;
    Ok(NonEmpty::new(rows).expect("each item maps to one row"))
}

/// One call to a list's `#:render` proc: each row's item and its place in
/// the list the script gave.
pub(in crate::editor) struct RenderCall {
    render: hume_scripting::Callback,
    // steel-val-transient: built for one render call and moved into it.
    items: Vec<(usize, SteelVal)>,
    /// The query the rows answer, passed after the budget. `None` for a list
    /// with no query, whose render proc takes two arguments.
    query: Option<String>,
}

/// A list's `#:render` proc and the rows it has made, by their place in
/// the list.
pub(in crate::editor) struct RenderedRows {
    render: hume_scripting::Callback,
    rows: Vec<Option<RenderedRow>>,
}

/// A row `#:render` made, cut to `budget` cells. It counts only while the
/// list is still `budget` wide, so a resize renders the rows it shows again
/// without a pass clearing every row. Shared with the list's view, which
/// takes the visible rows every frame.
struct RenderedRow {
    budget: usize,
    row: Arc<ListRow>,
}

impl RenderedRows {
    /// `render`'s rows for a list of `len` rows, none made yet. A list that
    /// grows past `len` stores its later rows as they are made.
    pub(in crate::editor) fn new(render: hume_scripting::Callback, len: usize) -> Self {
        Self {
            render,
            rows: std::iter::repeat_with(|| None).take(len).collect(),
        }
    }

    /// The row made for the row at `place`, `budget` cells wide, `None` when
    /// none was made at that width.
    pub(in crate::editor) fn at(&self, place: usize, budget: usize) -> Option<&Arc<ListRow>> {
        self.rows
            .get(place)?
            .as_ref()
            .filter(|rendered| rendered.budget == budget)
            .map(|rendered| &rendered.row)
    }

    /// The call that makes the rows at `places` not made `budget` cells
    /// wide, each passed as `item(place)`, with the `query` they answer, if
    /// the list has one. `None` when every one was made.
    pub(in crate::editor) fn unrendered(
        &self,
        places: impl Iterator<Item = usize>,
        budget: usize,
        query: Option<&str>,
        mut item: impl FnMut(usize) -> SteelVal,
    ) -> Option<RenderCall> {
        let items: Vec<(usize, SteelVal)> = places
            .filter(|&place| self.at(place, budget).is_none())
            .map(|place| (place, item(place)))
            .collect();
        (!items.is_empty()).then(|| RenderCall {
            render: self.render.clone(),
            items,
            query: query.map(str::to_owned),
        })
    }

    /// Stores `rows`, each made `budget` cells wide for the row at its place.
    fn fill(&mut self, rows: Vec<(usize, ListRow)>, budget: usize) {
        for (place, row) in rows {
            if place >= self.rows.len() {
                self.rows.resize_with(place + 1, || None);
            }
            self.rows[place] = Some(RenderedRow {
                budget,
                row: Arc::new(row),
            });
        }
    }

    /// Forgets every row made, for a list whose rows were replaced or now
    /// answer another query.
    pub(in crate::editor) fn clear(&mut self) {
        self.rows.clear();
    }

    /// Each row made, by place, at whatever width it was made.
    #[cfg(test)]
    pub(in crate::editor) fn made(&self) -> impl Iterator<Item = Option<&ListRow>> {
        self.rows
            .iter()
            .map(|rendered| rendered.as_ref().map(|rendered| &*rendered.row))
    }
}

impl Editor {
    /// Renders `call`'s rows, each cut to `budget` cells at its `cut` end,
    /// and stores them in the [`RenderedRows`] that `rendered` reaches in
    /// the open `L` list. A render proc that fails, returns anything but one
    /// row display per item, or has no scripting host to call it is the
    /// error; the caller closes its list on it.
    pub(in crate::editor) fn render_rows_into<L: Layer>(
        &mut self,
        call: RenderCall,
        budget: usize,
        cut: TruncateEnd,
        rendered: impl FnOnce(&mut L) -> &mut RenderedRows,
    ) -> Result<(), String> {
        let Some(scripting) = self.scripting.as_mut() else {
            return Err("scripting is not available".to_owned());
        };
        let mut host = EditorHostImpl::new(&mut self.state, &mut self.view);
        let result = scripting.render_rows(
            &call.render,
            call.items,
            budget,
            call.query.as_deref(),
            &mut host,
        );
        self.flush_script_messages();
        let rows = result?
            .into_iter()
            .map(|(place, display)| {
                let row = list_row(display, &mut self.view.registry).map_err(|message| {
                    SetError::Entry {
                        index: place,
                        message,
                    }
                    .labeled("row")
                })?;
                Ok((place, row.fit(budget, cut)))
            })
            .collect::<Result<_, String>>()?;
        let list = self
            .state
            .input
            .find_mut::<L>()
            .expect("a render cannot close its list");
        rendered(list).fill(rows, budget);
        Ok(())
    }
}
