//! Autocompletion: what can be typed at the cursor, from the database's tables and
//! columns, the tables the statement already uses, and SQL keywords.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::db::CatalogTable;
use crate::sql::{self, Dialect, Token, TokenKind};

/// What autocompletion knows about a database.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    /// Tables here can be named without their schema.
    pub default_schema: String,
    pub schemas: Vec<String>,
    pub tables: Vec<CatalogTable>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Column,
    Table,
    View,
    Alias,
    Schema,
    Keyword,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completion {
    pub label: String,
    pub kind: ItemKind,
    /// Where it comes from: the table of a column, the schema of a table.
    pub detail: String,
    /// The text to put in, quoted if it needs to be.
    pub insert: String,
}

/// A table the statement uses, as `FROM schema.table alias`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRef {
    pub schema: Option<String>,
    pub table: String,
    pub alias: Option<String>,
}

/// Suggestions for the cursor at byte `cursor`: the byte range they replace, and
/// the suggestions, best first. Nothing inside strings and comments.
pub fn complete(
    text: &str,
    dialect: Dialect,
    cursor: usize,
    catalog: &Catalog,
) -> (Range<usize>, Vec<Completion>) {
    let cursor = cursor.min(text.len());
    let tokens = sql::tokenize(text, dialect);
    let inside = tokens
        .iter()
        .find(|t| t.range.start < cursor && cursor <= t.range.end);
    if let Some(t) = inside {
        let ended = match t.kind {
            // A finished quoted word or block comment: the cursor is after it.
            TokenKind::String | TokenKind::QuotedIdentifier => {
                cursor == t.range.end
                    && text[t.range.clone()].len() > 1
                    && closes(&text[t.range.clone()])
            }
            TokenKind::Comment => cursor == t.range.end && text[t.range.clone()].ends_with("*/"),
            _ => true,
        };
        if !ended {
            return (cursor..cursor, vec![]);
        }
    }

    // The word being typed, and what's before a dot in front of it.
    let start = text[..cursor]
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_' || *c == '$')
        .last()
        .map_or(cursor, |(i, _)| i);
    let prefix = &text[start..cursor];
    let qualifier = text[..start]
        .strip_suffix('.')
        .and_then(|before| last_identifier(before, dialect));

    let statement = sql::statement_at(text, dialect, cursor).unwrap_or(0..text.len());
    let statement_tokens: Vec<&Token> = tokens
        .iter()
        .filter(|t| t.range.start >= statement.start && t.range.end <= statement.end.max(cursor))
        .collect();
    let refs = table_refs(text, &statement_tokens);
    let wants = expects(text, &tokens, start);

    let mut out: Vec<Completion> = vec![];
    let find_table = |name: &str, schema: Option<&str>| {
        catalog
            .tables
            .iter()
            .find(|t| {
                t.name.eq_ignore_ascii_case(name)
                    && schema.map_or(t.schema == catalog.default_schema, |s| {
                        t.schema.eq_ignore_ascii_case(s)
                    })
            })
            .or_else(|| {
                catalog
                    .tables
                    .iter()
                    .find(|t| t.name.eq_ignore_ascii_case(name))
            })
    };
    let column_items = |t: &CatalogTable, out: &mut Vec<Completion>| {
        for c in &t.columns {
            out.push(Completion {
                label: c.clone(),
                kind: ItemKind::Column,
                detail: t.name.clone(),
                insert: quote_if_needed(c, dialect),
            });
        }
    };

    if let Some(q) = qualifier {
        // alias. / table. -> columns; schema. -> tables
        let by_ref = refs.iter().find(|r| {
            r.alias
                .as_deref()
                .is_some_and(|a| a.eq_ignore_ascii_case(&q))
                || r.table.eq_ignore_ascii_case(&q)
        });
        if let Some(r) = by_ref {
            if let Some(t) = find_table(&r.table, r.schema.as_deref()) {
                column_items(t, &mut out);
            }
        } else if let Some(t) = find_table(&q, None) {
            column_items(t, &mut out);
        }
        if let Some(schema) = catalog.schemas.iter().find(|s| s.eq_ignore_ascii_case(&q)) {
            for t in catalog.tables.iter().filter(|t| &t.schema == schema) {
                out.push(table_item(t, dialect, false));
            }
        }
    } else {
        let mut columns = vec![];
        for r in &refs {
            if let Some(t) = find_table(&r.table, r.schema.as_deref()) {
                column_items(t, &mut columns);
            }
        }
        let mut tables: Vec<Completion> = catalog
            .tables
            .iter()
            .map(|t| table_item(t, dialect, t.schema != catalog.default_schema))
            .collect();
        tables.sort_by_key(|t| t.insert.contains('.'));
        let aliases: Vec<Completion> = refs
            .iter()
            .filter_map(|r| r.alias.clone())
            .map(|a| Completion {
                insert: a.clone(),
                label: a,
                kind: ItemKind::Alias,
                detail: "alias".into(),
            })
            .collect();
        let schemas = catalog.schemas.iter().map(|s| Completion {
            label: s.clone(),
            kind: ItemKind::Schema,
            detail: "schema".into(),
            insert: quote_if_needed(s, dialect),
        });
        let keywords = sql::keywords().iter().map(|k| Completion {
            label: (*k).to_owned(),
            kind: ItemKind::Keyword,
            detail: String::new(),
            insert: (*k).to_owned(),
        });
        match wants {
            Wants::Table => {
                out.extend(tables);
                out.extend(schemas);
                out.extend(keywords);
            }
            Wants::Column => {
                out.extend(columns);
                out.extend(aliases);
                out.extend(keywords);
                out.extend(tables);
            }
            Wants::Anything => {
                out.extend(keywords);
                out.extend(columns);
                out.extend(aliases);
                out.extend(tables);
                out.extend(schemas);
            }
        }
    }

    // Keep what matches what's typed: starts with it first, then contains it.
    let lower = prefix.to_lowercase();
    let mut starts = vec![];
    let mut contains = vec![];
    let mut seen = std::collections::HashSet::new();
    for c in out {
        let l = c.label.to_lowercase();
        if !seen.insert((l.clone(), c.kind, c.detail.clone())) || l == lower {
            continue;
        }
        if l.starts_with(&lower) {
            starts.push(c);
        } else if lower.len() >= 2 && l.contains(&lower) && c.kind != ItemKind::Keyword {
            contains.push(c);
        }
    }
    starts.extend(contains);
    starts.truncate(100);
    (start..cursor, starts)
}

