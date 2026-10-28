//! Query parameters: `:name`, `?` and `$1` placeholders, filled in with values asked
//! for before the query runs.

use std::collections::HashMap;
use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::edit::literal;
use crate::sql::{Dialect, TokenKind, tokenize};

/// One placeholder, and everywhere it appears.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    /// `:id`, `$1`; a bare `?` is `?1`, `?2`... by position.
    pub name: String,
    pub ranges: Vec<Range<usize>>,
}

/// How a value is written into the SQL.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamMode {
    /// A number as it is, `null` as NULL, anything else as quoted text.
    #[default]
    Auto,
    /// Always quoted text.
    Text,
    /// As written: an expression, a column, a list.
    Sql,
}

/// The placeholders in `sql`, in the order they first appear. PostgreSQL's `?` is an
/// operator (jsonb), so there only `:name` and `$1` count.
pub fn parameters(sql: &str, dialect: Dialect) -> Vec<Param> {
    let mut out: Vec<Param> = vec![];
    let mut positional = 0;
    for t in tokenize(sql, dialect) {
        if t.kind != TokenKind::Parameter {
            continue;
        }
        let text = &sql[t.range.clone()];
        let name = if text == "?" {
            positional += 1;
            format!("?{positional}")
        } else {
            text.to_owned()
        };
        match out.iter_mut().find(|p| p.name == name) {
            Some(p) => p.ranges.push(t.range),
            None => out.push(Param {
                name,
                ranges: vec![t.range],
            }),
        }
    }
    out
}

/// A value as SQL.
pub fn value_sql(value: &str, mode: ParamMode, dialect: Dialect) -> String {
    match mode {
        ParamMode::Sql => value.to_owned(),
        ParamMode::Text => literal(Some(value), dialect),
        ParamMode::Auto => {
            let v = value.trim();
            if v.eq_ignore_ascii_case("null") {
                "NULL".into()
            } else if is_plain_number(v) {
                v.to_owned()
            } else {
                literal(Some(value), dialect)
            }
        }
    }
}

/// `-12`, `3.5`, `0`, but not `007` (a code, kept as text) or `1e5`.
fn is_plain_number(v: &str) -> bool {
    let digits = v.strip_prefix('-').unwrap_or(v);
    let (int, frac) = match digits.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (digits, None),
    };
    !int.is_empty()
        && int.bytes().all(|b| b.is_ascii_digit())
        && (int == "0" || !int.starts_with('0'))
        && frac.is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
}

/// `sql` with each placeholder replaced by its value (already SQL, from
/// [`value_sql`]). Placeholders without a value are left as they are.
pub fn bind(sql: &str, params: &[Param], values: &HashMap<String, String>) -> String {
    let mut spans: Vec<(&Range<usize>, &str)> = params
        .iter()
        .filter_map(|p| Some((p, values.get(&p.name)?)))
        .flat_map(|(p, v)| p.ranges.iter().map(move |r| (r, v.as_str())))
        .collect();
    spans.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    let mut out = sql.to_owned();
    for (r, v) in spans {
        out.replace_range(r.clone(), v);
    }
    out
}

/// The name a placeholder is given by on the command line: `id` for `:id`, `1` for
/// `?1` or `$1`.
pub fn short_name(name: &str) -> &str {
    name.trim_start_matches([':', '?', '$', '@'])
}

#[cfg(test)]
mod tests {
    use super::*;
    use Dialect::*;

    fn names(sql: &str, d: Dialect) -> Vec<String> {
        parameters(sql, d).into_iter().map(|p| p.name).collect()
    }

    #[test]
    fn finds_placeholders() {
        assert_eq!(
            names(
                "select :id, ':no', $1, x::text, data ? 'k' -- :not\n from t where a = :id",
                Postgres
            ),
            [":id", "$1"]
        );
        assert_eq!(
            names("select * from t where a = ? and b = ? and c = :c", Mysql),
            ["?1", "?2", ":c"]
        );
        assert_eq!(
            names("select ?, ?7, :a, @b, $c", Sqlite),
            ["?1", "?7", ":a", "@b", "$c"]
        );
        assert!(parameters("select @x := 1", Mysql).is_empty());
    }

    #[test]
    fn values_become_sql() {
        assert_eq!(value_sql("42", ParamMode::Auto, Postgres), "42");
        assert_eq!(value_sql("-1.5", ParamMode::Auto, Postgres), "-1.5");
        assert_eq!(value_sql("007", ParamMode::Auto, Postgres), "'007'");
        assert_eq!(value_sql("NULL", ParamMode::Auto, Postgres), "NULL");
        assert_eq!(
            value_sql("O'Brien", ParamMode::Auto, Postgres),
            "'O''Brien'"
        );
        assert_eq!(value_sql("42", ParamMode::Text, Postgres), "'42'");
        assert_eq!(value_sql("now()", ParamMode::Sql, Postgres), "now()");
    }

    #[test]
    fn binds_every_occurrence() {
        let sql = "select * from t where a = :id or b = :id and c = $1";
        let p = parameters(sql, Postgres);
        let values = HashMap::from([(":id".to_owned(), "5".to_owned())]);
        assert_eq!(
            bind(sql, &p, &values),
            "select * from t where a = 5 or b = 5 and c = $1"
        );
        assert_eq!(short_name(":id"), "id");
        assert_eq!(short_name("?2"), "2");
    }
}
