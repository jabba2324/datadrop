//! Formatting SQL: clauses on their own lines, select lists and conditions one item
//! per line, subqueries indented, keywords in capitals. Only whitespace and the case
//! of keywords change, so the SQL means the same; strings, quoted names, comments
//! and function bodies are kept as written.

use crate::sql::{Dialect, Token, TokenKind, split_statements, tokenize};

const INDENT: &str = "    ";

/// Keywords written in capitals. Words that are often names too (`name`, `user`,
/// `type`...) are left as typed, since MySQL table names can be case-sensitive.
const UPPER: &[&str] = &[
    "ADD",
    "ALL",
    "ALTER",
    "AND",
    "ANY",
    "AS",
    "ASC",
    "BEGIN",
    "BETWEEN",
    "BY",
    "CASCADE",
    "CASE",
    "CAST",
    "CHECK",
    "COLUMN",
    "COMMIT",
    "CONFLICT",
    "CONSTRAINT",
    "CREATE",
    "CROSS",
    "DEFAULT",
    "DELETE",
    "DESC",
    "DISTINCT",
    "DO",
    "DROP",
    "ELSE",
    "END",
    "EXCEPT",
    "EXISTS",
    "FALSE",
    "FETCH",
    "FIRST",
    "FOREIGN",
    "FROM",
    "FULL",
    "GROUP",
    "HAVING",
    "IF",
    "ILIKE",
    "IN",
    "INDEX",
    "INNER",
    "INSERT",
    "INTERSECT",
    "INTO",
    "IS",
    "JOIN",
    "KEY",
    "LATERAL",
    "LEFT",
    "LIKE",
    "LIMIT",
    "NATURAL",
    "NEXT",
    "NOT",
    "NOTHING",
    "NULL",
    "OFFSET",
    "ON",
    "ONLY",
    "OR",
    "ORDER",
    "OUTER",
    "OVER",
    "PARTITION",
    "PRIMARY",
    "RECURSIVE",
    "REFERENCES",
    "RETURNING",
    "RIGHT",
    "ROLLBACK",
    "ROWS",
    "SELECT",
    "SET",
    "SOME",
    "TABLE",
    "THEN",
    "TRUE",
    "TRUNCATE",
    "UNION",
    "UNIQUE",
    "UPDATE",
    "USING",
    "VALUES",
    "VIEW",
    "WHEN",
    "WHERE",
    "WINDOW",
    "WITH",
];

/// Words that start a clause on a new line.
const CLAUSES: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "GROUP",
    "HAVING",
    "ORDER",
    "LIMIT",
    "OFFSET",
    "FETCH",
    "UNION",
    "INTERSECT",
    "EXCEPT",
    "WINDOW",
    "RETURNING",
    "VALUES",
    "SET",
    "JOIN",
    "LEFT",
    "RIGHT",
    "INNER",
    "FULL",
    "CROSS",
    "NATURAL",
    "ON",
];

/// Format a script, statement by statement.
pub fn format_sql(sql: &str, dialect: Dialect) -> String {
    // A script that changes the delimiter would need its DELIMITER lines kept where
    // they are; it's left as it is.
    if dialect == Dialect::Mysql && sql.to_ascii_uppercase().contains("DELIMITER") {
        return sql.to_owned();
    }
    let mut s = String::new();
    for range in split_statements(sql, dialect) {
        if !s.is_empty() {
            s.push('\n');
        }
        let stmt = &sql[range];
        s.push_str(&format_statement(stmt, dialect));
        // After a `--` comment, the `;` would be part of it.
        let ends_in_comment = tokenize(stmt, dialect)
            .iter()
            .rev()
            .find(|t| t.kind != TokenKind::Whitespace)
            .is_some_and(|t| {
                t.kind == TokenKind::Comment && !stmt[t.range.clone()].starts_with("/*")
            });
        if ends_in_comment {
            s.push('\n');
        }
        s.push_str(";\n");
    }
    s
}