fn closes(quoted: &str) -> bool {
    let mut chars = quoted.chars();
    let (Some(open), Some(close)) = (chars.next(), quoted.chars().last()) else {
        return false;
    };
    let expected = if open == '[' { ']' } else { open };
    close == expected
}

fn table_item(t: &CatalogTable, dialect: Dialect, qualified: bool) -> Completion {
    let name = quote_if_needed(&t.name, dialect);
    Completion {
        label: t.name.clone(),
        kind: if t.view {
            ItemKind::View
        } else {
            ItemKind::Table
        },
        detail: t.schema.clone(),
        insert: if qualified {
            format!("{}.{name}", quote_if_needed(&t.schema, dialect))
        } else {
            name
        },
    }
}

/// The identifier just before `before`'s end (`schema` in `schema.`), unquoted.
fn last_identifier(before: &str, dialect: Dialect) -> Option<String> {
    let tokens = sql::tokenize(before, dialect);
    let t = tokens.last()?;
    let word = &before[t.range.clone()];
    match t.kind {
        TokenKind::Identifier | TokenKind::Keyword => Some(word.to_owned()),
        TokenKind::QuotedIdentifier if word.len() >= 2 => Some(word[1..word.len() - 1].to_owned()),
        _ => None,
    }
}

#[derive(Debug, PartialEq)]
enum Wants {
    Table,
    Column,
    Anything,
}

/// What usually comes next, from the significant token before `at`.
fn expects(text: &str, tokens: &[Token], at: usize) -> Wants {
    let prev = tokens
        .iter()
        .rev()
        .filter(|t| t.range.end <= at)
        .find(|t| !matches!(t.kind, TokenKind::Whitespace | TokenKind::Comment));
    let Some(prev) = prev else {
        return Wants::Anything;
    };
    let word = text[prev.range.clone()].to_ascii_uppercase();
    match word.as_str() {
        "FROM" | "JOIN" | "UPDATE" | "INTO" | "TABLE" | "DESCRIBE" | "DESC" => Wants::Table,
        "SELECT" | "WHERE" | "ON" | "BY" | "SET" | "AND" | "OR" | "HAVING" | "," | "(" | "="
        | "<" | ">" | "NOT" | "DISTINCT" | "CASE" | "WHEN" | "THEN" | "ELSE" | "+" | "-" | "*"
        | "/" => Wants::Column,
        _ => Wants::Anything,
    }
}

