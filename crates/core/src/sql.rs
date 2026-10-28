//! A small SQL lexer: enough to highlight SQL, split a script into statements and
//! find the statement under the cursor, for PostgreSQL, MySQL, SQLite and SQL Server.
//!
//! It never fails: anything unrecognised becomes an operator token, and an
//! unterminated string or comment runs to the end of the text.

use std::ops::Range;

use crate::profile::Driver;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Postgres,
    Mysql,
    Sqlite,
    /// T-SQL.
    SqlServer,
    ClickHouse,
}

impl From<Driver> for Dialect {
    fn from(d: Driver) -> Self {
        match d {
            Driver::Postgres => Self::Postgres,
            Driver::Mysql => Self::Mysql,
            Driver::Sqlite => Self::Sqlite,
            Driver::SqlServer => Self::SqlServer,
            Driver::ClickHouse => Self::ClickHouse,
            // Close enough to PostgreSQL's (quoting, $$ strings, ::casts, ALTER).
            Driver::DuckDb => Self::Postgres,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Whitespace,
    Comment,
    Keyword,
    Identifier,
    /// `"name"` (PostgreSQL) or `` `name` `` (MySQL).
    QuotedIdentifier,
    /// Including PostgreSQL's `E'...'` and `$tag$...$tag$`.
    String,
    Number,
    /// `$1` (PostgreSQL) or `?` (MySQL).
    Parameter,
    /// `; , ( ) [ ] .`
    Punctuation,
    Operator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    /// Byte range in the text.
    pub range: Range<usize>,
}

pub fn tokenize(text: &str, dialect: Dialect) -> Vec<Token> {
    let mut lexer = Lexer {
        s: text,
        b: text.as_bytes(),
        i: 0,
        dialect,
        out: Vec::new(),
    };
    lexer.run();
    lexer.out
}

struct Lexer<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
    dialect: Dialect,
    out: Vec<Token>,
}

impl Lexer<'_> {
    fn peek(&self, ahead: usize) -> Option<u8> {
        self.b.get(self.i + ahead).copied()
    }

    fn push(&mut self, kind: TokenKind, start: usize) {
        self.out.push(Token {
            kind,
            range: start..self.i,
        });
    }

    /// The char starting at the current position (the text is valid UTF-8, and `i`
    /// only ever lands on char boundaries).
    fn char_here(&self) -> char {
        self.s[self.i..].chars().next().expect("not at the end")
    }