#[derive(Clone, Copy, PartialEq)]
enum Paren {
    /// `f(a, b)`, `IN (1, 2)`: on one line.
    Inline,
    /// A subquery, or a `CREATE TABLE` column list: indented, an item a line.
    Block,
}

/// What a block paren saves of the clause it's in.
#[derive(Clone, Copy)]
struct Outer {
    depth: usize,
    list: bool,
    list_long: bool,
    condition: bool,
}

struct Writer<'a> {
    src: &'a str,
    out: String,
    /// Indent of the current clause.
    depth: usize,
    /// Open parens, with the clause state outside them to go back to.
    parens: Vec<(Paren, Outer)>,
    /// What the last word was (in capitals), to know what a token means.
    last_word: String,
    last: Option<Token>,
    /// In a list (SELECT a, b; SET a = 1, b = 2)...
    list: bool,
    /// ...laid out an item a line.
    list_long: bool,
    /// In a WHERE/HAVING/ON condition: AND and OR start lines.
    condition: bool,
    between: bool,
    /// The statement is a CREATE TABLE (its column list is a block).
    create_table: bool,
    /// The next token follows the last one directly (after `::`, or a unary minus).
    glue: bool,
    /// The keyword before the last name (`INTO` in `INSERT INTO t`).
    ident_after: String,
}

impl Writer<'_> {
    fn text(&self, t: &Token) -> &str {
        &self.src[t.range.clone()]
    }

    fn newline(&mut self, extra: usize) {
        while self.out.ends_with(' ') {
            self.out.pop();
        }
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
        for _ in 0..self.depth + extra {
            self.out.push_str(INDENT);
        }
    }

    fn at_line_start(&self) -> bool {
        self.out.is_empty() || self.out.ends_with('\n') || self.out.ends_with(INDENT)
    }

    fn space(&mut self) {
        if !self.at_line_start() && !self.out.ends_with(' ') {
            self.out.push(' ');
        }
    }

    /// Whether the paren level is the clause's own (not inside a function call).
    fn top(&self) -> bool {
        !matches!(self.parens.last(), Some((Paren::Inline, _)))
    }
}