/// The tables a statement names after FROM, JOIN, UPDATE and INTO, with aliases.
pub fn table_refs(text: &str, tokens: &[&Token]) -> Vec<TableRef> {
    // Words that end a table reference rather than being its alias.
    const STOP: &[&str] = &[
        "WHERE",
        "ON",
        "USING",
        "JOIN",
        "INNER",
        "LEFT",
        "RIGHT",
        "FULL",
        "CROSS",
        "NATURAL",
        "OUTER",
        "GROUP",
        "ORDER",
        "LIMIT",
        "OFFSET",
        "HAVING",
        "SET",
        "VALUES",
        "UNION",
        "EXCEPT",
        "INTERSECT",
        "WINDOW",
        "RETURNING",
        "FOR",
        "LATERAL",
        "SELECT",
        "AS",
        "DEFAULT",
        "WITH",
        "FETCH",
        "STRAIGHT_JOIN",
    ];
    let words: Vec<(TokenKind, &str)> = tokens
        .iter()
        .filter(|t| !matches!(t.kind, TokenKind::Whitespace | TokenKind::Comment))
        .map(|t| (t.kind, &text[t.range.clone()]))
        .collect();
    let unquote = |k: TokenKind, w: &str| -> Option<String> {
        match k {
            TokenKind::Identifier | TokenKind::Keyword => Some(w.to_owned()),
            TokenKind::QuotedIdentifier if w.len() >= 2 => Some(w[1..w.len() - 1].to_owned()),
            _ => None,
        }
    };
    let is_stop = |w: &str| STOP.iter().any(|s| s.eq_ignore_ascii_case(w));
    let mut out = vec![];
    let mut i = 0;
    let mut in_from_list = false;
    while i < words.len() {
        let (_, w) = words[i];
        let starts = ["FROM", "JOIN", "UPDATE", "INTO"]
            .iter()
            .any(|k| k.eq_ignore_ascii_case(w));
        let continues = in_from_list && w == ",";
        if !(starts || continues) {
            if is_stop(w) && !w.eq_ignore_ascii_case("AS") {
                in_from_list = false;
            }
            i += 1;
            continue;
        }
        in_from_list = w.eq_ignore_ascii_case("FROM") || continues;
        i += 1;
        // [schema .] table
        let Some(&(k, first)) = words.get(i) else {
            break;
        };
        if first == "(" {
            continue; // a subquery
        }
        let Some(first) = unquote(k, first) else {
            continue;
        };
        let (schema, table) = if words.get(i + 1).is_some_and(|(_, w)| *w == ".") {
            match words.get(i + 2).and_then(|&(k, w)| unquote(k, w)) {
                Some(t) => {
                    i += 2;
                    (Some(first), t)
                }
                None => (None, first),
            }
        } else {
            (None, first)
        };
        i += 1;
        // [AS] alias
        if words
            .get(i)
            .is_some_and(|(_, w)| w.eq_ignore_ascii_case("AS"))
        {
            i += 1;
        }
        let alias = match words.get(i) {
            Some(&(k, w)) if !is_stop(w) && w != "," && w != ";" && w != "(" => {
                unquote(k, w).inspect(|_| i += 1)
            }
            _ => None,
        };
        out.push(TableRef {
            schema,
            table,
            alias,
        });
    }
    out
}

/// Words that would have to be quoted to be used as names.
const RESERVED: &[&str] = &[
    "ALL",
    "AND",
    "AS",
    "ASC",
    "BY",
    "CASE",
    "CHECK",
    "COLUMN",
    "CONSTRAINT",
    "CREATE",
    "DEFAULT",
    "DESC",
    "DISTINCT",
    "ELSE",
    "END",
    "FALSE",
    "FOR",
    "FOREIGN",
    "FROM",
    "GRANT",
    "GROUP",
    "HAVING",
    "IN",
    "INTO",
    "IS",
    "JOIN",
    "LIMIT",
    "NOT",
    "NULL",
    "ON",
    "OR",
    "ORDER",
    "PRIMARY",
    "REFERENCES",
    "SELECT",
    "TABLE",
    "THEN",
    "TO",
    "TRUE",
    "UNION",
    "UNIQUE",
    "USER",
    "USING",
    "WHEN",
    "WHERE",
    "WITH",
];