    fn run(&mut self) {
        while self.i < self.b.len() {
            let start = self.i;
            let c = self.b[self.i];
            match c {
                _ if self.char_here().is_whitespace() => {
                    while self.i < self.b.len() && self.char_here().is_whitespace() {
                        self.i += self.char_here().len_utf8();
                    }
                    self.push(TokenKind::Whitespace, start);
                }
                b'-' if self.peek(1) == Some(b'-') && self.line_comment_dash_ok() => {
                    self.skip_to_line_end();
                    self.push(TokenKind::Comment, start);
                }
                b'#' if self.dialect == Dialect::Mysql => {
                    self.skip_to_line_end();
                    self.push(TokenKind::Comment, start);
                }
                b'/' if self.peek(1) == Some(b'*') => {
                    self.block_comment();
                    self.push(TokenKind::Comment, start);
                }
                b'\'' => {
                    let backslash = matches!(self.dialect, Dialect::Mysql | Dialect::ClickHouse);
                    self.quoted(b'\'', backslash);
                    self.push(TokenKind::String, start);
                }
                b'"' => {
                    // MySQL treats "..." as a string (unless ANSI_QUOTES is on).
                    let mysql = self.dialect == Dialect::Mysql;
                    self.quoted(b'"', mysql);
                    let kind = if mysql {
                        TokenKind::String
                    } else {
                        TokenKind::QuotedIdentifier
                    };
                    self.push(kind, start);
                }
                b'`' if self.dialect != Dialect::Postgres => {
                    self.quoted(b'`', false);
                    self.push(TokenKind::QuotedIdentifier, start);
                }
                b'[' if matches!(self.dialect, Dialect::Sqlite | Dialect::SqlServer) => {
                    // [name]; SQL Server doubles a ] inside.
                    self.quoted(b']', false);
                    self.push(TokenKind::QuotedIdentifier, start);
                }
                b'N' | b'n'
                    if self.dialect == Dialect::SqlServer
                        && self.peek(1) == Some(b'\'')
                        && !self.prev_is_ident_char() =>
                {
                    // N'unicode text'.
                    self.i += 1;
                    self.quoted(b'\'', false);
                    self.push(TokenKind::String, start);
                }
                // @variable, @@rowcount, #temp, ##global: names, not placeholders.
                b'@' | b'#'
                    if self.dialect == Dialect::SqlServer
                        && self.s[self.i..]
                            .trim_start_matches(['@', '#'])
                            .chars()
                            .next()
                            .is_some_and(is_ident_start) =>
                {
                    while self.i < self.b.len() && matches!(self.b[self.i], b'@' | b'#') {
                        self.i += 1;
                    }
                    while self.i < self.b.len() && is_ident_char(self.char_here()) {
                        self.i += self.char_here().len_utf8();
                    }
                    self.push(TokenKind::Identifier, start);
                }
                b'?' | b':' | b'@' | b'$'
                    if self.dialect == Dialect::Sqlite
                        && (c == b'?'
                            || self.s[self.i + 1..]
                                .chars()
                                .next()
                                .is_some_and(is_ident_start)) =>
                {
                    // ?, ?NNN, :name, @name, $name
                    self.i += 1;
                    while self.i < self.b.len() && is_ident_char(self.char_here()) {
                        self.i += self.char_here().len_utf8();
                    }
                    self.push(TokenKind::Parameter, start);
                }
                b'e' | b'E'
                    if self.dialect == Dialect::Postgres
                        && self.peek(1) == Some(b'\'')
                        && !self.prev_is_ident_char() =>
                {
                    self.i += 1;
                    self.quoted(b'\'', true);
                    self.push(TokenKind::String, start);
                }
                b'$' if self.dialect == Dialect::Postgres => {
                    if self.peek(1).is_some_and(|d| d.is_ascii_digit()) {
                        self.i += 1;
                        while self.peek(0).is_some_and(|d| d.is_ascii_digit()) {
                            self.i += 1;
                        }
                        self.push(TokenKind::Parameter, start);
                    } else if let Some(tag_len) = self.dollar_tag() {
                        self.dollar_string(tag_len);
                        self.push(TokenKind::String, start);
                    } else {
                        self.i += 1;
                        self.push(TokenKind::Operator, start);
                    }
                }
                b'?' if self.dialect == Dialect::Mysql => {
                    self.i += 1;
                    self.push(TokenKind::Parameter, start);
                }
                // :name, for running with values asked for (not PostgreSQL's ::type,
                // nor ClickHouse's {name:Type}).
                b':' if self.s[self.i + 1..]
                    .chars()
                    .next()
                    .is_some_and(is_ident_start)
                    && (self.i == 0 || self.b[self.i - 1] != b':')
                    && !self.prev_is_ident_char() =>
                {
                    self.i += 1;
                    while self.i < self.b.len() && is_ident_char(self.char_here()) {
                        self.i += self.char_here().len_utf8();
                    }
                    self.push(TokenKind::Parameter, start);
                }
                b'0'..=b'9' => {
                    self.number();
                    self.push(TokenKind::Number, start);
                }
                b'.' if self.peek(1).is_some_and(|d| d.is_ascii_digit()) => {
                    self.number();
                    self.push(TokenKind::Number, start);
                }
                b';' | b',' | b'(' | b')' | b'[' | b']' | b'.' => {
                    self.i += 1;
                    self.push(TokenKind::Punctuation, start);
                }
                _ if is_ident_start(self.char_here()) => {
                    while self.i < self.b.len() && is_ident_char(self.char_here()) {
                        self.i += self.char_here().len_utf8();
                    }
                    let kind = if is_keyword(&self.s[start..self.i]) {
                        TokenKind::Keyword
                    } else {
                        TokenKind::Identifier
                    };
                    self.push(kind, start);
                }
                _ => {
                    self.i += self.char_here().len_utf8();
                    self.push(TokenKind::Operator, start);
                }
            }
        }
    }

    /// MySQL needs whitespace (or the end) after `--` for it to be a comment.
    fn line_comment_dash_ok(&self) -> bool {
        match self.dialect {
            Dialect::Postgres | Dialect::Sqlite | Dialect::SqlServer | Dialect::ClickHouse => true,
            Dialect::Mysql => match self.s.get(self.i + 2..) {
                Some(rest) => rest.chars().next().is_none_or(|c| c.is_whitespace()),
                None => true,
            },
        }
    }

    fn prev_is_ident_char(&self) -> bool {
        self.s[..self.i]
            .chars()
            .next_back()
            .is_some_and(is_ident_char)
    }

    fn skip_to_line_end(&mut self) {
        while self.i < self.b.len() && self.b[self.i] != b'\n' {
            self.i += 1;
        }
        // Stay on a char boundary (a comment can end in a multi-byte char).
        while !self.s.is_char_boundary(self.i) {
            self.i += 1;
        }
    }