fn format_statement(stmt: &str, dialect: Dialect) -> String {
    let tokens: Vec<Token> = tokenize(stmt, dialect)
        .into_iter()
        .filter(|t| t.kind != TokenKind::Whitespace)
        .collect();
    let mut w = Writer {
        src: stmt,
        out: String::new(),
        depth: 0,
        parens: vec![],
        last_word: String::new(),
        last: None,
        list: false,
        list_long: false,
        condition: false,
        between: false,
        create_table: false,
        glue: false,
        ident_after: String::new(),
    };
    for (n, t) in tokens.iter().enumerate() {
        let text = w.text(t).to_owned();
        let upper = text.to_ascii_uppercase();
        let next = tokens.get(n + 1);
        let next_text = next.map(|t| w.text(t).to_ascii_uppercase());
        match t.kind {
            TokenKind::Comment => {
                w.space();
                w.out.push_str(text.trim_end());
                if text.starts_with("--") || text.starts_with('#') {
                    w.newline(usize::from(w.list));
                }
            }
            TokenKind::Keyword => {
                let word = if UPPER.contains(&upper.as_str()) {
                    upper.clone()
                } else {
                    text.clone()
                };
                // LEFT and RIGHT are also functions: left(name, 3).
                let call =
                    next_text.as_deref() == Some("(") && matches!(upper.as_str(), "LEFT" | "RIGHT");
                let joins_prev = matches!(
                    (w.last_word.as_str(), upper.as_str()),
                    (
                        "LEFT" | "RIGHT" | "FULL" | "INNER" | "CROSS" | "NATURAL" | "OUTER",
                        "JOIN"
                    ) | ("LEFT" | "RIGHT" | "FULL", "OUTER")
                        | ("NATURAL", "LEFT" | "RIGHT" | "FULL" | "INNER")
                        | ("UNION" | "INTERSECT" | "EXCEPT", "ALL" | "DISTINCT")
                        | ("DELETE", "FROM")
                        | ("DO", "UPDATE" | "NOTHING")
                        | ("ON", "UPDATE" | "DELETE" | "CONFLICT")
                );
                let first = w.out.is_empty();
                let clause = CLAUSES.contains(&upper.as_str())
                    && !call
                    && !joins_prev
                    && !first
                    && w.top()
                    // ON in CREATE INDEX ... ON t, ON DELETE CASCADE.
                    && !(upper == "ON"
                        && next_text.as_deref() != Some("CONFLICT")
                        && (w.create_table || !w.condition_context()))
                    // SET in ON CONFLICT DO UPDATE SET / ALTER ... SET stays a clause.
                    && !(upper == "SET" && w.last_word == "ALTER");
                if upper == "TABLE" && w.last_word == "CREATE"
                    || upper == "TABLE" && matches!(w.last_word.as_str(), "TEMP" | "TEMPORARY")
                {
                    w.create_table = true;
                }
                if clause {
                    w.list = false;
                    w.list_long = false;
                    w.condition = false;
                    if upper == "ON" && next_text.as_deref() != Some("CONFLICT") {
                        w.newline(1);
                    } else {
                        w.newline(0);
                    }
                } else if (upper == "AND" || upper == "OR")
                    && w.condition
                    && w.top()
                    && !(upper == "AND" && w.between)
                {
                    w.newline(1);
                } else if !w.glue && !matches!(w.last.as_ref().map(|l| w.text(l)), Some("(" | "."))
                {
                    w.space();
                }
                w.out.push_str(&word);
                if upper == "AND" && w.between {
                    w.between = false;
                }
                match upper.as_str() {
                    "SELECT" => w.list = true,
                    "SET" if !w.create_table => w.list = true,
                    "WHERE" | "HAVING" if clause || first => w.condition = true,
                    "ON" if clause && next_text.as_deref() != Some("CONFLICT") => {
                        w.condition = true
                    }
                    "BETWEEN" => w.between = true,
                    _ => {}
                }
                if w.list && (upper == "SELECT" || upper == "SET") {
                    // The first item on the next line, unless it's a short list.
                    let items_follow = next.is_some_and(|t| {
                        !matches!(
                            w.text(t).to_ascii_uppercase().as_str(),
                            "DISTINCT" | "ALL" | "TOP"
                        )
                    });
                    w.list_long = items_follow && list_is_long(&tokens[n + 1..], stmt);
                    if w.list_long {
                        w.newline(1);
                    }
                } else if upper == "DISTINCT" && w.list && list_is_long(&tokens[n + 1..], stmt) {
                    w.list_long = true;
                    w.newline(1);
                }
                w.last_word = upper;
            }
            TokenKind::Punctuation => match text.as_str() {
                "(" => {
                    let subquery = matches!(next_text.as_deref(), Some("SELECT" | "WITH"));
                    let column_list = w.create_table && w.parens.is_empty();
                    let after_name = (matches!(
                        w.last.as_ref().map(|l| l.kind),
                        Some(TokenKind::Identifier | TokenKind::QuotedIdentifier)
                    ) || (w.last.as_ref().map(|l| l.kind)
                        == Some(TokenKind::Keyword)
                        && !matches!(
                            w.last_word.as_str(),
                            "IN" | "VALUES"
                                | "AS"
                                | "ON"
                                | "AND"
                                | "OR"
                                | "NOT"
                                | "EXISTS"
                                | "ANY"
                                | "ALL"
                                | "SOME"
                                | "FROM"
                                | "JOIN"
                                | "USING"
                                | "INTO"
                                | "TABLE"
                                | "OVER"
                                | "WHERE"
                                | "SELECT"
                                | "THEN"
                                | "ELSE"
                                | "WHEN"
                                | "KEY"
                                | "UNIQUE"
                                | "CHECK"
                                | "REFERENCES"
                                | "RETURNING"
                                | "BY"
                                | "SET"
                                | "IS"
                                | "LIKE"
                                | "WITH"
                                | "UNION"
                                | "EXCEPT"
                                | "INTERSECT"
                                | "LATERAL"
                                | "FILTER"
                                | "WITHIN"
                                | "GROUP"
                                | "CONFLICT"
                        )))
                        && !matches!(w.ident_after.as_str(), "INTO" | "TABLE" | "REFERENCES");
                    if !after_name && !matches!(w.last.as_ref().map(|l| w.text(l)), Some("(" | "."))
                    {
                        w.space();
                    }
                    w.out.push('(');
                    let outer = Outer {
                        depth: w.depth,
                        list: w.list,
                        list_long: w.list_long,
                        condition: w.condition,
                    };
                    if subquery || column_list {
                        w.parens.push((Paren::Block, outer));
                        // Saved with the paren: the outer clause's state.
                        w.depth += usize::from(w.list) + 1;
                        w.list = column_list;
                        w.list_long = column_list;
                        w.condition = false;
                        // Its first line (a subquery's SELECT) is indented under it.
                        w.newline(0);
                    } else {
                        w.parens.push((Paren::Inline, outer));
                    }
                    w.last_word.clear();
                }
                ")" => {
                    match w.parens.pop() {
                        Some((Paren::Block, outer)) => {
                            w.depth = outer.depth;
                            // Under the item it belongs to, in a list.
                            w.newline(usize::from(outer.list));
                            // Back in the outer clause.
                            w.list = outer.list;
                            w.list_long = outer.list_long;
                            w.condition = outer.condition;
                            w.out.push(')');
                        }
                        _ => {
                            while w.out.ends_with(' ') {
                                w.out.pop();
                            }
                            w.out.push(')');
                        }
                    }
                    w.last_word.clear();
                }
                "," => {
                    while w.out.ends_with(' ') {
                        w.out.pop();
                    }
                    w.out.push(',');
                    if w.list && w.list_long && w.top() {
                        // A CREATE TABLE column list is already indented.
                        let in_table =
                            w.create_table && matches!(w.parens.last(), Some((Paren::Block, _)));
                        w.newline(usize::from(!in_table));
                    }
                }
                "." => {
                    while w.out.ends_with(' ') {
                        w.out.pop();
                    }
                    w.out.push('.');
                }
                ";" => w.out.push(';'),
                other => {
                    w.space();
                    w.out.push_str(other);
                }
            },
            TokenKind::Operator => {
                let prev = w.last.as_ref().map(|l| (l.kind, w.text(l).to_owned()));
                let tight = matches!(text.as_str(), "::" | "." | ":");
                let unary = matches!(text.as_str(), "-" | "+")
                    && matches!(
                        prev.as_ref().map(|(k, s)| (*k, s.as_str())),
                        None | Some((TokenKind::Operator, _))
                            | Some((TokenKind::Keyword, _))
                            | Some((TokenKind::Punctuation, "(" | ","))
                    );
                let star = text == "*"
                    && matches!(prev.as_ref().map(|(_, s)| s.as_str()), Some("(" | "."));
                if w.glue {
                } else if !tight && !star {
                    w.space();
                } else {
                    while w.out.ends_with(' ') {
                        w.out.pop();
                    }
                }
                w.out.push_str(&text);
                w.last = Some(t.clone());
                w.glue = tight || unary;
                continue;
            }
            _ => {
                let glued = w.glue || matches!(w.last.as_ref().map(|l| w.text(l)), Some("(" | "."));
                if !glued {
                    w.space();
                }
                w.out.push_str(&text);
                if !w.last_word.is_empty() {
                    w.ident_after = std::mem::take(&mut w.last_word);
                }
            }
        }
        w.glue = false;
        w.last = Some(t.clone());
    }
    w.out.trim().to_owned()
}