/// The name as it must be written: quoted if it isn't a plain lower-case (any case
/// outside PostgreSQL) word, or is reserved.
pub fn quote_if_needed(name: &str, dialect: Dialect) -> String {
    let mut chars = name.chars();
    let plain = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| {
            c.is_ascii_digit()
                || c == '_'
                || match dialect {
                    Dialect::Postgres => c.is_ascii_lowercase(),
                    _ => c.is_ascii_alphabetic() || c == '$',
                }
        });
    if plain && !RESERVED.iter().any(|r| r.eq_ignore_ascii_case(name)) {
        name.to_owned()
    } else {
        sql::quote_ident(name, dialect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        let t = |schema: &str, name: &str, cols: &[&str]| CatalogTable {
            schema: schema.into(),
            name: name.into(),
            view: false,
            columns: cols.iter().map(|c| (*c).to_owned()).collect(),
        };
        Catalog {
            default_schema: "public".into(),
            schemas: vec!["public".into(), "sales".into()],
            tables: vec![
                t("public", "customers", &["id", "name", "email"]),
                t(
                    "public",
                    "orders",
                    &["id", "customer_id", "total", "Created At"],
                ),
                t("sales", "invoices", &["id", "amount"]),
            ],
        }
    }

    /// Suggestions at the `|` in `text`.
    fn at(text: &str) -> Vec<String> {
        let cursor = text.find('|').unwrap();
        let text = text.replace('|', "");
        let (range, items) = complete(&text, Dialect::Postgres, cursor, &catalog());
        assert_eq!(range.end, cursor);
        items.into_iter().map(|c| c.insert).collect()
    }

    #[test]
    fn tables_after_from() {
        let got = at("select * from cu|");
        assert_eq!(got[0], "customers");
        let got = at("select * from |");
        assert_eq!(&got[..2], ["customers", "orders"]);
        assert!(got.contains(&"sales.invoices".to_owned()));
    }

    #[test]
    fn columns_of_the_tables_used() {
        let got = at("select to| from orders o join customers c on c.id = o.customer_id");
        assert_eq!(got[0], "total");
        let got = at("select * from orders where cust|");
        assert_eq!(
            got[0], "customer_id",
            "the column before the table of the same name"
        );
        // After an alias and a dot: that table's columns, quoted when they must be.
        let got = at("select o.| from orders o");
        assert_eq!(got, ["id", "customer_id", "total", "\"Created At\""]);
        let got = at("select c.e| from orders o, customers c");
        assert_eq!(got, ["email"]);
        // table. works without an alias, and schema. lists its tables.
        assert_eq!(at("select orders.tot|"), ["total"]);
        assert_eq!(at("select * from sales.|"), ["invoices"]);
    }

    #[test]
    fn keywords_and_contains() {
        assert!(at("sel|").contains(&"SELECT".to_owned()));
        // Matching inside a name comes after matching the start.
        let got = at("select * from orders where |");
        assert!(got.iter().any(|g| g == "customer_id"));
        assert_eq!(at("select * from mers|"), ["customers"]);
    }

    #[test]
    fn nothing_in_strings_or_comments() {
        assert!(at("select 'cu|' from orders").is_empty());
        assert!(at("select 1 -- cu|").is_empty());
        assert!(at("select /* cu| */ 1").is_empty());
    }

    #[test]
    fn finds_table_references() {
        let text = "update public.orders as o set total = 1; select * from a, b bb join \"Odd\" x on true where c";
        let tokens = sql::tokenize(text, Dialect::Postgres);
        let refs: Vec<&Token> = tokens.iter().collect();
        let found = table_refs(text, &refs);
        let short: Vec<(Option<&str>, &str, Option<&str>)> = found
            .iter()
            .map(|r| (r.schema.as_deref(), r.table.as_str(), r.alias.as_deref()))
            .collect();
        assert_eq!(
            short,
            [
                (Some("public"), "orders", Some("o")),
                (None, "a", None),
                (None, "b", Some("bb")),
                (None, "Odd", Some("x")),
            ]
        );
    }

    #[test]
    fn quoting() {
        assert_eq!(quote_if_needed("orders", Dialect::Postgres), "orders");
        assert_eq!(quote_if_needed("Orders", Dialect::Postgres), "\"Orders\"");
        assert_eq!(quote_if_needed("Orders", Dialect::Mysql), "Orders");
        assert_eq!(quote_if_needed("order", Dialect::Mysql), "`order`");
        assert_eq!(quote_if_needed("my table", Dialect::Sqlite), "\"my table\"");
    }
}