    fn block_comment(&mut self) {
        // PostgreSQL nests /* */; MySQL doesn't.
        let nests = self.dialect == Dialect::Postgres;
        self.i += 2;
        let mut depth = 1;
        while self.i < self.b.len() {
            if self.b[self.i] == b'*' && self.peek(1) == Some(b'/') {
                self.i += 2;
                depth -= 1;
                if depth == 0 {
                    return;
                }
            } else if nests && self.b[self.i] == b'/' && self.peek(1) == Some(b'*') {
                self.i += 2;
                depth += 1;
            } else {
                self.i += 1;
            }
        }
        self.fix_boundary();
    }

    /// A quoted run ending at an unescaped `q`; a doubled `q` is an escaped one.
    fn quoted(&mut self, q: u8, backslash_escapes: bool) {
        self.i += 1;
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if backslash_escapes && c == b'\\' {
                self.i += 2;
                continue;
            }
            if c == q {
                if self.peek(1) == Some(q) {
                    self.i += 2;
                    continue;
                }
                self.i += 1;
                return;
            }
            self.i += 1;
        }
        self.fix_boundary();
    }

    fn fix_boundary(&mut self) {
        self.i = self.i.min(self.b.len());
        while !self.s.is_char_boundary(self.i) {
            self.i += 1;
        }
    }

    /// The length of a `$tag$` opener here (including both `$`), if there is one.
    fn dollar_tag(&self) -> Option<usize> {
        if self.prev_is_ident_char() {
            return None;
        }
        let rest = &self.s[self.i + 1..];
        for (n, c) in rest.char_indices() {
            if c == '$' {
                return Some(n + 2);
            }
            let ok = if n == 0 {
                c.is_alphabetic() || c == '_'
            } else {
                c.is_alphanumeric() || c == '_'
            };
            if !ok {
                return None;
            }
        }
        None
    }

    fn dollar_string(&mut self, tag_len: usize) {
        let tag = &self.s[self.i..self.i + tag_len];
        let body = self.i + tag_len;
        match self.s[body..].find(tag) {
            Some(end) => self.i = body + end + tag_len,
            None => self.i = self.b.len(),
        }
    }

    fn number(&mut self) {
        let digits = |l: &mut Self| {
            while l.peek(0).is_some_and(|d| d.is_ascii_digit() || d == b'_') {
                l.i += 1;
            }
        };
        if self.b[self.i] == b'0' && matches!(self.peek(1), Some(b'x' | b'X')) {
            self.i += 2;
            while self.peek(0).is_some_and(|d| d.is_ascii_hexdigit()) {
                self.i += 1;
            }
            return;
        }
        digits(self);
        if self.peek(0) == Some(b'.') && self.peek(1) != Some(b'.') {
            self.i += 1;
            digits(self);
        }
        if matches!(self.peek(0), Some(b'e' | b'E')) {
            let sign = matches!(self.peek(1), Some(b'+' | b'-')) as usize;
            if self.peek(1 + sign).is_some_and(|d| d.is_ascii_digit()) {
                self.i += 1 + sign;
                digits(self);
            }
        }
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Reserved and common keywords of PostgreSQL and MySQL, upper case and sorted.
const KEYWORDS: &[&str] = &[
    "ADD",
    "AFTER",
    "ALL",
    "ALTER",
    "ANALYZE",
    "AND",
    "ANY",
    "ARRAY",
    "AS",
    "ASC",
    "AUTO_INCREMENT",
    "BEFORE",
    "BEGIN",
    "BETWEEN",
    "BIGINT",
    "BIGSERIAL",
    "BINARY",
    "BLOB",
    "BOOLEAN",
    "BOTH",
    "BY",
    "CALL",
    "CASCADE",
    "CASE",
    "CAST",
    "CHAR",
    "CHARACTER",
    "CHECK",
    "COLLATE",
    "COLUMN",
    "COMMENT",
    "COMMIT",
    "CONCURRENTLY",
    "CONFLICT",
    "CONSTRAINT",
    "COPY",
    "CREATE",
    "CROSS",
    "CURRENT_DATE",
    "CURRENT_TIME",
    "CURRENT_TIMESTAMP",
    "CURRENT_USER",
    "DATABASE",
    "DATE",
    "DATETIME",
    "DECIMAL",
    "DECLARE",
    "DEFAULT",
    "DEFERRABLE",
    "DELETE",
    "DESC",
    "DESCRIBE",
    "DISTINCT",
    "DO",
    "DOUBLE",
    "DROP",
    "EACH",
    "ELSE",
    "END",
    "ENGINE",
    "ENUM",
    "ESCAPE",
    "EXCEPT",
    "EXEC",
    "EXECUTE",
    "EXISTS",
    "EXPLAIN",
    "EXTENSION",
    "FALSE",
    "FETCH",
    "FILTER",
    "FIRST",
    "FLOAT",
    "FOR",
    "FOREIGN",
    "FROM",
    "FULL",
    "FUNCTION",
    "GO",
    "GRANT",
    "GROUP",
    "HAVING",
    "IDENTITY",
    "IF",
    "ILIKE",
    "IN",
    "INDEX",
    "INNER",
    "INSERT",
    "INT",
    "INTEGER",
    "INTERSECT",
    "INTERVAL",
    "INTO",
    "IS",
    "JOIN",
    "JSON",
    "JSONB",
    "KEY",
    "LANGUAGE",
    "LAST",
    "LATERAL",
    "LEADING",
    "LEFT",
    "LIKE",
    "LIMIT",
    "LOCK",
    "LONGTEXT",
    "MATERIALIZED",
    "MERGE",
    "NATURAL",
    "NOLOCK",
    "NOT",
    "NOTHING",
    "NULL",
    "NULLS",
    "NUMERIC",
    "NVARCHAR",
    "OFFSET",
    "ON",
    "ONLY",
    "OR",
    "ORDER",
    "OUTER",
    "OUTPUT",
    "OVER",
    "PARTITION",
    "PIVOT",
    "PRIMARY",
    "PRINT",
    "PROCEDURE",
    "RECURSIVE",
    "REFERENCES",
    "REGEXP",
    "RELEASE",
    "RENAME",
    "REPLACE",
    "RESTRICT",
    "RETURN",
    "RETURNING",
    "RETURNS",
    "REVOKE",
    "RIGHT",
    "ROLLBACK",
    "ROW",
    "ROWS",
    "SAVEPOINT",
    "SCHEMA",
    "SELECT",
    "SEQUENCE",
    "SERIAL",
    "SESSION",
    "SET",
    "SHOW",
    "SMALLINT",
    "SOME",
    "START",
    "TABLE",
    "TEMP",
    "TEMPORARY",
    "TEXT",
    "THEN",
    "TIME",
    "TIMESTAMP",
    "TIMESTAMPTZ",
    "TINYINT",
    "TO",
    "TOP",
    "TRAILING",
    "TRAN",
    "TRANSACTION",
    "TRIGGER",
    "TRUE",
    "TRUNCATE",
    "TYPE",
    "UNION",
    "UNIQUE",
    "UNPIVOT",
    "UNSIGNED",
    "UPDATE",
    "USE",
    "USING",
    "UUID",
    "VACUUM",
    "VALUES",
    "VARCHAR",
    "VIEW",
    "WHEN",
    "WHERE",
    "WINDOW",
    "WITH",
    "WITHOUT",
];

/// The keywords the lexer knows, upper case.
pub fn keywords() -> &'static [&'static str] {
    KEYWORDS
}