impl Writer<'_> {
    /// ON starts a line when it's a join condition.
    fn condition_context(&self) -> bool {
        // After a JOIN's table: the last clause word on the line was a join.
        let line = self.out.rsplit('\n').next().unwrap_or("");
        line.split_whitespace()
            .any(|w| w.eq_ignore_ascii_case("JOIN"))
    }
}

/// Whether a list (until the next clause at this level) is long enough to put an
/// item on each line: more than one item and over 60 characters.
fn list_is_long(rest: &[Token], src: &str) -> bool {
    let mut depth = 0i32;
    let mut commas = 0;
    let mut len = 0;
    for t in rest {
        let text = &src[t.range.clone()];
        match text {
            "(" => depth += 1,
            ")" => {
                depth -= 1;
                if depth < 0 {
                    break;
                }
            }
            "," if depth == 0 => commas += 1,
            ";" => break,
            _ => {}
        }
        if depth == 0
            && t.kind == TokenKind::Keyword
            && CLAUSES.contains(&text.to_ascii_uppercase().as_str())
            && !text.eq_ignore_ascii_case("ON")
        {
            break;
        }
        len += text.len() + 1;
    }
    commas > 0 && len > 60
}

#[cfg(test)]
mod tests {
    use super::*;
    use Dialect::*;

