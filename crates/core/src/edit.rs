//! Editing results: which results can be edited, and the SQL that saves the edits.
//!
//! Only a plain `SELECT ... FROM one_table` whose rows can be identified by the
//! table's primary key is editable. Edits become `UPDATE`, `INSERT` and `DELETE`
//! statements that are shown before they run, and run in one transaction.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::complete;
use crate::error::{CoreError, Result};
use crate::sql::{self, Dialect, TokenKind};

/// The table a result came from, if editing it is possible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceTable {
    pub schema: Option<String>,
    pub table: String,
}

/// The single table a statement reads, when it's simple enough to edit its rows:
/// a SELECT from one table, without joins, grouping, DISTINCT or set operations.
pub fn source_table(statement: &str, dialect: Dialect) -> Option<SourceTable> {
    if sql::first_keyword(statement, dialect).as_deref() != Some("SELECT") {
        return None;
    }
    let tokens = sql::tokenize(statement, dialect);
    let words: Vec<String> = tokens
        .iter()
        .filter(|t| matches!(t.kind, TokenKind::Keyword | TokenKind::Identifier))
        .map(|t| statement[t.range.clone()].to_ascii_uppercase())
        .collect();
    const NOT_EDITABLE: &[&str] = &[
        "JOIN",
        "GROUP",
        "DISTINCT",
        "UNION",
        "EXCEPT",
        "INTERSECT",
        "HAVING",
        "WINDOW",
        "OVER",
    ];
    if words.iter().any(|w| NOT_EDITABLE.contains(&w.as_str())) {
        return None;
    }
    // Exactly one FROM, at the top level (no subqueries).
    let depth_ok = tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Punctuation && &statement[t.range.clone()] == "(")
        .all(|open| {
            // A bracket is fine unless a SELECT is inside it.
            let rest = &statement[open.range.end..];
            !rest
                .split(')')
                .next()
                .is_some_and(|inner| inner.to_ascii_uppercase().contains("SELECT"))
        });
    if !depth_ok || words.iter().filter(|w| *w == "FROM").count() != 1 {
        return None;
    }
    let refs: Vec<&sql::Token> = tokens.iter().collect();
    let found = complete::table_refs(statement, &refs);
    match found.as_slice() {
        [one] => Some(SourceTable {
            schema: one.schema.clone(),
            table: one.table.clone(),
        }),
        _ => None,
    }
}

/// Changes to a result, by row and column position.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edits {
    /// (row, column) -> new value (`None` is NULL).
    pub cells: BTreeMap<(usize, usize), Option<String>>,
    pub deleted: BTreeSet<usize>,
    /// New rows: column -> value. Columns not set get their default.
    pub inserted: Vec<BTreeMap<usize, Option<String>>>,
}

impl Edits {
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
            && self.deleted.is_empty()
            && self.inserted.iter().all(|r| r.is_empty())
    }

    pub fn count(&self) -> usize {
        let rows: BTreeSet<usize> = self
            .cells
            .keys()
            .map(|(r, _)| *r)
            .filter(|r| !self.deleted.contains(r))
            .collect();
        rows.len() + self.deleted.len() + self.inserted.iter().filter(|r| !r.is_empty()).count()
    }
}

/// A value as an SQL literal for the dialect: `'text'` with the needed escapes, or
/// `NULL`. The database converts it to the column's type, as it does for any
/// quoted literal.
pub fn literal(value: Option<&str>, dialect: Dialect) -> String {
    match value {
        None => "NULL".into(),
        // N'...', so text outside the database's code page survives.
        Some(v) if dialect == Dialect::SqlServer => format!("N'{}'", v.replace('\'', "''")),
        Some(v) => {
            let mut s = v.replace('\'', "''");
            if dialect == Dialect::Mysql {
                // MySQL also treats backslashes as escapes in strings.
                s = s.replace('\\', "\\\\");
            }
            format!("'{s}'")
        }
    }
}