pub fn is_keyword(word: &str) -> bool {
    word.len() <= 20
        && KEYWORDS
            .binary_search(&word.to_ascii_uppercase().as_str())
            .is_ok()
}

/// The statements in a script: byte ranges without the closing `;` or surrounding
/// whitespace. Pieces holding only comments are left out.
pub fn split_statements(text: &str, dialect: Dialect) -> Vec<Range<usize>> {
    if dialect == Dialect::Mysql && text.to_ascii_uppercase().contains("DELIMITER") {
        return split_with_delimiters(text);
    }
    if dialect == Dialect::SqlServer {
        // Statements within each GO batch.
        return go_batches(text)
            .into_iter()
            .flat_map(|b| {
                split_semicolons(&text[b.clone()], dialect)
                    .into_iter()
                    .map(move |r| r.start + b.start..r.end + b.start)
            })
            .collect();
    }
    split_semicolons(text, dialect)
}

/// What runs as one request: a statement each, except on SQL Server, where it's a
/// batch (up to a `GO` line), so variables declared in it last through it.
pub fn batches(text: &str, dialect: Dialect) -> Vec<Range<usize>> {
    if dialect != Dialect::SqlServer {
        return split_statements(text, dialect);
    }
    go_batches(text)
        .into_iter()
        .filter_map(|b| {
            // Trimmed to its statements; nothing but comments isn't a batch.
            let stmts = split_semicolons(&text[b.clone()], dialect);
            let first = stmts.first()?.start + b.start;
            let last = stmts.last()?.end + b.start;
            Some(first..last)
        })
        .collect()
}

