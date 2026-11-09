/// A compiled `context.scm` query: the scopes whose first line a pane shows
/// above its text once that line has scrolled off. `@context` captures a
/// scope's node; `@context.start` optionally names the node whose first line
/// is the scope's header instead.
#[expect(dead_code)]
pub struct ContextQuery {
    query: tree_sitter::Query,
    context: Option<u32>,
    start: Option<u32>,
}

impl ContextQuery {
    pub(crate) fn new(query: tree_sitter::Query) -> Self {
        let context = query.capture_index_for_name("context");
        let start = query.capture_index_for_name("context.start");
        Self {
            query,
            context,
            start,
        }
    }
}

#[cfg(test)]
mod tests;