/// The statements that save `edits` to `table`, whose primary key is `key`.
/// `columns` and `rows` are the result as shown.
pub fn statements(
    dialect: Dialect,
    table: &SourceTable,
    key: &[String],
    columns: &[String],
    rows: &[Vec<Option<String>>],
    edits: &Edits,
) -> Result<Vec<String>> {
    let q = |name: &str| sql::quote_ident(name, dialect);
    let target = match &table.schema {
        Some(s) => format!("{}.{}", q(s), q(&table.table)),
        None => q(&table.table),
    };
    if key.is_empty() {
        return Err(CoreError::Invalid(format!(
            "{} has no primary key, so its rows can't be told apart to save changes",
            table.table
        )));
    }
    let key_pos: Vec<usize> = key
        .iter()
        .map(|k| {
            columns.iter().position(|c| c == k).ok_or_else(|| {
                CoreError::Invalid(format!(
                    "the result needs the key column {k} to save changes; add it to the SELECT"
                ))
            })
        })
        .collect::<Result<_>>()?;
    let where_key = |row: usize| -> Result<String> {
        let values = rows
            .get(row)
            .ok_or_else(|| CoreError::Invalid(format!("there's no row {}", row + 1)))?;
        let parts: Vec<String> = key
            .iter()
            .zip(&key_pos)
            .map(|(k, &i)| match &values[i] {
                Some(v) => Ok(format!("{} = {}", q(k), literal(Some(v), dialect))),
                None => Err(CoreError::Invalid(format!(
                    "row {} has no value in the key column {k}",
                    row + 1
                ))),
            })
            .collect::<Result<_>>()?;
        Ok(parts.join(" AND "))
    };

    let mut out = vec![];
    // Updates, a statement per row, skipping cells set back to what they were.
    let mut by_row: BTreeMap<usize, Vec<(usize, &Option<String>)>> = BTreeMap::new();
    for ((row, col), value) in &edits.cells {
        if edits.deleted.contains(row) {
            continue;
        }
        if rows.get(*row).and_then(|r| r.get(*col)) == Some(value) {
            continue;
        }
        by_row.entry(*row).or_default().push((*col, value));
    }
    for (row, changes) in by_row {
        let set: Vec<String> = changes
            .iter()
            .map(|(col, v)| format!("{} = {}", q(&columns[*col]), literal(v.as_deref(), dialect)))
            .collect();
        out.push(format!(
            "UPDATE {target} SET {} WHERE {}",
            set.join(", "),
            where_key(row)?
        ));
    }
    for row in &edits.deleted {
        out.push(format!("DELETE FROM {target} WHERE {}", where_key(*row)?));
    }
    for new in &edits.inserted {
        if new.is_empty() {
            continue;
        }
        let names: Vec<String> = new.keys().map(|c| q(&columns[*c])).collect();
        let values: Vec<String> = new
            .values()
            .map(|v| literal(v.as_deref(), dialect))
            .collect();
        out.push(format!(
            "INSERT INTO {target} ({}) VALUES ({})",
            names.join(", "),
            values.join(", ")
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(sql: &str) -> Option<(Option<String>, String)> {
        source_table(sql, Dialect::Postgres).map(|t| (t.schema, t.table))
    }

    #[test]
    fn which_results_are_editable() {
        assert_eq!(src("select * from orders"), Some((None, "orders".into())));
        assert_eq!(
            src("SELECT id, total FROM shop.orders o WHERE total > 1 ORDER BY id LIMIT 5"),
            Some((Some("shop".into()), "orders".into()))
        );
        assert_eq!(
            src("select * from \"Odd Name\""),
            Some((None, "Odd Name".into()))
        );
        for not in [
            "select * from a join b on a.id = b.id",
            "select * from a, b",
            "select count(*) from a group by x",
            "select distinct x from a",
            "select * from a union select * from b",
            "select * from (select * from a) s",
            "select * from a where id in (select id from b)",
            "with x as (select 1) select * from x",
            "update a set x = 1",
            "select 1",
        ] {
            assert_eq!(src(not), None, "{not}");
        }
    }

    #[test]
    fn literals() {
        assert_eq!(literal(None, Dialect::Postgres), "NULL");
        assert_eq!(literal(Some("it's"), Dialect::Postgres), "'it''s'");
        assert_eq!(literal(Some("a\\b"), Dialect::Postgres), "'a\\b'");
        assert_eq!(literal(Some("a\\b'"), Dialect::Mysql), "'a\\\\b'''");
    }

    #[test]
    fn generates_statements() {
        let table = SourceTable {
            schema: Some("shop".into()),
            table: "orders".into(),
        };
        let columns: Vec<String> = ["id", "note", "total"].map(String::from).to_vec();
        let rows = vec![
            vec![Some("1".into()), Some("a".into()), Some("10".into())],
            vec![Some("2".into()), None, Some("20".into())],
            vec![Some("3".into()), Some("c".into()), Some("30".into())],
        ];
        let mut e = Edits::default();
        e.cells.insert((0, 1), Some("it's".into()));
        e.cells.insert((0, 2), None);
        e.cells.insert((1, 2), Some("20".into())); // unchanged: no statement
        e.cells.insert((2, 1), Some("gone".into())); // the row is deleted anyway
        e.deleted.insert(2);
        e.inserted
            .push(BTreeMap::from([(0, Some("4".into())), (1, None)]));
        e.inserted.push(BTreeMap::new()); // left empty: nothing
        assert_eq!(e.count(), 4);
        let got = statements(
            Dialect::Postgres,
            &table,
            &["id".into()],
            &columns,
            &rows,
            &e,
        )
        .unwrap();
        assert_eq!(
            got,
            [
                "UPDATE \"shop\".\"orders\" SET \"note\" = 'it''s', \"total\" = NULL WHERE \"id\" = '1'",
                "DELETE FROM \"shop\".\"orders\" WHERE \"id\" = '3'",
                "INSERT INTO \"shop\".\"orders\" (\"id\", \"note\") VALUES ('4', NULL)",
            ]
        );
        // The key has to be in the result, and have values.
        let err = statements(
            Dialect::Postgres,
            &table,
            &["uuid".into()],
            &columns,
            &rows,
            &e,
        )
        .unwrap_err();
        assert!(err.to_string().contains("key column uuid"));
        let err = statements(Dialect::Postgres, &table, &[], &columns, &rows, &e).unwrap_err();
        assert!(err.to_string().contains("no primary key"));
    }
}