/// The text between `GO` lines (SQL Server's batch separator, a line of its own,
/// maybe with a count: `GO 5`).
fn go_batches(text: &str) -> Vec<Range<usize>> {
    let tokens = tokenize(text, Dialect::SqlServer);
    let mut out = vec![];
    let mut start = 0;
    for (i, t) in tokens.iter().enumerate() {
        if !text[t.range.clone()].eq_ignore_ascii_case("GO") {
            continue;
        }
        let line_start = text[..t.range.start].rfind('\n').map_or(0, |n| n + 1);
        if !text[line_start..t.range.start].trim().is_empty() {
            continue;
        }
        // Only a count or a comment may follow on its line.
        let mut end = t.range.end;
        let mut alone = true;
        for u in &tokens[i + 1..] {
            let s = &text[u.range.clone()];
            match u.kind {
                TokenKind::Whitespace if s.contains('\n') => {
                    end = u.range.start + s.find('\n').expect("checked") + 1;
                    break;
                }
                TokenKind::Whitespace | TokenKind::Number | TokenKind::Comment => {
                    end = u.range.end;
                }
                _ => {
                    alone = false;
                    break;
                }
            }
        }
        if alone {
            out.push(start..line_start);
            start = end;
        }
    }
    out.push(start..text.len());
    out
}

