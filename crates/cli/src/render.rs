//! Plain-text output: aligned tables (CSV, TSV and JSON come from `datadrop_core::export`).

use std::io::{self, Write};

use datadrop_core::TableKind;

/// Cells longer than this are cut short in tables (CSV and TSV keep everything).
const MAX_CELL: usize = 60;

/// A query plan as an indented tree, with estimates and (after ANALYZE) actuals.
pub fn plan(out: &mut impl Write, plan: &datadrop_core::Plan) -> io::Result<()> {
    fn num(v: f64) -> String {
        if v.fract() == 0.0 && v.abs() < 1e15 {
            format!("{}", v as i64)
        } else {
            format!("{v:.2}")
        }
    }
    fn node(
        out: &mut dyn Write,
        n: &datadrop_core::PlanNode,
        prefix: &str,
        last: bool,
        top: bool,
    ) -> io::Result<()> {
        let branch = if top {
            ""
        } else if last {
            "└─ "
        } else {
            "├─ "
        };
        let mut figures = vec![];
        if let Some(c) = n.estimated_cost {
            figures.push(format!("cost {}", num(c)));
        }
        if let Some(r) = n.estimated_rows {
            figures.push(format!("~{} rows", num(r)));
        }
        if let Some(r) = n.actual_rows {
            let loops = n
                .loops
                .filter(|l| *l > 1.0)
                .map(|l| format!(" ×{}", num(l)));
            figures.push(format!("{} rows{}", num(r), loops.unwrap_or_default()));
        }
        if let Some(ms) = n.actual_ms {
            figures.push(format!("{} ms", num(ms)));
        }
        let figures = if figures.is_empty() {
            String::new()
        } else {
            format!("  ({})", figures.join(", "))
        };
        writeln!(out, "{prefix}{branch}{}{figures}", n.label)?;
        let child_prefix = if top {
            prefix.to_owned()
        } else {
            format!("{prefix}{}", if last { "   " } else { "│  " })
        };
        for (k, v) in &n.details {
            if !n.label.contains(v.as_str()) {
                writeln!(out, "{child_prefix}   {k}: {v}")?;
            }
        }
        for (i, c) in n.children.iter().enumerate() {
            node(out, c, &child_prefix, i + 1 == n.children.len(), false)?;
        }
        Ok(())
    }
    for r in &plan.roots {
        node(out, r, "", true, true)?;
    }
    if let Some(ms) = plan.planning_ms {
        writeln!(out, "Planning: {} ms", num(ms))?;
    }
    if let Some(ms) = plan.execution_ms {
        writeln!(out, "Execution: {} ms", num(ms))?;
    }
    Ok(())
}

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
    fn long_cells_are_cut() {
        assert_eq!(shorten("abcdef", 4), "abc…");
        assert_eq!(one_line("select\n  *\nfrom t", 70), "select * from t");
    }
}
