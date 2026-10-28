//! Reading CSV, TSV and JSON files to import into a table.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::edit::literal;
use crate::error::{CoreError, Result};
use crate::export::ExportFormat;
use crate::schema::{column_list, qualified};
use crate::sql::Dialect;

/// A file's column names and rows.
pub type Table = (Vec<String>, Vec<Vec<Option<String>>>);

/// Rows per INSERT statement.
pub const BATCH: usize = 200;

/// A file's columns and its first rows, to choose how to import it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportPreview {
    pub format: ExportFormat,
    /// The header's names, or `column1`, `column2`... without one.
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
}

/// How to read a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadOptions {
    pub format: ExportFormat,
    /// CSV/TSV: the first row names the columns.
    pub header: bool,
    /// Empty CSV/TSV fields are NULL (rather than empty text).
    pub empty_as_null: bool,
}

/// The format a file is in: from its extension, else CSV.
pub fn format_of(path: &Path, given: Option<ExportFormat>) -> ExportFormat {
    given
        .or_else(|| ExportFormat::from_path(path))
        .unwrap_or(ExportFormat::Csv)
}

/// Read a whole file: its column names, and every row.
pub fn read(path: &Path, opts: ReadOptions) -> Result<Table> {
    let ctx = |e: std::io::Error| CoreError::io(format!("reading {}", path.display()), e);
    match opts.format {
        ExportFormat::Json => {
            let file = std::fs::File::open(path).map_err(ctx)?;
            read_json(std::io::BufReader::new(file))
        }
        ExportFormat::Csv | ExportFormat::Tsv => {
            let file = std::fs::File::open(path).map_err(ctx)?;
            read_delimited(file, opts)
        }
    }
}

pub fn preview(path: &Path, opts: ReadOptions, rows: usize) -> Result<ImportPreview> {
    let (columns, mut all) = read(path, opts)?;
    all.truncate(rows);
    Ok(ImportPreview {
        format: opts.format,
        columns,
        rows: all,
    })
}

fn read_delimited(input: impl std::io::Read, opts: ReadOptions) -> Result<Table> {
    let mut r = csv::ReaderBuilder::new()
        .delimiter(if opts.format == ExportFormat::Tsv {
            b'\t'
        } else {
            b','
        })
        .has_headers(opts.header)
        .flexible(true)
        .from_reader(input);
    let bad = |e: csv::Error| CoreError::Invalid(format!("the file isn't valid CSV: {e}"));
    let mut columns: Vec<String> = if opts.header {
        r.headers()
            .map_err(bad)?
            .iter()
            .map(|h| h.trim_start_matches('\u{feff}').to_owned())
            .collect()
    } else {
        vec![]
    };
    let mut rows = vec![];
    for record in r.records() {
        let record = record.map_err(bad)?;
        let row: Vec<Option<String>> = record
            .iter()
            .map(|v| (!(opts.empty_as_null && v.is_empty())).then(|| v.to_owned()))
            .collect();
        while columns.len() < row.len() {
            columns.push(format!("column{}", columns.len() + 1));
        }
        rows.push(row);
    }
    for row in &mut rows {
        row.resize(columns.len(), None);
    }
    Ok((columns, rows))
}

/// An array of objects (as exported), keyed by column name. Values that aren't
/// strings are written as JSON (`12`, `true`, `{"a":1}`); null is NULL.
fn read_json(input: impl std::io::Read) -> Result<Table> {
    let value: serde_json::Value = serde_json::from_reader(input)
        .map_err(|e| CoreError::Invalid(format!("the file isn't valid JSON: {e}")))?;
    let serde_json::Value::Array(items) = value else {
        return Err(CoreError::Invalid(
            "the JSON should be an array of objects, one per row".into(),
        ));
    };
    let mut columns: Vec<String> = vec![];
    for item in &items {
        let serde_json::Value::Object(obj) = item else {
            return Err(CoreError::Invalid(
                "the JSON should be an array of objects, one per row".into(),
            ));
        };
        for k in obj.keys() {
            if !columns.contains(k) {
                columns.push(k.clone());
            }
        }
    }
    let rows = items
        .iter()
        .map(|item| {
            columns
                .iter()
                .map(|c| match item.get(c) {
                    None | Some(serde_json::Value::Null) => None,
                    Some(serde_json::Value::String(s)) => Some(s.clone()),
                    Some(v) => Some(v.to_string()),
                })
                .collect()
        })
        .collect();
    Ok((columns, rows))
}