    /// The tokens of `sql`, keywords in capitals, without whitespace.
    fn meaning(sql: &str, d: Dialect) -> Vec<String> {
        tokenize(sql, d)
            .into_iter()
            .filter(|t| t.kind != TokenKind::Whitespace)
            .map(|t| {
                let s = &sql[t.range];
                if t.kind == TokenKind::Keyword {
                    s.to_ascii_uppercase()
                } else {
                    s.trim_end().to_owned()
                }
            })
            .filter(|s| s != ";")
            .collect()
    }

    fn check(sql: &str, d: Dialect) -> String {
        let f = format_sql(sql, d);
        assert_eq!(meaning(&f, d), meaning(sql, d), "changed the SQL:\n{f}");
        assert_eq!(format_sql(&f, d), f, "not stable:\n{f}");
        f
    }

    #[test]
    fn clauses_and_lists() {
        let f = check(
            "select o.id, o.total, c.name as customer, count(*) filter (where x > 1) as n \
             from orders o left join customers c on c.id = o.customer_id and c.active \
             where o.total > 100 and (o.status = 'open' or o.status = 'held') \
             and o.placed between now() - interval '1 day' and now() \
             group by o.id, c.name order by o.total desc limit 10",
            Postgres,
        );
        assert_eq!(
            f,
            "SELECT\n    o.id,\n    o.total,\n    c.name AS customer,\n    \
             count(*) filter (WHERE x > 1) AS n\n\
             FROM orders o\n\
             LEFT JOIN customers c\n    ON c.id = o.customer_id\n    AND c.active\n\
             WHERE o.total > 100\n    AND (o.status = 'open' OR o.status = 'held')\n    \
             AND o.placed BETWEEN now() - interval '1 day' AND now()\n\
             GROUP BY o.id, c.name\n\
             ORDER BY o.total DESC\n\
             LIMIT 10;\n"
        );
    }

    #[test]
    fn short_selects_stay_on_one_line() {
        assert_eq!(
            check("select a, b from t where id = :id", Postgres),
            "SELECT a, b\nFROM t\nWHERE id = :id;\n"
        );
        assert_eq!(
            check("select 1;select 2", Mysql),
            "SELECT 1;\n\nSELECT 2;\n"
        );
        assert_eq!(
            check("select -1, x::text, t.*", Postgres),
            "SELECT -1, x::text, t.*;\n"
        );
    }