fn split_semicolons(text: &str, dialect: Dialect) -> Vec<Range<usize>> {
    let mut out = vec![];
    let mut first: Option<usize> = None;
    let mut last_end = 0;
    let mut has_code = false;
    // SQLite's CREATE TRIGGER has a BEGIN ... END body with `;`s inside (and CASE
    // ... END expressions in it).
    let mut keywords = 0;
    let mut trigger = false;
    let mut depth = 0usize;
    for t in tokenize(text, dialect) {
        if dialect == Dialect::Sqlite && t.kind == TokenKind::Keyword {
            let word = text[t.range.clone()].to_ascii_uppercase();
            keywords += 1;
            match word.as_str() {
                "TRIGGER" if keywords <= 3 && has_code => trigger = true,
                "BEGIN" | "CASE" if trigger => depth += 1,
                "END" if trigger => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        match t.kind {
            TokenKind::Punctuation if &text[t.range.clone()] == ";" && depth == 0 => {
                if has_code && let Some(f) = first {
                    out.push(f..last_end);
                }
                first = None;
                has_code = false;
                keywords = 0;
                trigger = false;
            }
            TokenKind::Whitespace => {}
            kind => {
                first.get_or_insert(t.range.start);
                last_end = t.range.end;
                has_code |= kind != TokenKind::Comment;
            }
        }
    }
    if has_code && let Some(f) = first {
        out.push(f..last_end);
    }
    out
}

/// MySQL scripts that change the statement delimiter, as the `mysql` client allows:
///
/// ```sql
/// DELIMITER $$
/// CREATE PROCEDURE p() BEGIN SELECT 1; SELECT 2; END $$
/// DELIMITER ;
/// ```
///
/// `DELIMITER` lines are for the client and aren't part of any statement.
fn split_with_delimiters(text: &str) -> Vec<Range<usize>> {
    let tokens = tokenize(text, Dialect::Mysql);
    let mut out = vec![];
    let mut delimiter = ";".to_owned();
    let mut first: Option<usize> = None;
    let mut last_end = 0;
    let mut has_code = false;
    let mut skip_until = 0;
    let mut finish = |first: &mut Option<usize>, has_code: &mut bool, last_end: usize| {
        if *has_code && let Some(f) = *first {
            out.push(f..last_end);
        }
        *first = None;
        *has_code = false;
    };
    for t in &tokens {
        if t.range.start < skip_until {
            continue;
        }
        let word = &text[t.range.clone()];
        let at_line_start = text[..t.range.start]
            .rsplit('\n')
            .next()
            .is_some_and(|before| before.trim().is_empty());
        match t.kind {
            TokenKind::Whitespace => {}
            TokenKind::Identifier
                if first.is_none() && at_line_start && word.eq_ignore_ascii_case("DELIMITER") =>
            {
                let line_end = text[t.range.end..]
                    .find('\n')
                    .map_or(text.len(), |n| t.range.end + n);
                if let Some(d) = text[t.range.end..line_end].split_whitespace().next() {
                    delimiter = d.to_owned();
                }
                skip_until = line_end;
            }
            TokenKind::String | TokenKind::QuotedIdentifier | TokenKind::Comment => {
                first.get_or_insert(t.range.start);
                last_end = t.range.end;
                has_code |= t.kind != TokenKind::Comment;
            }
            _ if text[t.range.start..].starts_with(delimiter.as_str()) => {
                finish(&mut first, &mut has_code, last_end);
                skip_until = t.range.start + delimiter.len();
            }
            _ => {
                first.get_or_insert(t.range.start);
                last_end = t.range.end;
                has_code = true;
            }
        }
    }
    finish(&mut first, &mut has_code, last_end);
    out
}

/// The statement to run for a cursor at byte `pos`: the one it's in (or whose `;` it
/// is just after); otherwise the closest one above it, or else the first below.
pub fn statement_at(text: &str, dialect: Dialect, pos: usize) -> Option<Range<usize>> {
    let stmts = split_statements(text, dialect);
    let inside = stmts.iter().find(|r| r.start <= pos && pos <= r.end);
    let after_semicolon = || {
        stmts
            .iter()
            .find(|r| pos == r.end + 1 && text[r.end..].starts_with(';'))
    };
    let above = || stmts.iter().rev().find(|r| r.end <= pos);
    inside
        .or_else(after_semicolon)
        .or_else(above)
        .or(stmts.first())
        .cloned()
}

/// The statement's first keyword, upper case (`SELECT`, `WITH`, ...), skipping
/// comments and opening brackets.
pub fn first_keyword(sql: &str, dialect: Dialect) -> Option<String> {
    tokenize(sql, dialect)
        .into_iter()
        .find(|t| {
            !matches!(t.kind, TokenKind::Whitespace | TokenKind::Comment)
                && &sql[t.range.clone()] != "("
        })
        .filter(|t| matches!(t.kind, TokenKind::Keyword | TokenKind::Identifier))
        .map(|t| sql[t.range].to_ascii_uppercase())
}

/// The statement that begins a transaction.
pub fn begin(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Mysql => "START TRANSACTION",
        Dialect::SqlServer | Dialect::ClickHouse => "BEGIN TRANSACTION",
        Dialect::Postgres | Dialect::Sqlite => "BEGIN",
    }
}

/// Whether a transaction is open after `statement` ran (it was open before if
/// `open`): BEGIN and START TRANSACTION open one; COMMIT, END and ROLLBACK (but not
/// ROLLBACK TO a savepoint) close it, as do MySQL's statements that commit
/// implicitly (DDL and the like).
pub fn transaction_after(open: bool, statement: &str, dialect: Dialect) -> bool {
    let words: Vec<String> = tokenize(statement, dialect)
        .into_iter()
        .filter(|t| matches!(t.kind, TokenKind::Keyword | TokenKind::Identifier))
        .take(2)
        .map(|t| statement[t.range].to_ascii_uppercase())
        .collect();
    let first = words.first().map(String::as_str);
    let second = words.get(1).map(String::as_str);
    match first {
        Some("BEGIN") if dialect == Dialect::SqlServer => {
            open || matches!(second, Some("TRAN" | "TRANSACTION"))
        }
        Some("BEGIN") => true,
        Some("START") => second == Some("TRANSACTION") || open,
        Some("COMMIT" | "END") => false,
        Some("ROLLBACK") => open && second == Some("TO"),
        Some("CREATE" | "ALTER" | "DROP" | "RENAME" | "TRUNCATE" | "GRANT" | "REVOKE")
            if dialect == Dialect::Mysql =>
        {
            false
        }
        _ => open,
    }
}

/// Whether a statement's rows can be filtered and sorted by wrapping it in
/// `SELECT * FROM (...)`.
pub fn is_query(sql: &str, dialect: Dialect) -> bool {
    matches!(
        first_keyword(sql, dialect).as_deref(),
        Some("SELECT" | "WITH" | "VALUES" | "TABLE")
    )
}

/// `base`'s rows, filtered by a `WHERE` condition and sorted by a column (descending
/// if `desc`). On their own lines, so a comment at the end of `base` stays one.
pub fn filtered(
    base: &str,
    condition: &str,
    order: Option<(&str, bool)>,
    dialect: Dialect,
) -> String {
    let mut s = format!("SELECT * FROM (\n{}\n) AS dd_rows", base.trim());
    let condition = condition.trim().trim_end_matches(';').trim();
    if !condition.is_empty() {
        s.push_str(&format!("\nWHERE {condition}"));
    }
    if let Some((col, desc)) = order {
        s.push_str(&format!(
            "\nORDER BY {}{}",
            quote_ident(col, dialect),
            if desc { " DESC" } else { "" }
        ));
    }
    s
}

/// Quote an identifier for the dialect: `"a""b"`, `` `a``b` `` or `[a]]b]`.
pub fn quote_ident(name: &str, dialect: Dialect) -> String {
    match dialect {
        Dialect::Postgres | Dialect::Sqlite => format!("\"{}\"", name.replace('"', "\"\"")),
        Dialect::Mysql | Dialect::ClickHouse => format!("`{}`", name.replace('`', "``")),
        Dialect::SqlServer => format!("[{}]", name.replace(']', "]]")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Dialect::*;

    fn kinds(text: &str, d: Dialect) -> Vec<(TokenKind, &str)> {
        tokenize(text, d)
            .into_iter()
            .filter(|t| t.kind != TokenKind::Whitespace)
            .map(|t| (t.kind, &text[t.range]))
            .collect()
    }

    fn stmts(text: &str, d: Dialect) -> Vec<&str> {
        split_statements(text, d)
            .into_iter()
            .map(|r| &text[r])
            .collect()
    }

    #[test]
    fn keywords_sorted() {
        let mut sorted = KEYWORDS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, KEYWORDS);
    }

    #[test]
    fn tokens_basic() {
        use TokenKind::*;
        assert_eq!(
            kinds("select a.b, 'it''s', 1.5e3 from t -- hi", Postgres),
            [
                (Keyword, "select"),
                (Identifier, "a"),
                (Punctuation, "."),
                (Identifier, "b"),
                (Punctuation, ","),
                (String, "'it''s'"),
                (Punctuation, ","),
                (Number, "1.5e3"),
                (Keyword, "from"),
                (Identifier, "t"),
                (Comment, "-- hi"),
            ]
        );
    }

    #[test]
    fn postgres_specials() {
        use TokenKind::*;
        let sql =
            r#"select "Odd ""name""", $1, E'a\'b', $fn$ select ';' $fn$, x::int /* a /* b */ c */"#;
        assert_eq!(
            kinds(sql, Postgres),
            [
                (Keyword, "select"),
                (QuotedIdentifier, r#""Odd ""name""""#),
                (Punctuation, ","),
                (Parameter, "$1"),
                (Punctuation, ","),
                (String, r"E'a\'b'"),
                (Punctuation, ","),
                (String, "$fn$ select ';' $fn$"),
                (Punctuation, ","),
                (Identifier, "x"),
                (Operator, ":"),
                (Operator, ":"),
                (Keyword, "int"),
                (Comment, "/* a /* b */ c */"),
            ]
        );
    }

    #[test]
    fn mysql_specials() {
        use TokenKind::*;
        let sql = "select `a``b`, \"str\", 'x\\'y', ? # note\n-- c\n--x";
        assert_eq!(
            kinds(sql, Mysql),
            [
                (Keyword, "select"),
                (QuotedIdentifier, "`a``b`"),
                (Punctuation, ","),
                (String, "\"str\""),
                (Punctuation, ","),
                (String, "'x\\'y'"),
                (Punctuation, ","),
                (Parameter, "?"),
                (Comment, "# note"),
                (Comment, "-- c"),
                // No space after --: MySQL reads it as minus minus.
                (Operator, "-"),
                (Operator, "-"),
                (Identifier, "x"),
            ]
        );
    }

    #[test]
    fn unterminated_and_unicode() {
        // Never panics, and never splits a multi-byte char.
        for text in [
            "'café",
            "/* ünïcode",
            "\"ab",
            "$a$ never",
            "-- é",
            "é;ü",
            "$tag",
        ] {
            for d in [Postgres, Mysql] {
                let toks = tokenize(text, d);
                assert_eq!(toks.last().map(|t| t.range.end), Some(text.len()));
                for t in toks {
                    assert!(
                        text.is_char_boundary(t.range.start) && text.is_char_boundary(t.range.end)
                    );
                }
            }
        }
    }

    #[test]
    fn sql_server_batches() {
        use TokenKind::*;
        let script = "declare @x int = 1;\nselect @x as [a]]b], N'it''s' -- go\nGO\n\
                      /* GO */ select 2; select '\nGO\n'\ngo 3\n-- just a comment\nGO\n";
        let b: Vec<&str> = batches(script, SqlServer)
            .into_iter()
            .map(|r| &script[r])
            .collect();
        assert_eq!(
            b,
            [
                "declare @x int = 1;\nselect @x as [a]]b], N'it''s' -- go",
                "/* GO */ select 2; select '\nGO\n'",
            ]
        );
        assert_eq!(split_statements(script, SqlServer).len(), 4);
        let k = kinds("select @x, @@rowcount, #t, [n], N'x', ?", SqlServer);
        assert!(k.contains(&(Identifier, "@x")));
        assert!(k.contains(&(Identifier, "#t")));
        assert!(k.contains(&(QuotedIdentifier, "[n]")));
        assert!(k.contains(&(String, "N'x'")));
        assert_eq!(quote_ident("a]b", SqlServer), "[a]]b]");
    }

    #[test]
    fn tracks_transactions() {
        assert!(transaction_after(false, "BEGIN", Postgres));
        assert!(transaction_after(false, "start transaction", Mysql));
        assert!(transaction_after(true, "update t set a = 1", Postgres));
        assert!(!transaction_after(true, "commit", Postgres));
        assert!(!transaction_after(true, "END", Sqlite));
        assert!(!transaction_after(true, "rollback", Mysql));
        assert!(transaction_after(true, "rollback to savepoint a", Postgres));
        assert!(!transaction_after(true, "create table x (a int)", Mysql));
        assert!(transaction_after(true, "create table x (a int)", Postgres));
        assert!(!transaction_after(false, "select 1", Sqlite));
    }

    #[test]
    fn filtering_wraps_the_query() {
        assert!(is_query("with x as (select 1) select * from x", Postgres));
        assert!(!is_query("update t set a = 1", Postgres));
        assert_eq!(
            filtered(
                "select * from t -- all",
                " a > 1; ",
                Some(("Name", true)),
                Postgres
            ),
            "SELECT * FROM (\nselect * from t -- all\n) AS dd_rows\nWHERE a > 1\nORDER BY \"Name\" DESC"
        );
        assert_eq!(
            filtered("select 1", "", None, Mysql),
            "SELECT * FROM (\nselect 1\n) AS dd_rows"
        );
    }

    #[test]
    fn splitting() {
        let script = "select 1;\n\n-- comment only\n;  select ';' ; \ninsert into t values (1);\n-- trailing";
        assert_eq!(
            stmts(script, Postgres),
            ["select 1", "select ';'", "insert into t values (1)"]
        );
        let body = "create function f() returns int as $$ begin return 1; end $$ language plpgsql; select f()";
        assert_eq!(stmts(body, Postgres).len(), 2);
        let trigger = "create temp trigger t after insert on a begin \
                       update a set n = case when new.n > 0 then 1 else 0 end; \
                       delete from b; end; select 1";
        assert_eq!(
            stmts(trigger, Sqlite),
            [
                "create temp trigger t after insert on a begin \
                 update a set n = case when new.n > 0 then 1 else 0 end; \
                 delete from b; end",
                "select 1"
            ]
        );
        // BEGIN on its own is a transaction.
        assert_eq!(stmts("begin; select 1; end", Sqlite).len(), 3);
        assert_eq!(stmts("", Postgres), Vec::<&str>::new());
        assert_eq!(
            stmts("-- just this\nselect 2", Mysql),
            ["-- just this\nselect 2"]
        );
    }

    #[test]
    fn sqlite_specials() {
        use TokenKind::*;
        let sql = "select [odd name], `b`, \"c\", ?1, :n, @m, $v from t";
        let k: Vec<_> = kinds(sql, Sqlite)
            .into_iter()
            .filter(|(k, _)| *k != Punctuation)
            .collect();
        assert_eq!(
            k,
            [
                (Keyword, "select"),
                (QuotedIdentifier, "[odd name]"),
                (QuotedIdentifier, "`b`"),
                (QuotedIdentifier, "\"c\""),
                (Parameter, "?1"),
                (Parameter, ":n"),
                (Parameter, "@m"),
                (Parameter, "$v"),
                (Keyword, "from"),
                (Identifier, "t"),
            ]
        );
        // PostgreSQL's array brackets stay punctuation there.
        assert_eq!(kinds("a[1]", Postgres)[1], (Punctuation, "["));
    }

    #[test]
    fn mysql_delimiters() {
        let script = "DELIMITER $$\n\
            CREATE PROCEDURE p()\nBEGIN\n  SELECT 1;\n  SELECT ';$$';\nEND $$\n\
            DELIMITER ;\n\
            CALL p();\n\
            delimiter //\nselect 2 //\n";
        assert_eq!(
            stmts(script, Mysql),
            [
                "CREATE PROCEDURE p()\nBEGIN\n  SELECT 1;\n  SELECT ';$$';\nEND",
                "CALL p()",
                "select 2",
            ]
        );
        // "delimiter" as a column name isn't a command.
        assert_eq!(stmts("select delimiter from t; select 1", Mysql).len(), 2);
    }

    #[test]
    fn statement_under_cursor() {
        let text = "select 1;\nselect 2;\n\n\nselect 3";
        let at = |pos| statement_at(text, Postgres, pos).map(|r| &text[r]);
        assert_eq!(at(0), Some("select 1"));
        assert_eq!(at(9), Some("select 1")); // right after the first ;
        assert_eq!(at(12), Some("select 2"));
        assert_eq!(at(21), Some("select 2")); // on a blank line after it
        assert_eq!(at(text.len()), Some("select 3"));
        assert_eq!(statement_at("  ", Postgres, 1), None);
        assert_eq!(
            statement_at("\n\nselect 9;", Postgres, 0).map(|r| r.start),
            Some(2)
        );
    }

    #[test]
    fn first_keywords_and_quoting() {
        assert_eq!(
            first_keyword("  -- x\n(SELECT 1)", Postgres).as_deref(),
            Some("SELECT")
        );
        assert_eq!(
            first_keyword("with x as (select 1) select * from x", Mysql).as_deref(),
            Some("WITH")
        );
        assert_eq!(first_keyword("'str'", Postgres), None);
        assert_eq!(quote_ident("we\"ird", Postgres), "\"we\"\"ird\"");
        assert_eq!(quote_ident("we`ird", Mysql), "`we``ird`");
    }
}
