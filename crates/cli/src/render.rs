//! Plain-text output: aligned tables, CSV and TSV.

use std::io::{self, Write};

use datadrop_core::TableKind;

/// Cells longer than this are cut short in tables (CSV and TSV keep everything).
const MAX_CELL: usize = 60;

pub fn kind(k: TableKind) -> &'static str {
    match k {
        TableKind::Table => "table",
        TableKind::View => "view",
        TableKind::MaterializedView => "materialized view",
        TableKind::ForeignTable => "foreign table",
    }
}

/// The text on one line, at most `max` characters.
pub fn one_line(s: &str, max: usize) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    shorten(&flat, max)
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

fn cell(v: &Option<String>) -> String {
    match v {
        None => "NULL".into(),
        Some(s) => shorten(&s.replace(['\n', '\r', '\t'], " "), MAX_CELL),
    }
}

pub fn table(
    out: &mut impl Write,
    headers: &[&str],
    rows: &[Vec<Option<String>>],
) -> io::Result<()> {
    let cells: Vec<Vec<String>> = rows.iter().map(|r| r.iter().map(cell).collect()).collect();
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for r in &cells {
        for (w, c) in widths.iter_mut().zip(r) {
            *w = (*w).max(c.chars().count());
        }
    }
    let line = |out: &mut dyn Write, values: &mut dyn Iterator<Item = &str>| -> io::Result<()> {
        let mut first = true;
        let mut s = String::new();
        for (v, w) in values.zip(&widths) {
            if !first {
                s.push_str("  ");
            }
            first = false;
            s.push_str(v);
            s.extend(std::iter::repeat_n(' ', w - v.chars().count()));
        }
        writeln!(out, "{}", s.trim_end())
    };
    line(out, &mut headers.iter().copied())?;
    let rule: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
    line(out, &mut rule.iter().map(String::as_str))?;
    for r in &cells {
        line(out, &mut r.iter().map(String::as_str))?;
    }
    Ok(())
}

/// CSV (RFC 4180 quoting) or TSV. NULL is an empty field.
pub fn delimited(
    out: &mut impl Write,
    headers: &[&str],
    rows: &[Vec<Option<String>>],
    sep: u8,
) -> io::Result<()> {
    let field = |s: &str| -> String {
        let special = s
            .bytes()
            .any(|b| b == sep || b == b'"' || b == b'\n' || b == b'\r');
        if special {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_owned()
        }
    };
    let sep = sep as char;
    let head: Vec<String> = headers.iter().map(|h| field(h)).collect();
    writeln!(out, "{}", head.join(&sep.to_string()))?;
    for r in rows {
        let line: Vec<String> = r
            .iter()
            .map(|v| v.as_deref().map(field).unwrap_or_default())
            .collect();
        writeln!(out, "{}", line.join(&sep.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Option<String> {
        Some(v.to_owned())
    }

    #[test]
    fn aligned_table() {
        let mut out = vec![];
        table(
            &mut out,
            &["id", "name"],
            &[vec![s("1"), s("café")], vec![s("22"), None]],
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "id  name\n--  ----\n1   café\n22  NULL\n"
        );
    }

    #[test]
    fn csv_quoting() {
        let mut out = vec![];
        delimited(
            &mut out,
            &["a", "b"],
            &[vec![s("x,y"), s("say \"hi\"")], vec![None, s("2")]],
            b',',
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "a,b\n\"x,y\",\"say \"\"hi\"\"\"\n,2\n"
        );
    }

    #[test]
    fn long_cells_are_cut() {
        assert_eq!(shorten("abcdef", 4), "abc…");
        assert_eq!(one_line("select\n  *\nfrom t", 70), "select * from t");
    }
}
