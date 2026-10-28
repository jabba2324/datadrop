//! The results grid's state: sorting and filtering, pending edits, and the review
//! before saving them.

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
    /// Sorted here (not by the server) by this column, descending if true.
    pub sort: Option<(usize, bool)>,
    /// The rows in display order (indexes into the result's rows), for `sort`.
    order: Option<((usize, bool), Vec<usize>)>,
}

impl Default for GridEdit {
    fn default() -> Self {
        Self {
            result: None,
            editability: Editability::Unknown,
            edits: Edits::default(),
            editing: None,
            review: None,
            sort: None,
            order: None,
        }
    }
}

/// A tab's results filtered and sorted by the server: its query runs again wrapped
/// in `SELECT * FROM (...) WHERE ... ORDER BY ...`.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    /// The query as written.
    pub base: String,
    /// The condition being typed.
    pub text: String,
    /// The condition in effect.
    pub applied: String,
    /// Column and descending.
    pub order: Option<(String, bool)>,
    /// The SQL that ran (the wrapped query), to recognise its result.
    pub wrapped: String,
}

/// The order to show rows in, sorted by column `col`: numbers as numbers, NULLs last
/// (first when descending), ties in their original order.
pub fn sort_order(rows: &[Vec<Option<String>>], col: usize, desc: bool) -> Vec<usize> {
    use std::cmp::Ordering;
    let mut order: Vec<usize> = (0..rows.len()).collect();
    let key = |i: usize| rows[i].get(col).cloned().flatten();
    order.sort_by(|&a, &b| {
        let o = match (key(a), key(b)) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(x), Some(y)) => match (x.trim().parse::<f64>(), y.trim().parse::<f64>()) {
                (Ok(p), Ok(q)) => p.partial_cmp(&q).unwrap_or(Ordering::Equal),
                _ => x.cmp(&y),
            },
        };
        if desc { o.reverse() } else { o }
    });
    order
}

impl GridEdit {
    pub fn for_result(result: usize) -> Self {
        Self {
            result: Some(result),
            ..Self::default()
        }
    }

    /// Row `d` as shown: which row of the result it is.
    pub fn row_at(&mut self, rows: &[Vec<Option<String>>], d: usize) -> usize {
        let Some(sort) = self.sort else { return d };
        if d >= rows.len() {
            return d;
        }
        if self
            .order
            .as_ref()
            .is_none_or(|(s, o)| *s != sort || o.len() != rows.len())
        {
            self.order = Some((sort, sort_order(rows, sort.0, sort.1)));
        }
        self.order.as_ref().map_or(d, |(_, o)| o[d])
    }

    /// Clicking a header: ascending, then descending, then as it came.
    pub fn cycle_sort(&mut self, col: usize) {
        self.sort = match self.sort {
            Some((c, false)) if c == col => Some((col, true)),
            Some((c, true)) if c == col => None,
            _ => Some((col, false)),
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(values: &[Option<&str>]) -> Vec<Vec<Option<String>>> {
        values.iter().map(|v| vec![v.map(Into::into)]).collect()
    }

    #[test]
    fn sorts_numbers_text_and_nulls() {
        let r = rows(&[Some("10"), None, Some("9"), Some("-1.5"), Some("10")]);
        assert_eq!(sort_order(&r, 0, false), [3, 2, 0, 4, 1]);
        assert_eq!(sort_order(&r, 0, true), [1, 0, 4, 2, 3]);
        let r = rows(&[Some("pear"), Some("apple"), Some("10")]);
        assert_eq!(sort_order(&r, 0, false), [2, 1, 0]);
    }

    #[test]
    fn header_clicks_cycle() {
        let mut g = GridEdit::default();
        g.cycle_sort(1);
        assert_eq!(g.sort, Some((1, false)));
        g.cycle_sort(1);
        assert_eq!(g.sort, Some((1, true)));
        g.cycle_sort(1);
        assert_eq!(g.sort, None);
        g.cycle_sort(1);
        g.cycle_sort(2);
        assert_eq!(g.sort, Some((2, false)));
    }
}