/// The INSERT statements for `rows`, [`BATCH`] rows each. `targets[i]` is the table
/// column the file's column `i` goes into (`None` to leave it out).
pub fn insert_statements(
    schema: &str,
    table: &str,
    targets: &[Option<String>],
    rows: &[Vec<Option<String>>],
    dialect: Dialect,
) -> Result<Vec<String>> {
    let used: Vec<usize> = (0..targets.len())
        .filter(|&i| targets[i].is_some())
        .collect();
    if used.is_empty() {
        return Err(CoreError::Invalid(
            "choose at least one column to import".into(),
        ));
    }
    let names: Vec<String> = used
        .iter()
        .map(|&i| targets[i].clone().expect("used"))
        .collect();
    let head = format!(
        "INSERT INTO {} ({}) VALUES ",
        qualified(schema, table, dialect),
        column_list(&names, dialect)
    );
    Ok(rows
        .chunks(BATCH)
        .map(|chunk| {
            let values: Vec<String> = chunk
                .iter()
                .map(|row| {
                    let vals: Vec<String> = used
                        .iter()
                        .map(|&i| literal(row.get(i).cloned().flatten().as_deref(), dialect))
                        .collect();
                    format!("({})", vals.join(", "))
                })
                .collect();
            format!("{head}{}", values.join(", "))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(format: ExportFormat) -> ReadOptions {
        ReadOptions {
            format,
            header: true,
            empty_as_null: true,
        }
    }

    #[test]
    fn reads_csv_with_quotes_and_line_breaks() {
        let csv =
            "\u{feff}id,name,note\n1,\"Smith, J\",\"two\nlines\"\n2,,\"say \"\"hi\"\"\"\n3,x\n";
        let (cols, rows) = read_delimited(csv.as_bytes(), opts(ExportFormat::Csv)).unwrap();
        assert_eq!(cols, ["id", "name", "note"]);
        assert_eq!(rows[0][1].as_deref(), Some("Smith, J"));
        assert_eq!(rows[0][2].as_deref(), Some("two\nlines"));
        assert_eq!(rows[1][1], None);
        assert_eq!(rows[1][2].as_deref(), Some("say \"hi\""));
        assert_eq!(rows[2], [Some("3".into()), Some("x".into()), None]);

        let no_header = ReadOptions {
            header: false,
            ..opts(ExportFormat::Tsv)
        };
        let (cols, rows) = read_delimited("a\tb\n".as_bytes(), no_header).unwrap();
        assert_eq!(cols, ["column1", "column2"]);
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn reads_json_arrays() {
        let json =
            r#"[{"id": 1, "name": "a", "tags": ["x"]}, {"id": 2, "name": null, "extra": true}]"#;
        let (cols, rows) = read_json(json.as_bytes()).unwrap();
        assert_eq!(cols, ["id", "name", "tags", "extra"]);
        assert_eq!(
            rows[0],
            [
                Some("1".into()),
                Some("a".into()),
                Some("[\"x\"]".into()),
                None
            ]
        );
        assert_eq!(rows[1][1], None);
        assert_eq!(rows[1][3].as_deref(), Some("true"));
        assert!(read_json("{}".as_bytes()).is_err());
    }

    #[test]
    fn batches_inserts() {
        let rows: Vec<Vec<Option<String>>> = (0..450)
            .map(|i| vec![Some(i.to_string()), Some("O'k".into()), None])
            .collect();
        let targets = [Some("id".into()), Some("name".into()), None];
        let st = insert_statements("public", "t", &targets, &rows, Dialect::Postgres).unwrap();
        assert_eq!(st.len(), 3);
        assert!(st[0].starts_with(
            "INSERT INTO \"public\".\"t\" (\"id\", \"name\") VALUES ('0', 'O''k'), ('1', 'O''k')"
        ));
        assert!(insert_statements("s", "t", &[None], &rows, Dialect::Postgres).is_err());
    }
}