    #[test]
    fn subqueries_are_indented() {
        let f = check(
            "select * from (select id, sum(total) from orders group by id) s where s.id in (select id from vip)",
            Postgres,
        );
        assert_eq!(
            f,
            "SELECT *\nFROM (\n    SELECT id, sum(total)\n    FROM orders\n    GROUP BY id\n) s\n\
             WHERE s.id IN (\n    SELECT id\n    FROM vip\n);\n"
        );
    }

    #[test]
    fn writes_and_ddl() {
        let f = check(
            "insert into t (a, b) values (1, 'x'), (2, 'y') returning a",
            Postgres,
        );
        assert_eq!(
            f,
            "INSERT INTO t (a, b)\nVALUES (1, 'x'), (2, 'y')\nRETURNING a;\n"
        );
        let f = check(
            "update t set a = 1, b = 'something rather longer than that', c = now() where id = 3",
            Postgres,
        );
        assert_eq!(
            f,
            "UPDATE t\nSET\n    a = 1,\n    b = 'something rather longer than that',\n    c = now()\n\
             WHERE id = 3;\n"
        );
        let f = check(
            "create table t (id int primary key, name text not null default 'x', \
             owner int references owners (id) on delete cascade)",
            Postgres,
        );
        assert_eq!(
            f,
            "CREATE TABLE t (\n    id int PRIMARY KEY,\n    name text NOT NULL DEFAULT 'x',\n    \
             owner int REFERENCES owners (id) ON DELETE CASCADE\n);\n"
        );
    }

    #[test]
    fn keeps_the_meaning_of_harder_sql() {
        for (sql, d) in [
            (
                "select case when a > 0 then 'pos' when a < 0 then 'neg' else 'zero' end as sign, \
                 row_number() over (partition by g order by a desc) as rn, \
                 coalesce(nullif(b, ''), 'none') from t where not exists (select 1 from u \
                 where u.id = t.id) and a in (1, 2, (select max(x) from v))",
                Postgres,
            ),
            (
                "select a from t group by a having count(*) > 1 and sum(b) between 1 and 9 \
                 order by 1 offset 5 rows fetch next 10 rows only",
                Postgres,
            ),
            (
                "insert into t (a) select a from u on conflict (a) do update set a = excluded.a",
                Postgres,
            ),
            (
                "create index concurrently i on t using gin (data jsonb_path_ops) where a is not null",
                Postgres,
            ),
            (
                "select data->>'k', data #> '{a,b}', arr[1:2] from t",
                Postgres,
            ),
            (
                "create trigger tr after insert on t begin update t set n = 1; delete from u; end",
                Sqlite,
            ),
            (
                "select * from a natural join b cross join c left outer join d using (id) \
                 right join e on e.x = d.x and e.y = 1",
                Mysql,
            ),
            ("select left(name, 3), right(name, 2) from t", Mysql),
            ("update t set a = 1 where id = 1 limit 1", Mysql),
            (
                "alter table t add column c int not null default 0, drop column d",
                Postgres,
            ),
            ("select 1 -- trailing", Postgres),
        ] {
            check(sql, d);
        }
        let delimited = "DELIMITER $$\ncreate procedure p() begin select 1; end $$\nDELIMITER ;";
        assert_eq!(format_sql(delimited, Mysql), delimited);
    }

    #[test]
    fn keeps_comments_strings_and_bodies() {
        check(
            "-- the report\nselect a -- first\n, 'select  from' as s /* note */ from t",
            Postgres,
        );
        check(
            "create function f() returns int as $$ select   1 $$ language sql",
            Postgres,
        );
        check("select `order`, \"x\" from `my table` # end", Mysql);
        check("delete from t where id in (1, 2, 3)", Sqlite);
        check(
            "with x as (select 1 as a) select a from x union all select 2",
            Postgres,
        );
    }
}
