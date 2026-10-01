mod align;
mod case;
mod corpus;
mod delete;
mod indent;
mod insert;
mod join;
mod paste;
mod replace;
mod sort;

use super::*;
use hume_editing::{selection::SelectionSet, text::BufferText};

impl test_fixtures::testing::IntoTestResult for Removal {
    fn into_test_result(self) -> (BufferText, SelectionSet) {
        self.edited.into_test_result()
    }
}
