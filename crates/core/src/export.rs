//! Writing results as CSV, TSV or JSON, row by row (so exports of any size stream
//! straight to the file).

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    /// RFC 4180: fields with commas, quotes or line breaks are quoted. NULL is empty.
    Csv,
    /// Tab-separated; tabs and line breaks inside values become spaces.
    Tsv,
    /// An array of objects, one per row, keyed by column name. NULL is null; every
    /// other value is a string (the database's text for it).
    Json,
}

impl ExportFormat {
    /// From a file name's extension (`.csv`, `.tsv`/`.tab`, `.json`).
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "csv" => Some(Self::Csv),
            "tsv" | "tab" | "txt" => Some(Self::Tsv),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

pub struct ExportWriter<W: Write> {
    out: W,
    format: ExportFormat,
    /// JSON keys (column names, made unique).
    keys: Vec<String>,
    started: bool,
    rows: u64,
}

impl<W: Write> ExportWriter<W> {
    pub fn new(out: W, format: ExportFormat) -> Self {
        Self {
            out,
            format,
            keys: vec![],
            started: false,
            rows: 0,
        }
    }

    /// The header. Call once, before the rows.
    pub fn columns(&mut self, names: &[&str]) -> io::Result<()> {
        self.started = true;
        match self.format {
            ExportFormat::Csv => self.line(names.iter().map(|n| Some(*n)), b','),
            ExportFormat::Tsv => self.line(names.iter().map(|n| Some(*n)), b'\t'),
            ExportFormat::Json => {
                self.keys = unique_keys(names);
                self.out.write_all(b"[")
            }
        }
    }

    pub fn row(&mut self, values: &[Option<String>]) -> io::Result<()> {
        let fields = values.iter().map(Option::as_deref);
        match self.format {
            ExportFormat::Csv => self.line(fields, b',')?,
            ExportFormat::Tsv => self.line(fields, b'\t')?,
            ExportFormat::Json => {
                let obj: serde_json::Map<String, serde_json::Value> = self
                    .keys
                    .iter()
                    .cloned()
                    .zip(values.iter().map(|v| match v {
                        Some(s) => serde_json::Value::String(s.clone()),
                        None => serde_json::Value::Null,
                    }))
                    .collect();
                if self.rows > 0 {
                    self.out.write_all(b",")?;
                }
                self.out.write_all(b"\n  ")?;
                serde_json::to_writer(&mut self.out, &obj)?;
            }
        }
        self.rows += 1;
        Ok(())
    }

    /// Finish the file; returns the number of rows written.
    pub fn finish(mut self) -> io::Result<u64> {
        if self.format == ExportFormat::Json {
            if !self.started {
                self.out.write_all(b"[")?;
            }
            self.out
                .write_all(if self.rows > 0 { b"\n]\n" } else { b"]\n" })?;
        }
        self.out.flush()?;
        Ok(self.rows)
    }

    fn line<'a>(
        &mut self,
        fields: impl Iterator<Item = Option<&'a str>>,
        sep: u8,
    ) -> io::Result<()> {
        let mut first = true;
        for f in fields {
            if !first {
                self.out.write_all(&[sep])?;
            }
            first = false;
            let Some(f) = f else { continue };
            if sep == b'\t' {
                self.out
                    .write_all(f.replace(['\t', '\n', '\r'], " ").as_bytes())?;
            } else if f
                .bytes()
                .any(|b| b == sep || b == b'"' || b == b'\n' || b == b'\r')
            {
                write!(self.out, "\"{}\"", f.replace('"', "\"\""))?;
            } else {
                self.out.write_all(f.as_bytes())?;
            }
        }
        self.out.write_all(b"\n")
    }
}

/// Column names as JSON keys: a repeated name gets `_2`, `_3`...
fn unique_keys(names: &[&str]) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    names
        .iter()
        .map(|n| {
            let count = seen.entry((*n).to_owned()).or_insert(0);
            *count += 1;
            if *count == 1 {
                (*n).to_owned()
            } else {
                format!("{n}_{count}")
            }
        })
        .collect()
}

/// Write a whole result held in memory.
pub fn write_all(
    out: impl Write,
    format: ExportFormat,
    columns: &[&str],
    rows: &[Vec<Option<String>>],
) -> io::Result<u64> {
    let mut w = ExportWriter::new(out, format);
    w.columns(columns)?;
    for r in rows {
        w.row(r)?;
    }
    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Option<String> {
        Some(v.to_owned())
    }

    fn render(format: ExportFormat) -> String {
        let mut out = vec![];
        let n = write_all(
            &mut out,
            format,
            &["id", "note", "id"],
            &[
                vec![s("1"), s("say \"hi\", ok"), None],
                vec![s("2"), s("tab\there"), s("x")],
            ],
        )
        .unwrap();
        assert_eq!(n, 2);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn csv() {
        assert_eq!(
            render(ExportFormat::Csv),
            "id,note,id\n1,\"say \"\"hi\"\", ok\",\n2,tab\there,x\n"
        );
    }

    #[test]
    fn tsv() {
        assert_eq!(
            render(ExportFormat::Tsv),
            "id\tnote\tid\n1\tsay \"hi\", ok\t\n2\ttab here\tx\n"
        );
    }

    #[test]
    fn json() {
        let v: serde_json::Value = serde_json::from_str(&render(ExportFormat::Json)).unwrap();
        assert_eq!(
            v,
            serde_json::json!([
                {"id": "1", "note": "say \"hi\", ok", "id_2": null},
                {"id": "2", "note": "tab\there", "id_2": "x"}
            ])
        );
        // Empty results are still valid JSON.
        let mut out = vec![];
        write_all(&mut out, ExportFormat::Json, &["a"], &[]).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "[]\n");
    }

    #[test]
    fn format_from_extension() {
        assert_eq!(
            ExportFormat::from_path(Path::new("a/b.CSV")),
            Some(ExportFormat::Csv)
        );
        assert_eq!(
            ExportFormat::from_path(Path::new("b.json")),
            Some(ExportFormat::Json)
        );
        assert_eq!(ExportFormat::from_path(Path::new("b")), None);
    }
}
