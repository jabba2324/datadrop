//! Editing a result in the grid: pending changes, and the review before saving.

use datadrop_core::edit::{Edits, SourceTable};

use crate::backend::Task;

pub enum Editability {
    /// Not looked at yet.
    Unknown,
    /// Reading the table's primary key.
    Checking(Option<Task>, SourceTable),
    Editable {
        source: SourceTable,
        key: Vec<String>,
    },
    /// Why not.
    ReadOnly(String),
}

/// A cell being typed in.
pub struct CellEditor {
    /// A row of the result, or (counting on from its last row) a new row.
    pub row: usize,
    pub col: usize,
    pub buffer: String,
    pub focus: bool,
}

pub struct Review {
    pub statements: Vec<String>,
    pub saving: Option<Task>,
    pub error: Option<String>,
    /// The query to run again once saved, to show the table as it now is.
    pub refresh: String,
}

pub struct GridEdit {
    /// Which result (an index into the output's results) this is for.
    pub result: Option<usize>,
    pub editability: Editability,
    pub edits: Edits,
    pub editing: Option<CellEditor>,
    pub review: Option<Review>,
}

impl Default for GridEdit {
    fn default() -> Self {
        Self {
            result: None,
            editability: Editability::Unknown,
            edits: Edits::default(),
            editing: None,
            review: None,
        }
    }
}

impl GridEdit {
    pub fn for_result(result: usize) -> Self {
        Self {
            result: Some(result),
            ..Self::default()
        }
    }

    pub fn is_editable(&self) -> bool {
        matches!(self.editability, Editability::Editable { .. })
    }

    /// Store what was typed in the cell being edited.
    pub fn commit(&mut self, original: &[Vec<Option<String>>]) {
        let Some(e) = self.editing.take() else { return };
        self.set(e.row, e.col, Some(e.buffer), original);
    }

    /// Change a cell (of an existing row or a new one).
    pub fn set(
        &mut self,
        row: usize,
        col: usize,
        value: Option<String>,
        original: &[Vec<Option<String>>],
    ) {
        let n = original.len();
        if row >= n {
            if let Some(new) = self.edits.inserted.get_mut(row - n) {
                new.insert(col, value);
            }
        } else if original[row].get(col) == Some(&value) {
            // Back to how it was: not a change.
            self.edits.cells.remove(&(row, col));
        } else {
            self.edits.cells.insert((row, col), value);
        }
    }
}
