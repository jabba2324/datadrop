//! A schema's structure: tables and views with their columns, constraints, indexes
//! and triggers, plus routines and sequences. The schema browser, the DDL view, the
//! table editor, schema diffs and diagrams all work from this.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::db::{MetaPool, TableKind};
use crate::edit::literal;
use crate::error::{CoreError, Result};
use crate::profile::Driver;
use crate::sql::{Dialect, quote_ident};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaModel {
    pub schema: String,
    pub tables: Vec<TableModel>,
    pub routines: Vec<Routine>,
    pub sequences: Vec<Sequence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableModel {
    pub name: String,
    pub kind: TableKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub columns: Vec<ColumnModel>,
    #[serde(default)]
    pub constraints: Vec<Constraint>,
    #[serde(default)]
    pub indexes: Vec<Index>,
    #[serde(default)]
    pub triggers: Vec<Trigger>,
    /// A view's `CREATE VIEW` (and, on SQLite, a table's own `CREATE TABLE`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnModel {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    /// As SQL (`0`, `'new'`, `now()`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// The clause that numbers it: `GENERATED ALWAYS AS IDENTITY`, `AUTO_INCREMENT`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    /// The clause that computes it: `GENERATED ALWAYS AS (a + b) STORED`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    PrimaryKey,
    Unique,
    Check,
    ForeignKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constraint {
    pub name: String,
    pub kind: ConstraintKind,
    pub columns: Vec<String>,
    /// What follows `CONSTRAINT name`: `PRIMARY KEY (id)`, `CHECK (price > 0)`...
    pub definition: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub references: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    pub schema: String,
    pub table: String,
    pub columns: Vec<String>,
    pub on_update: String,
    pub on_delete: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    pub name: String,
    /// Column names, or expressions.
    pub columns: Vec<String>,
    pub unique: bool,
    /// The whole `CREATE INDEX` statement.
    pub definition: String,
    /// MySQL: the index as part of `CREATE TABLE` (`UNIQUE KEY name (cols)`), which
    /// is how it's created there, so a foreign key reuses it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trigger {
    pub name: String,
    /// The whole `CREATE TRIGGER` statement.
    pub definition: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineKind {
    Function,
    Procedure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routine {
    pub name: String,
    pub kind: RoutineKind,
    /// The argument list, as in `name(arguments)`.
    pub arguments: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returns: Option<String>,
    /// The whole `CREATE FUNCTION`/`PROCEDURE` statement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sequence {
    pub name: String,
    pub definition: String,
}

/// What a DDL request is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    /// A table or view (with its indexes and triggers).
    Table,
    Routine,
    Sequence,
    /// Everything in the schema.
    Schema,
}

impl TableModel {
    pub fn primary_key(&self) -> Option<&Constraint> {
        self.constraints
            .iter()
            .find(|c| c.kind == ConstraintKind::PrimaryKey)
    }

    pub fn foreign_keys(&self) -> impl Iterator<Item = &Constraint> {
        self.constraints
            .iter()
            .filter(|c| c.kind == ConstraintKind::ForeignKey)
    }

    pub fn column(&self, name: &str) -> Option<&ColumnModel> {
        self.columns.iter().find(|c| c.name == name)
    }

    pub fn is_view(&self) -> bool {
        matches!(self.kind, TableKind::View | TableKind::MaterializedView)
    }
}

impl SchemaModel {
    pub fn table(&self, name: &str) -> Option<&TableModel> {
        self.tables.iter().find(|t| t.name == name)
    }
}

/// `"schema"."name"`, or just `"name"` for SQLite's main database.
pub fn qualified(schema: &str, name: &str, dialect: Dialect) -> String {
    if dialect == Dialect::Sqlite && schema == "main" {
        quote_ident(name, dialect)
    } else {
        format!(
            "{}.{}",
            quote_ident(schema, dialect),
            quote_ident(name, dialect)
        )
    }
}

pub fn column_list(cols: &[String], dialect: Dialect) -> String {
    cols.iter()
        .map(|c| quote_ident(c, dialect))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A column as it's written in `CREATE TABLE` / `ADD COLUMN`.
pub fn column_sql(c: &ColumnModel, dialect: Dialect) -> String {
    let mut s = quote_ident(&c.name, dialect);
    if !c.data_type.is_empty() {
        s.push(' ');
        s.push_str(&c.data_type);
    }
    if let Some(g) = &c.generated {
        s.push(' ');
        s.push_str(g);
    }
    if !c.nullable {
        s.push_str(" NOT NULL");
    }
    if let Some(d) = &c.default
        && c.generated.is_none()
    {
        s.push_str(" DEFAULT ");
        s.push_str(d);
    }
    if let Some(i) = &c.identity {
        s.push(' ');
        s.push_str(i);
    }
    if dialect == Dialect::Mysql
        && let Some(comment) = &c.comment
    {
        s.push_str(" COMMENT ");
        s.push_str(&literal(Some(comment), dialect));
    }
    s
}

/// `CREATE TABLE` for a table, then its indexes, comments and triggers; or a view's
/// definition. Statements end with `;`.
pub fn table_ddl(schema: &str, t: &TableModel, dialect: Dialect) -> String {
    let mut out = String::new();
    if t.is_view() {
        if let Some(d) = &t.definition {
            out.push_str(&statement(d.trim_end().trim_end_matches(';'), dialect));
        }
    } else if dialect == Dialect::Sqlite
        && let Some(d) = &t.definition
    {
        out.push_str(d.trim_end().trim_end_matches(';'));
        out.push_str(";\n");
    } else {
        out.push_str(&create_table(schema, t, dialect));
    }
    for i in t
        .indexes
        .iter()
        .filter(|i| i.inline.is_none() || t.is_view())
    {
        out.push_str(&i.definition);
        out.push_str(";\n");
    }
    if dialect == Dialect::Postgres {
        let name = qualified(schema, &t.name, dialect);
        if let Some(c) = &t.comment {
            out.push_str(&format!(
                "COMMENT ON {} {name} IS {};\n",
                if t.is_view() { "VIEW" } else { "TABLE" },
                literal(Some(c), dialect)
            ));
        }
        for col in &t.columns {
            if let Some(c) = &col.comment {
                out.push_str(&format!(
                    "COMMENT ON COLUMN {name}.{} IS {};\n",
                    quote_ident(&col.name, dialect),
                    literal(Some(c), dialect)
                ));
            }
        }
    }
    if dialect == Dialect::SqlServer {
        let describe = |text: &str, column: Option<&str>| {
            let mut s = format!(
                "EXEC sp_addextendedproperty N'MS_Description', {}, N'SCHEMA', {}, N'{}', {}",
                literal(Some(text), dialect),
                literal(Some(schema), dialect),
                if t.is_view() { "VIEW" } else { "TABLE" },
                literal(Some(&t.name), dialect)
            );
            if let Some(c) = column {
                s.push_str(&format!(", N'COLUMN', {}", literal(Some(c), dialect)));
            }
            s + ";\n"
        };
        if let Some(c) = &t.comment {
            out.push_str(&describe(c, None));
        }
        for col in &t.columns {
            if let Some(c) = &col.comment {
                out.push_str(&describe(c, Some(&col.name)));
            }
        }
    }
    for tr in &t.triggers {
        out.push_str(&statement(&tr.definition, dialect));
    }
    out
}

/// A statement with its terminator: MySQL bodies with `;` inside need a delimiter.
fn statement(sql: &str, dialect: Dialect) -> String {
    let sql = sql.trim_end();
    // CREATE VIEW / PROCEDURE / FUNCTION / TRIGGER must be alone in its batch.
    if dialect == Dialect::SqlServer {
        return format!("GO\n{sql}\nGO\n");
    }
    if dialect == Dialect::Mysql && sql.contains(';') {
        format!("DELIMITER $$\n{sql}$$\nDELIMITER ;\n")
    } else {
        format!("{};\n", sql.trim_end_matches(';'))
    }
}

pub fn create_table(schema: &str, t: &TableModel, dialect: Dialect) -> String {
    let mut lines: Vec<String> = t
        .columns
        .iter()
        .map(|c| format!("    {}", column_sql(c, dialect)))
        .collect();
    for c in &t.constraints {
        // MySQL's primary key is always called PRIMARY.
        if dialect == Dialect::Mysql && c.kind == ConstraintKind::PrimaryKey {
            lines.push(format!("    {}", c.definition));
            continue;
        }
        lines.push(format!(
            "    CONSTRAINT {} {}",
            quote_ident(&c.name, dialect),
            c.definition
        ));
    }
    for i in &t.indexes {
        if let Some(inline) = &i.inline {
            lines.push(format!("    {inline}"));
        }
    }
    let mut s = format!(
        "CREATE TABLE {} (\n{}\n)",
        qualified(schema, &t.name, dialect),
        lines.join(",\n")
    );
    if dialect == Dialect::Mysql
        && let Some(c) = &t.comment
    {
        s.push_str(&format!(" COMMENT={}", literal(Some(c), dialect)));
    }
    s.push_str(";\n");
    s
}

pub fn routine_ddl(r: &Routine, dialect: Dialect) -> String {
    r.definition
        .as_deref()
        .map(|d| statement(d, dialect))
        .unwrap_or_default()
}

/// Every object in the schema, in an order that runs: sequences and routines (with
/// function bodies not checked yet, as pg_dump does), tables (referenced ones first;
/// foreign keys between tables that refer to each other are added after), views, and
/// then triggers.
pub fn schema_ddl(m: &SchemaModel, dialect: Dialect) -> String {
    let mut out = String::new();
    if dialect == Dialect::Postgres && !m.routines.is_empty() {
        out.push_str("SET check_function_bodies = false;\n\n");
    }
    for s in &m.sequences {
        out.push_str(&s.definition);
        out.push_str(";\n");
    }
    if !m.sequences.is_empty() {
        out.push('\n');
    }
    for r in &m.routines {
        out.push_str(&routine_ddl(r, dialect));
        out.push('\n');
    }
    let (order, deferred) = table_order(m);
    let mut triggers = vec![];
    for t in &order {
        let mut t = (*t).clone();
        if dialect != Dialect::Sqlite {
            t.constraints
                .retain(|c| !deferred.contains(&(t.name.clone(), c.name.clone())));
        }
        triggers.append(&mut t.triggers);
        out.push_str(&table_ddl(&m.schema, &t, dialect));
        out.push('\n');
    }
    if dialect != Dialect::Sqlite {
        for (table, name) in &deferred {
            let t = m.table(table).expect("deferred from this model");
            let c = t
                .constraints
                .iter()
                .find(|c| &c.name == name)
                .expect("deferred constraint");
            out.push_str(&format!(
                "ALTER TABLE {} ADD CONSTRAINT {} {};\n",
                qualified(&m.schema, table, dialect),
                quote_ident(name, dialect),
                c.definition
            ));
        }
        if !deferred.is_empty() {
            out.push('\n');
        }
    }
    for v in m.tables.iter().filter(|t| t.is_view()) {
        let mut v = v.clone();
        triggers.append(&mut v.triggers);
        out.push_str(&table_ddl(&m.schema, &v, dialect));
        out.push('\n');
    }
    for tr in &triggers {
        out.push_str(&statement(&tr.definition, dialect));
    }
    out.trim_end().to_owned() + "\n"
}

/// Tables ordered so each comes after the tables it refers to, and the foreign keys
/// (table, constraint) that had to be left out to break a cycle.
pub(crate) fn table_order(m: &SchemaModel) -> (Vec<&TableModel>, Vec<(String, String)>) {
    let tables: Vec<&TableModel> = m.tables.iter().filter(|t| !t.is_view()).collect();
    let mut done: Vec<&str> = vec![];
    let mut order = vec![];
    let mut deferred = vec![];
    let mut visiting: Vec<&str> = vec![];
    fn visit<'a>(
        t: &'a TableModel,
        m: &'a SchemaModel,
        tables: &[&'a TableModel],
        done: &mut Vec<&'a str>,
        visiting: &mut Vec<&'a str>,
        order: &mut Vec<&'a TableModel>,
        deferred: &mut Vec<(String, String)>,
    ) {
        if done.contains(&t.name.as_str()) {
            return;
        }
        visiting.push(&t.name);
        for fk in t.foreign_keys() {
            let Some(r) = &fk.references else { continue };
            if r.schema != m.schema || r.table == t.name {
                continue;
            }
            if visiting.contains(&r.table.as_str()) {
                deferred.push((t.name.clone(), fk.name.clone()));
                continue;
            }
            if let Some(target) = tables.iter().find(|x| x.name == r.table) {
                visit(target, m, tables, done, visiting, order, deferred);
            }
        }
        visiting.pop();
        done.push(&t.name);
        order.push(t);
    }
    for t in &tables {
        visit(
            t,
            m,
            &tables,
            &mut done,
            &mut visiting,
            &mut order,
            &mut deferred,
        );
    }
    (order, deferred)
}

/// The DDL for one object (or the whole schema).
pub async fn ddl(pool: &MetaPool, schema: &str, kind: ObjectKind, name: &str) -> Result<String> {
    let dialect: Dialect = pool.driver().into();
    // MySQL's own rendering of a table includes its engine, charset and options.
    if kind == ObjectKind::Table && pool.driver() == Driver::Mysql {
        let sql = format!(
            "SHOW CREATE TABLE {}",
            qualified(schema, name, Dialect::Mysql)
        );
        let rows = pool.text_rows(&sql, &[]).await?;
        if let Some(create) = rows.first().and_then(|r| r.get(1).cloned().flatten()) {
            let m = load(pool, schema).await?;
            let mut out = format!("{create};\n");
            if let Some(t) = m.table(name) {
                for tr in &t.triggers {
                    out.push_str(&statement(&tr.definition, dialect));
                }
            }
            return Ok(out);
        }
    }
    let m = load(pool, schema).await?;
    let missing = || CoreError::Invalid(format!("there's nothing called {name} in {schema}"));
    Ok(match kind {
        ObjectKind::Schema => schema_ddl(&m, dialect),
        ObjectKind::Table => table_ddl(schema, m.table(name).ok_or_else(missing)?, dialect),
        ObjectKind::Routine => {
            let defs: Vec<String> = m
                .routines
                .iter()
                .filter(|r| r.name == name)
                .map(|r| routine_ddl(r, dialect))
                .collect();
            if defs.is_empty() {
                return Err(missing());
            }
            defs.join("\n")
        }
        ObjectKind::Sequence => {
            let s = m
                .sequences
                .iter()
                .find(|s| s.name == name)
                .ok_or_else(missing)?;
            format!("{};\n", s.definition)
        }
    })
}

/// Read a schema's structure.
pub async fn load(pool: &MetaPool, schema: &str) -> Result<SchemaModel> {
    match pool.driver() {
        Driver::Postgres => load_postgres(pool, schema).await,
        Driver::Mysql => load_mysql(pool, schema).await,
        Driver::Sqlite => load_sqlite(pool, schema).await,
        Driver::SqlServer => load_sqlserver(pool, schema).await,
    }
}

type Row = Vec<Option<String>>;

fn col(r: &Row, i: usize) -> String {
    r.get(i).cloned().flatten().unwrap_or_default()
}

fn opt(r: &Row, i: usize) -> Option<String> {
    r.get(i).cloned().flatten().filter(|s| !s.is_empty())
}

fn split_names(s: &str) -> Vec<String> {
    if s.is_empty() {
        vec![]
    } else {
        s.split('\u{1f}').map(str::to_owned).collect()
    }
}

/// The tables (in order) and a map from name to index, to attach things to them.
struct Tables {
    list: Vec<TableModel>,
    by_name: BTreeMap<String, usize>,
}

impl Tables {
    fn new(list: Vec<TableModel>) -> Self {
        let by_name = list
            .iter()
            .enumerate()
            .map(|(i, t)| (t.name.clone(), i))
            .collect();
        Self { list, by_name }
    }

    fn get(&mut self, name: &str) -> Option<&mut TableModel> {
        self.by_name.get(name).map(|&i| &mut self.list[i])
    }
}

fn new_table(name: String, kind: TableKind) -> TableModel {
    TableModel {
        name,
        kind,
        comment: None,
        columns: vec![],
        constraints: vec![],
        indexes: vec![],
        triggers: vec![],
        definition: None,
    }
}

fn pg_rule(c: &str) -> String {
    match c {
        "r" => "RESTRICT",
        "c" => "CASCADE",
        "n" => "SET NULL",
        "d" => "SET DEFAULT",
        _ => "NO ACTION",
    }
    .into()
}

async fn load_postgres(pool: &MetaPool, schema: &str) -> Result<SchemaModel> {
    let d = Dialect::Postgres;
    let rows = pool
        .text_rows(
            "SELECT c.relname::text, c.relkind::text, obj_description(c.oid, 'pg_class')::text, \
             CASE WHEN c.relkind IN ('v', 'm') THEN pg_get_viewdef(c.oid, true) END::text \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'f') \
             AND NOT coalesce(c.relispartition, false) ORDER BY 1",
            &[schema],
        )
        .await?;
    let mut tables = Tables::new(
        rows.iter()
            .map(|r| {
                let kind = match col(r, 1).as_str() {
                    "v" => TableKind::View,
                    "m" => TableKind::MaterializedView,
                    "f" => TableKind::ForeignTable,
                    _ => TableKind::Table,
                };
                let mut t = new_table(col(r, 0), kind);
                t.comment = opt(r, 2);
                if let Some(body) = opt(r, 3) {
                    t.definition = Some(format!(
                        "CREATE {}VIEW {} AS\n{}",
                        if kind == TableKind::MaterializedView {
                            "MATERIALIZED "
                        } else {
                            ""
                        },
                        qualified(schema, &t.name, d),
                        body.trim_end().trim_end_matches(';')
                    ));
                }
                t
            })
            .collect(),
    );

    for r in pool
        .text_rows(
            "SELECT c.relname::text, a.attname::text, format_type(a.atttypid, a.atttypmod)::text, \
             a.attnotnull::text, pg_get_expr(ad.adbin, ad.adrelid)::text, a.attidentity::text, \
             a.attgenerated::text, col_description(c.oid, a.attnum)::text \
             FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             LEFT JOIN pg_attrdef ad ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum \
             WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'f') \
             AND NOT coalesce(c.relispartition, false) AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY c.relname, a.attnum",
            &[schema],
        )
        .await?
    {
        let Some(t) = tables.get(&col(&r, 0)) else {
            continue;
        };
        let expr = opt(&r, 4);
        let generated = opt(&r, 6).is_some_and(|g| g == "s" || g == "v");
        t.columns.push(ColumnModel {
            name: col(&r, 1),
            data_type: col(&r, 2),
            nullable: col(&r, 3) != "true",
            default: if generated { None } else { expr.clone() },
            identity: match col(&r, 5).as_str() {
                "a" => Some("GENERATED ALWAYS AS IDENTITY".into()),
                "d" => Some("GENERATED BY DEFAULT AS IDENTITY".into()),
                _ => None,
            },
            generated: generated.then(|| {
                format!(
                    "GENERATED ALWAYS AS ({}) {}",
                    expr.unwrap_or_default(),
                    if col(&r, 6) == "v" {
                        "VIRTUAL"
                    } else {
                        "STORED"
                    }
                )
            }),
            comment: opt(&r, 7),
        });
    }

    for r in pool
        .text_rows(
            "SELECT c.relname::text, k.conname::text, k.contype::text, \
             pg_get_constraintdef(k.oid, true)::text, \
             (SELECT string_agg(a.attname::text, chr(31) ORDER BY u.o) \
              FROM unnest(k.conkey) WITH ORDINALITY u(n, o) \
              JOIN pg_attribute a ON a.attrelid = k.conrelid AND a.attnum = u.n)::text, \
             fn.nspname::text, fc.relname::text, \
             (SELECT string_agg(a.attname::text, chr(31) ORDER BY u.o) \
              FROM unnest(k.confkey) WITH ORDINALITY u(n, o) \
              JOIN pg_attribute a ON a.attrelid = k.confrelid AND a.attnum = u.n)::text, \
             k.confupdtype::text, k.confdeltype::text \
             FROM pg_constraint k JOIN pg_class c ON c.oid = k.conrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             LEFT JOIN pg_class fc ON fc.oid = k.confrelid \
             LEFT JOIN pg_namespace fn ON fn.oid = fc.relnamespace \
             WHERE n.nspname = $1 AND k.contype IN ('p', 'u', 'f', 'c') AND k.conislocal \
             ORDER BY c.relname, k.contype = 'f', k.conname",
            &[schema],
        )
        .await?
    {
        let Some(t) = tables.get(&col(&r, 0)) else {
            continue;
        };
        let kind = match col(&r, 2).as_str() {
            "p" => ConstraintKind::PrimaryKey,
            "u" => ConstraintKind::Unique,
            "f" => ConstraintKind::ForeignKey,
            _ => ConstraintKind::Check,
        };
        t.constraints.push(Constraint {
            name: col(&r, 1),
            kind,
            columns: split_names(&col(&r, 4)),
            definition: col(&r, 3),
            references: (kind == ConstraintKind::ForeignKey).then(|| Reference {
                schema: col(&r, 5),
                table: col(&r, 6),
                columns: split_names(&col(&r, 7)),
                on_update: pg_rule(&col(&r, 8)),
                on_delete: pg_rule(&col(&r, 9)),
            }),
        });
    }

    for r in pool
        .text_rows(
            "SELECT t.relname::text, i.relname::text, x.indisunique::text, \
             pg_get_indexdef(x.indexrelid)::text, \
             (SELECT string_agg(pg_get_indexdef(x.indexrelid, k, true), chr(31) ORDER BY k) \
              FROM generate_series(1, x.indnkeyatts) k)::text \
             FROM pg_index x JOIN pg_class i ON i.oid = x.indexrelid \
             JOIN pg_class t ON t.oid = x.indrelid JOIN pg_namespace n ON n.oid = t.relnamespace \
             WHERE n.nspname = $1 AND NOT EXISTS (SELECT 1 FROM pg_constraint k \
               WHERE k.conindid = x.indexrelid AND k.conrelid = x.indrelid \
               AND k.contype IN ('p', 'u', 'x')) \
             ORDER BY 1, 2",
            &[schema],
        )
        .await?
    {
        let Some(t) = tables.get(&col(&r, 0)) else {
            continue;
        };
        t.indexes.push(Index {
            name: col(&r, 1),
            columns: split_names(&col(&r, 4)),
            unique: col(&r, 2) == "true",
            definition: col(&r, 3),
            inline: None,
        });
    }

    for r in pool
        .text_rows(
            "SELECT c.relname::text, t.tgname::text, pg_get_triggerdef(t.oid, true)::text \
             FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND NOT t.tgisinternal ORDER BY 1, 2",
            &[schema],
        )
        .await?
    {
        let Some(t) = tables.get(&col(&r, 0)) else {
            continue;
        };
        t.triggers.push(Trigger {
            name: col(&r, 1),
            definition: col(&r, 2),
        });
    }

    let routines = pool
        .text_rows(
            "SELECT p.proname::text, p.prokind::text, \
             pg_get_function_identity_arguments(p.oid)::text, \
             CASE WHEN p.prokind = 'f' THEN pg_get_function_result(p.oid) END::text, \
             pg_get_functiondef(p.oid)::text \
             FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
             WHERE n.nspname = $1 AND p.prokind IN ('f', 'p') AND NOT EXISTS ( \
               SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_proc'::regclass \
               AND d.objid = p.oid AND d.deptype = 'e') \
             ORDER BY 1, 3",
            &[schema],
        )
        .await?
        .iter()
        .map(|r| Routine {
            name: col(r, 0),
            kind: if col(r, 1) == "p" {
                RoutineKind::Procedure
            } else {
                RoutineKind::Function
            },
            arguments: col(r, 2),
            returns: opt(r, 3),
            definition: opt(r, 4).map(|d| d.trim_end().to_owned()),
        })
        .collect();

    let sequences = pool
        .text_rows(
            "SELECT c.relname::text, format_type(q.seqtypid, NULL)::text, q.seqstart::text, \
             q.seqmin::text, q.seqmax::text, q.seqincrement::text, q.seqcycle::text, \
             q.seqcache::text \
             FROM pg_sequence q JOIN pg_class c ON c.oid = q.seqrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND NOT EXISTS (SELECT 1 FROM pg_depend d \
               WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'i') \
             ORDER BY 1",
            &[schema],
        )
        .await?
        .iter()
        .map(|r| Sequence {
            name: col(r, 0),
            definition: format!(
                "CREATE SEQUENCE {} AS {} INCREMENT BY {} MINVALUE {} MAXVALUE {} START WITH {} \
                 CACHE {}{}",
                qualified(schema, &col(r, 0), d),
                col(r, 1),
                col(r, 5),
                col(r, 3),
                col(r, 4),
                col(r, 2),
                col(r, 7),
                if col(r, 6) == "true" { " CYCLE" } else { "" }
            ),
        })
        .collect();

    Ok(SchemaModel {
        schema: schema.to_owned(),
        tables: tables.list,
        routines,
        sequences,
    })
}

/// Remove `DEFINER=user@host ` from MySQL's `SHOW CREATE` output, so the statement
/// can run as someone else.
fn strip_definer(sql: &str) -> String {
    let Some(at) = sql.find("DEFINER=") else {
        return sql.to_owned();
    };
    let rest = &sql[at + "DEFINER=".len()..];
    // `user`@`host` (quoted parts may contain spaces), then a space.
    let mut chars = rest.char_indices().peekable();
    let mut quote: Option<char> = None;
    let mut end = rest.len();
    while let Some((i, ch)) = chars.next() {
        match quote {
            Some(q) if ch == q => {
                if chars.peek().map(|(_, c)| *c) == Some(q) {
                    chars.next();
                } else {
                    quote = None;
                }
            }
            Some(_) => {}
            None if ch == '`' || ch == '\'' || ch == '"' => quote = Some(ch),
            None if ch.is_whitespace() => {
                end = i + ch.len_utf8();
                break;
            }
            None => {}
        }
    }
    format!("{}{}", &sql[..at], &rest[end..])
}

/// MySQL reports defaults as bare text; make them SQL.
fn mysql_default(value: Option<String>, extra: &str, data_type: &str) -> Option<String> {
    let v = value?;
    if extra.contains("DEFAULT_GENERATED") {
        let upper = v.to_ascii_uppercase();
        return Some(
            if upper.starts_with("CURRENT_TIMESTAMP") || upper.starts_with("NOW(") {
                v
            } else {
                format!("({v})")
            },
        );
    }
    let numeric = matches!(
        data_type,
        "tinyint"
            | "smallint"
            | "mediumint"
            | "int"
            | "bigint"
            | "decimal"
            | "float"
            | "double"
            | "bit"
            | "year"
    );
    Some(
        if numeric && v.parse::<f64>().is_ok() || v.starts_with("b'") {
            v
        } else {
            literal(Some(&v), Dialect::Mysql)
        },
    )
}

async fn load_mysql(pool: &MetaPool, schema: &str) -> Result<SchemaModel> {
    let d = Dialect::Mysql;
    let rows = pool
        .text_rows(
            "SELECT CAST(table_name AS CHAR), CAST(table_type AS CHAR), \
             CAST(table_comment AS CHAR) FROM information_schema.tables \
             WHERE table_schema = ? ORDER BY 1",
            &[schema],
        )
        .await?;
    let mut tables = Tables::new(
        rows.iter()
            .map(|r| {
                let view = col(r, 1).contains("VIEW");
                let mut t = new_table(
                    col(r, 0),
                    if view {
                        TableKind::View
                    } else {
                        TableKind::Table
                    },
                );
                // Views have "VIEW" as their comment.
                t.comment = opt(r, 2).filter(|_| !view);
                t
            })
            .collect(),
    );
    for r in pool
        .text_rows(
            "SELECT CAST(table_name AS CHAR), CAST(view_definition AS CHAR) \
             FROM information_schema.views WHERE table_schema = ?",
            &[schema],
        )
        .await?
    {
        let name = col(&r, 0);
        if let Some(t) = tables.get(&name) {
            t.definition = Some(format!(
                "CREATE VIEW {} AS {}",
                qualified(schema, &name, d),
                col(&r, 1)
            ));
        }
    }

    for r in pool
        .text_rows(
            "SELECT CAST(table_name AS CHAR), CAST(column_name AS CHAR), \
             CAST(column_type AS CHAR), CAST(is_nullable AS CHAR), \
             CAST(column_default AS CHAR), CAST(extra AS CHAR), \
             CAST(generation_expression AS CHAR), CAST(column_comment AS CHAR), \
             CAST(data_type AS CHAR) \
             FROM information_schema.columns WHERE table_schema = ? \
             ORDER BY table_name, ordinal_position",
            &[schema],
        )
        .await?
    {
        let Some(t) = tables.get(&col(&r, 0)) else {
            continue;
        };
        let extra = col(&r, 5);
        let generated = opt(&r, 6).map(|expr| {
            format!(
                "GENERATED ALWAYS AS ({expr}) {}",
                if extra.contains("STORED") {
                    "STORED"
                } else {
                    "VIRTUAL"
                }
            )
        });
        t.columns.push(ColumnModel {
            name: col(&r, 1),
            data_type: col(&r, 2),
            nullable: col(&r, 3) == "YES",
            default: mysql_default(opt(&r, 4), &extra, &col(&r, 8)),
            identity: extra
                .contains("auto_increment")
                .then(|| "AUTO_INCREMENT".into()),
            generated,
            comment: opt(&r, 7),
        });
    }

    // Indexes, including the primary key. `expression` is MySQL 8.0.13+.
    let index_rows = match pool
        .text_rows(
            "SELECT CAST(table_name AS CHAR), CAST(index_name AS CHAR), \
             CAST(non_unique AS CHAR), CAST(column_name AS CHAR), CAST(index_type AS CHAR), \
             CAST(sub_part AS CHAR), CAST(expression AS CHAR) \
             FROM information_schema.statistics WHERE table_schema = ? \
             ORDER BY table_name, index_name, seq_in_index",
            &[schema],
        )
        .await
    {
        Ok(r) => r,
        Err(_) => {
            pool.text_rows(
                "SELECT CAST(table_name AS CHAR), CAST(index_name AS CHAR), \
                 CAST(non_unique AS CHAR), CAST(column_name AS CHAR), \
                 CAST(index_type AS CHAR), CAST(sub_part AS CHAR) \
                 FROM information_schema.statistics WHERE table_schema = ? \
                 ORDER BY table_name, index_name, seq_in_index",
                &[schema],
            )
            .await?
        }
    };
    /// Unique, index type, column names, and how each part is written.
    type Parts = (bool, String, Vec<String>, Vec<String>);
    let mut indexes: BTreeMap<(String, String), Parts> = BTreeMap::new();
    let mut index_order = vec![];
    for r in index_rows {
        let key = (col(&r, 0), col(&r, 1));
        let e = indexes.entry(key.clone()).or_insert_with(|| {
            index_order.push(key.clone());
            (col(&r, 2) == "0", col(&r, 4), vec![], vec![])
        });
        let (name, sql) = match (opt(&r, 3), opt(&r, 6)) {
            (Some(c), _) => {
                let mut sql = quote_ident(&c, d);
                if let Some(len) = opt(&r, 5) {
                    sql.push_str(&format!("({len})"));
                }
                (c, sql)
            }
            (None, Some(expr)) => (expr.clone(), format!("({expr})")),
            (None, None) => continue,
        };
        e.2.push(name);
        e.3.push(sql);
    }
    for key in index_order {
        let (unique, kind, columns, parts) = indexes.remove(&key).expect("collected");
        let (table, name) = key;
        let Some(t) = tables.get(&table) else {
            continue;
        };
        if name == "PRIMARY" {
            t.constraints.insert(
                0,
                Constraint {
                    name,
                    kind: ConstraintKind::PrimaryKey,
                    definition: format!("PRIMARY KEY ({})", column_list(&columns, d)),
                    columns,
                    references: None,
                },
            );
            continue;
        }
        let prefix = match kind.as_str() {
            "FULLTEXT" => "FULLTEXT ",
            "SPATIAL" => "SPATIAL ",
            _ if unique => "UNIQUE ",
            _ => "",
        };
        t.indexes.push(Index {
            definition: format!(
                "CREATE {prefix}INDEX {} ON {} ({})",
                quote_ident(&name, d),
                qualified(schema, &table, d),
                parts.join(", ")
            ),
            inline: Some(format!(
                "{prefix}KEY {} ({})",
                quote_ident(&name, d),
                parts.join(", ")
            )),
            name,
            columns,
            unique,
        });
    }

    let mut fks: BTreeMap<(String, String), Constraint> = BTreeMap::new();
    for r in pool
        .text_rows(
            "SELECT CAST(k.table_name AS CHAR), CAST(k.constraint_name AS CHAR), \
             CAST(k.column_name AS CHAR), CAST(k.referenced_table_schema AS CHAR), \
             CAST(k.referenced_table_name AS CHAR), CAST(k.referenced_column_name AS CHAR), \
             CAST(r.update_rule AS CHAR), CAST(r.delete_rule AS CHAR) \
             FROM information_schema.key_column_usage k \
             JOIN information_schema.referential_constraints r \
               ON r.constraint_schema = k.constraint_schema \
               AND r.constraint_name = k.constraint_name AND r.table_name = k.table_name \
             WHERE k.table_schema = ? AND k.referenced_table_name IS NOT NULL \
             ORDER BY k.table_name, k.constraint_name, k.ordinal_position",
            &[schema],
        )
        .await?
    {
        let c = fks
            .entry((col(&r, 0), col(&r, 1)))
            .or_insert_with(|| Constraint {
                name: col(&r, 1),
                kind: ConstraintKind::ForeignKey,
                columns: vec![],
                definition: String::new(),
                references: Some(Reference {
                    schema: col(&r, 3),
                    table: col(&r, 4),
                    columns: vec![],
                    on_update: col(&r, 6),
                    on_delete: col(&r, 7),
                }),
            });
        c.columns.push(col(&r, 2));
        if let Some(rf) = &mut c.references {
            rf.columns.push(col(&r, 5));
        }
    }

    // CHECK constraints are MySQL 8.0.16+.
    let checks = pool
        .text_rows(
            "SELECT CAST(t.table_name AS CHAR), CAST(c.constraint_name AS CHAR), \
             CAST(c.check_clause AS CHAR) FROM information_schema.check_constraints c \
             JOIN information_schema.table_constraints t \
               ON t.constraint_schema = c.constraint_schema \
               AND t.constraint_name = c.constraint_name \
             WHERE c.constraint_schema = ? AND t.constraint_type = 'CHECK' ORDER BY 1, 2",
            &[schema],
        )
        .await
        .unwrap_or_default();
    for r in checks {
        if let Some(t) = tables.get(&col(&r, 0)) {
            t.constraints.push(Constraint {
                name: col(&r, 1),
                kind: ConstraintKind::Check,
                columns: vec![],
                definition: format!("CHECK ({})", col(&r, 2)),
                references: None,
            });
        }
    }
    for ((table, _), mut c) in fks {
        if let Some(t) = tables.get(&table) {
            c.definition = fk_definition(&c, schema, d);
            t.constraints.push(c);
        }
    }

    for r in pool
        .text_rows(
            "SELECT CAST(event_object_table AS CHAR), CAST(trigger_name AS CHAR), \
             CAST(action_timing AS CHAR), CAST(event_manipulation AS CHAR), \
             CAST(action_statement AS CHAR) FROM information_schema.triggers \
             WHERE trigger_schema = ? ORDER BY 1, 2",
            &[schema],
        )
        .await?
    {
        let table = col(&r, 0);
        if let Some(t) = tables.get(&table) {
            t.triggers.push(Trigger {
                definition: format!(
                    "CREATE TRIGGER {} {} {} ON {} FOR EACH ROW {}",
                    quote_ident(&col(&r, 1), d),
                    col(&r, 2),
                    col(&r, 3),
                    qualified(schema, &table, d),
                    col(&r, 4)
                ),
                name: col(&r, 1),
            });
        }
    }

    let mut params: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in pool
        .text_rows(
            "SELECT CAST(specific_name AS CHAR), CAST(parameter_mode AS CHAR), \
             CAST(parameter_name AS CHAR), CAST(dtd_identifier AS CHAR) \
             FROM information_schema.parameters \
             WHERE specific_schema = ? AND ordinal_position > 0 \
             ORDER BY specific_name, ordinal_position",
            &[schema],
        )
        .await?
    {
        let mode = opt(&r, 1).map(|m| format!("{m} ")).unwrap_or_default();
        params.entry(col(&r, 0)).or_default().push(format!(
            "{mode}{} {}",
            quote_ident(&col(&r, 2), d),
            col(&r, 3)
        ));
    }
    let mut routines = vec![];
    for r in pool
        .text_rows(
            "SELECT CAST(routine_name AS CHAR), CAST(routine_type AS CHAR), \
             CAST(dtd_identifier AS CHAR), CAST(specific_name AS CHAR) \
             FROM information_schema.routines WHERE routine_schema = ? ORDER BY 1",
            &[schema],
        )
        .await?
    {
        let name = col(&r, 0);
        let kind = if col(&r, 1) == "PROCEDURE" {
            RoutineKind::Procedure
        } else {
            RoutineKind::Function
        };
        let show = format!(
            "SHOW CREATE {} {}",
            if kind == RoutineKind::Procedure {
                "PROCEDURE"
            } else {
                "FUNCTION"
            },
            qualified(schema, &name, d)
        );
        // Column 2 is the statement; it's NULL without the privilege to see it.
        let definition = pool
            .text_rows(&show, &[])
            .await
            .ok()
            .and_then(|rows| rows.first().and_then(|row| opt(row, 2)))
            .map(|s| strip_definer(&s));
        routines.push(Routine {
            arguments: params.remove(&col(&r, 3)).unwrap_or_default().join(", "),
            returns: opt(&r, 2).filter(|_| kind == RoutineKind::Function),
            name,
            kind,
            definition,
        });
    }

    Ok(SchemaModel {
        schema: schema.to_owned(),
        tables: tables.list,
        routines,
        sequences: vec![],
    })
}

/// `FOREIGN KEY (a) REFERENCES t (id) ON DELETE CASCADE`, from its parts.
pub fn fk_definition(c: &Constraint, schema: &str, d: Dialect) -> String {
    let Some(r) = &c.references else {
        return String::new();
    };
    let target = if r.schema == schema || r.schema.is_empty() {
        quote_ident(&r.table, d)
    } else {
        qualified(&r.schema, &r.table, d)
    };
    let mut s = format!(
        "FOREIGN KEY ({}) REFERENCES {target}",
        column_list(&c.columns, d)
    );
    if !r.columns.is_empty() {
        s.push_str(&format!(" ({})", column_list(&r.columns, d)));
    }
    for (what, rule) in [("UPDATE", &r.on_update), ("DELETE", &r.on_delete)] {
        if !rule.is_empty() && rule != "NO ACTION" {
            s.push_str(&format!(" ON {what} {rule}"));
        }
    }
    s
}

async fn load_sqlite(pool: &MetaPool, schema: &str) -> Result<SchemaModel> {
    let d = Dialect::Sqlite;
    let master = format!("{}.sqlite_master", quote_ident(schema, d));
    let objects = pool
        .text_rows(
            &format!(
                "SELECT type, name, tbl_name, sql FROM {master} \
                 WHERE name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name"
            ),
            &[],
        )
        .await?;
    let mut tables = Tables::new(
        objects
            .iter()
            .filter(|r| matches!(col(r, 0).as_str(), "table" | "view"))
            .map(|r| {
                let mut t = new_table(
                    col(r, 1),
                    if col(r, 0) == "view" {
                        TableKind::View
                    } else {
                        TableKind::Table
                    },
                );
                t.definition = opt(r, 3);
                t
            })
            .collect(),
    );
    let mut pk: BTreeMap<String, Vec<(i64, String)>> = BTreeMap::new();
    for r in pool
        .text_rows(
            &format!(
                "SELECT m.name, c.name, c.type, c.\"notnull\", c.dflt_value, c.pk, c.hidden \
                 FROM {master} m JOIN pragma_table_xinfo(m.name, ?1) c \
                 WHERE m.type IN ('table', 'view') ORDER BY m.name, c.cid"
            ),
            &[schema],
        )
        .await?
    {
        let table = col(&r, 0);
        let Some(t) = tables.get(&table) else {
            continue;
        };
        let order: i64 = col(&r, 5).parse().unwrap_or(0);
        if order > 0 {
            pk.entry(table).or_default().push((order, col(&r, 1)));
        }
        // Hidden 2 and 3 are generated columns (their expression is in the table's SQL).
        let hidden = col(&r, 6);
        t.columns.push(ColumnModel {
            name: col(&r, 1),
            data_type: col(&r, 2),
            nullable: col(&r, 3) != "1",
            default: opt(&r, 4),
            identity: None,
            generated: matches!(hidden.as_str(), "2" | "3").then(|| {
                if hidden == "3" {
                    "GENERATED ALWAYS AS (…) STORED".into()
                } else {
                    "GENERATED ALWAYS AS (…) VIRTUAL".into()
                }
            }),
            comment: None,
        });
    }
    for (table, mut cols) in pk {
        cols.sort();
        let columns: Vec<String> = cols.into_iter().map(|(_, c)| c).collect();
        if let Some(t) = tables.get(&table)
            && !t.is_view()
        {
            t.constraints.push(Constraint {
                name: format!("{table}_pkey"),
                kind: ConstraintKind::PrimaryKey,
                definition: format!("PRIMARY KEY ({})", column_list(&columns, d)),
                columns,
                references: None,
            });
        }
    }

    let mut fks: BTreeMap<(String, i64), Constraint> = BTreeMap::new();
    for r in pool
        .text_rows(
            &format!(
                "SELECT m.name, f.id, f.\"table\", f.\"from\", f.\"to\", f.on_update, f.on_delete \
                 FROM {master} m JOIN pragma_foreign_key_list(m.name, ?1) f \
                 WHERE m.type = 'table' ORDER BY m.name, f.id, f.seq"
            ),
            &[schema],
        )
        .await?
    {
        let table = col(&r, 0);
        let id: i64 = col(&r, 1).parse().unwrap_or(0);
        let c = fks
            .entry((table.clone(), id))
            .or_insert_with(|| Constraint {
                name: format!("{table}_fk{}", id + 1),
                kind: ConstraintKind::ForeignKey,
                columns: vec![],
                definition: String::new(),
                references: Some(Reference {
                    schema: schema.to_owned(),
                    table: col(&r, 2),
                    columns: vec![],
                    on_update: col(&r, 5),
                    on_delete: col(&r, 6),
                }),
            });
        c.columns.push(col(&r, 3));
        if let (Some(rf), Some(to)) = (&mut c.references, opt(&r, 4)) {
            rf.columns.push(to);
        }
    }
    for ((table, _), mut c) in fks {
        c.definition = fk_definition(&c, schema, d);
        if let Some(t) = tables.get(&table) {
            t.constraints.push(c);
        }
    }

    let index_sql: BTreeMap<String, String> = objects
        .iter()
        .filter(|r| col(r, 0) == "index")
        .filter_map(|r| Some((col(r, 1), opt(r, 3)?)))
        .collect();
    let mut idx: BTreeMap<(String, String), (bool, String, Vec<String>)> = BTreeMap::new();
    for r in pool
        .text_rows(
            &format!(
                "SELECT m.name, l.name, l.\"unique\", l.origin, i.name \
                 FROM {master} m JOIN pragma_index_list(m.name, ?1) l \
                 JOIN pragma_index_info(l.name, ?1) i \
                 WHERE m.type = 'table' ORDER BY m.name, l.name, i.seqno"
            ),
            &[schema],
        )
        .await?
    {
        let e = idx
            .entry((col(&r, 0), col(&r, 1)))
            .or_insert_with(|| (col(&r, 2) == "1", col(&r, 3), vec![]));
        e.2.push(opt(&r, 4).unwrap_or_else(|| "<expression>".into()));
    }
    for ((table, name), (unique, origin, columns)) in idx {
        let Some(t) = tables.get(&table) else {
            continue;
        };
        match origin.as_str() {
            "c" => t.indexes.push(Index {
                definition: index_sql.get(&name).cloned().unwrap_or_default(),
                inline: None,
                name,
                columns,
                unique,
            }),
            "u" => t.constraints.push(Constraint {
                definition: format!("UNIQUE ({})", column_list(&columns, d)),
                name,
                kind: ConstraintKind::Unique,
                columns,
                references: None,
            }),
            _ => {}
        }
    }
    for r in objects.iter().filter(|r| col(r, 0) == "trigger") {
        if let Some(t) = tables.get(&col(r, 2)) {
            t.triggers.push(Trigger {
                name: col(r, 1),
                definition: col(r, 3),
            });
        }
    }

    Ok(SchemaModel {
        schema: schema.to_owned(),
        tables: tables.list,
        routines: vec![],
        sequences: vec![],
    })
}

async fn load_sqlserver(pool: &MetaPool, schema: &str) -> Result<SchemaModel> {
    let d = Dialect::SqlServer;
    #[cfg(feature = "sqlserver")]
    let ms_type = crate::db::MS_TYPE;
    #[cfg(not(feature = "sqlserver"))]
    let ms_type = "tp.name";
    // The objects in the schema, by id.
    const IN_SCHEMA: &str = "JOIN sys.schemas s ON s.schema_id = o.schema_id \
        WHERE s.name = @p1 AND o.is_ms_shipped = 0";
    let objects = pool
        .text_rows(
            &format!(
                "SELECT o.name, RTRIM(o.type), OBJECT_DEFINITION(o.object_id), \
                 CAST(ep.value AS nvarchar(4000)) FROM sys.objects o \
                 LEFT JOIN sys.extended_properties ep ON ep.class = 1 \
                   AND ep.major_id = o.object_id AND ep.minor_id = 0 \
                   AND ep.name = 'MS_Description' \
                 {IN_SCHEMA} AND o.type IN ('U', 'V') ORDER BY o.name"
            ),
            &[schema],
        )
        .await?;
    let mut tables = Tables::new(
        objects
            .iter()
            .map(|r| {
                let view = col(r, 1) == "V";
                let mut t = new_table(
                    col(r, 0),
                    if view {
                        TableKind::View
                    } else {
                        TableKind::Table
                    },
                );
                if view {
                    t.definition = opt(r, 2).map(|s| s.trim().to_owned());
                }
                t.comment = opt(r, 3);
                t
            })
            .collect(),
    );

    for r in pool
        .text_rows(
            &format!(
                "SELECT o.name, c.name, {ms_type}, c.is_nullable, \
                 OBJECT_DEFINITION(c.default_object_id), \
                 CASE WHEN ic.column_id IS NOT NULL THEN 'IDENTITY(' + \
                   CAST(ic.seed_value AS varchar(40)) + ',' + \
                   CAST(ic.increment_value AS varchar(40)) + ')' END, \
                 cc.definition, cc.is_persisted, CAST(ep.value AS nvarchar(4000)) \
                 FROM sys.columns c JOIN sys.objects o ON o.object_id = c.object_id \
                 JOIN sys.types tp ON tp.user_type_id = c.user_type_id \
                 LEFT JOIN sys.identity_columns ic ON ic.object_id = c.object_id \
                   AND ic.column_id = c.column_id \
                 LEFT JOIN sys.computed_columns cc ON cc.object_id = c.object_id \
                   AND cc.column_id = c.column_id \
                 LEFT JOIN sys.extended_properties ep ON ep.class = 1 \
                   AND ep.major_id = c.object_id AND ep.minor_id = c.column_id \
                   AND ep.name = 'MS_Description' \
                 {IN_SCHEMA} AND o.type IN ('U', 'V') ORDER BY o.name, c.column_id"
            ),
            &[schema],
        )
        .await?
    {
        let Some(t) = tables.get(&col(&r, 0)) else {
            continue;
        };
        let computed = opt(&r, 6);
        t.columns.push(ColumnModel {
            name: col(&r, 1),
            // A computed column has no type of its own in its definition.
            data_type: if computed.is_some() {
                String::new()
            } else {
                col(&r, 2)
            },
            nullable: col(&r, 3) == "1",
            default: opt(&r, 4),
            identity: opt(&r, 5),
            generated: computed.map(|c| {
                format!(
                    "AS {c}{}",
                    if col(&r, 7) == "1" { " PERSISTED" } else { "" }
                )
            }),
            comment: opt(&r, 8),
        });
    }

    // Keys (primary and unique), with their columns in order.
    let mut keys: BTreeMap<(String, String), Constraint> = BTreeMap::new();
    for r in pool
        .text_rows(
            &format!(
                "SELECT o.name, k.name, k.type, i.type_desc, c.name \
                 FROM sys.key_constraints k JOIN sys.objects o ON o.object_id = k.parent_object_id \
                 JOIN sys.indexes i ON i.object_id = k.parent_object_id \
                   AND i.index_id = k.unique_index_id \
                 JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id \
                 JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id \
                 {IN_SCHEMA} ORDER BY o.name, k.name, ic.key_ordinal"
            ),
            &[schema],
        )
        .await?
    {
        let primary = col(&r, 2).trim() == "PK";
        let c = keys
            .entry((col(&r, 0), col(&r, 1)))
            .or_insert_with(|| Constraint {
                name: col(&r, 1),
                kind: if primary {
                    ConstraintKind::PrimaryKey
                } else {
                    ConstraintKind::Unique
                },
                columns: vec![],
                // Finished below, with the columns.
                definition: format!(
                    "{} {}",
                    if primary { "PRIMARY KEY" } else { "UNIQUE" },
                    col(&r, 3)
                ),
                references: None,
            });
        c.columns.push(col(&r, 4));
    }
    for ((table, _), mut c) in keys {
        c.definition = format!("{} ({})", c.definition, column_list(&c.columns, d));
        if let Some(t) = tables.get(&table) {
            if c.kind == ConstraintKind::PrimaryKey {
                t.constraints.insert(0, c);
            } else {
                t.constraints.push(c);
            }
        }
    }
    for r in pool
        .text_rows(
            &format!(
                "SELECT o.name, k.name, k.definition FROM sys.check_constraints k \
                 JOIN sys.objects o ON o.object_id = k.parent_object_id \
                 {IN_SCHEMA} ORDER BY o.name, k.name"
            ),
            &[schema],
        )
        .await?
    {
        if let Some(t) = tables.get(&col(&r, 0)) {
            t.constraints.push(Constraint {
                name: col(&r, 1),
                kind: ConstraintKind::Check,
                columns: vec![],
                definition: format!("CHECK {}", col(&r, 2)),
                references: None,
            });
        }
    }
    let mut fks: BTreeMap<(String, String), Constraint> = BTreeMap::new();
    for r in pool
        .text_rows(
            &format!(
                "SELECT o.name, f.name, pc.name, rs.name, ro.name, rc.name, \
                 f.update_referential_action_desc, f.delete_referential_action_desc \
                 FROM sys.foreign_keys f JOIN sys.objects o ON o.object_id = f.parent_object_id \
                 JOIN sys.foreign_key_columns fc ON fc.constraint_object_id = f.object_id \
                 JOIN sys.columns pc ON pc.object_id = fc.parent_object_id \
                   AND pc.column_id = fc.parent_column_id \
                 JOIN sys.objects ro ON ro.object_id = fc.referenced_object_id \
                 JOIN sys.schemas rs ON rs.schema_id = ro.schema_id \
                 JOIN sys.columns rc ON rc.object_id = fc.referenced_object_id \
                   AND rc.column_id = fc.referenced_column_id \
                 {IN_SCHEMA} ORDER BY o.name, f.name, fc.constraint_column_id"
            ),
            &[schema],
        )
        .await?
    {
        let rule = |i| col(&r, i).replace('_', " ");
        let c = fks
            .entry((col(&r, 0), col(&r, 1)))
            .or_insert_with(|| Constraint {
                name: col(&r, 1),
                kind: ConstraintKind::ForeignKey,
                columns: vec![],
                definition: String::new(),
                references: Some(Reference {
                    schema: col(&r, 3),
                    table: col(&r, 4),
                    columns: vec![],
                    on_update: rule(6),
                    on_delete: rule(7),
                }),
            });
        c.columns.push(col(&r, 2));
        if let Some(rf) = &mut c.references {
            rf.columns.push(col(&r, 5));
        }
    }
    for ((table, _), mut c) in fks {
        c.definition = fk_definition(&c, schema, d);
        if let Some(t) = tables.get(&table) {
            t.constraints.push(c);
        }
    }

    // Indexes that aren't a key's.
    /// Unique, type, filter, key columns, how each is written, included columns.
    type Parts = (bool, String, String, Vec<String>, Vec<String>, Vec<String>);
    let mut idx: BTreeMap<(String, String), Parts> = BTreeMap::new();
    for r in pool
        .text_rows(
            &format!(
                "SELECT o.name, i.name, i.is_unique, i.type_desc, i.filter_definition, \
                 c.name, ic.is_descending_key, ic.is_included_column \
                 FROM sys.indexes i JOIN sys.objects o ON o.object_id = i.object_id \
                 JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id \
                 JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id \
                 {IN_SCHEMA} AND i.is_primary_key = 0 AND i.is_unique_constraint = 0 \
                   AND i.is_hypothetical = 0 AND i.type > 0 \
                 ORDER BY o.name, i.name, ic.key_ordinal, ic.index_column_id"
            ),
            &[schema],
        )
        .await?
    {
        let e = idx.entry((col(&r, 0), col(&r, 1))).or_insert_with(|| {
            (
                col(&r, 2) == "1",
                col(&r, 3),
                col(&r, 4),
                vec![],
                vec![],
                vec![],
            )
        });
        let name = col(&r, 5);
        if col(&r, 7) == "1" {
            e.5.push(quote_ident(&name, d));
        } else {
            let mut part = quote_ident(&name, d);
            if col(&r, 6) == "1" {
                part.push_str(" DESC");
            }
            e.4.push(part);
            e.3.push(name);
        }
    }
    for ((table, name), (unique, kind, filter, columns, parts, include)) in idx {
        let Some(t) = tables.get(&table) else {
            continue;
        };
        let mut def = format!(
            "CREATE {}{} INDEX {} ON {} ({})",
            if unique { "UNIQUE " } else { "" },
            kind,
            quote_ident(&name, d),
            qualified(schema, &table, d),
            parts.join(", ")
        );
        if !include.is_empty() {
            def.push_str(&format!(" INCLUDE ({})", include.join(", ")));
        }
        if !filter.is_empty() {
            def.push_str(&format!(" WHERE {filter}"));
        }
        t.indexes.push(Index {
            name,
            columns,
            unique,
            definition: def,
            inline: None,
        });
    }
    for r in pool
        .text_rows(
            "SELECT o.name, t.name, OBJECT_DEFINITION(t.object_id) FROM sys.triggers t \
             JOIN sys.objects o ON o.object_id = t.parent_id \
             JOIN sys.schemas s ON s.schema_id = o.schema_id WHERE s.name = @p1 \
             ORDER BY o.name, t.name",
            &[schema],
        )
        .await?
    {
        if let Some(t) = tables.get(&col(&r, 0)) {
            t.triggers.push(Trigger {
                name: col(&r, 1),
                definition: col(&r, 2).trim().to_owned(),
            });
        }
    }

    let mut params: BTreeMap<String, (Vec<String>, Option<String>)> = BTreeMap::new();
    for r in pool
        .text_rows(
            &format!(
                "SELECT o.name, p.name, TYPE_NAME(p.user_type_id), p.is_output, p.parameter_id \
                 FROM sys.parameters p JOIN sys.objects o ON o.object_id = p.object_id \
                 {IN_SCHEMA} AND o.type IN ('P', 'FN', 'IF', 'TF') ORDER BY o.name, p.parameter_id"
            ),
            &[schema],
        )
        .await?
    {
        let e = params.entry(col(&r, 0)).or_default();
        if col(&r, 4) == "0" {
            // The return value of a scalar function.
            e.1 = opt(&r, 2);
        } else {
            e.0.push(format!(
                "{} {}{}",
                col(&r, 1),
                col(&r, 2),
                if col(&r, 3) == "1" { " OUTPUT" } else { "" }
            ));
        }
    }
    let routines = pool
        .text_rows(
            &format!(
                "SELECT o.name, RTRIM(o.type), OBJECT_DEFINITION(o.object_id) FROM sys.objects o \
                 {IN_SCHEMA} AND o.type IN ('P', 'FN', 'IF', 'TF') ORDER BY o.name"
            ),
            &[schema],
        )
        .await?
        .iter()
        .map(|r| {
            let (args, ret) = params.remove(&col(r, 0)).unwrap_or_default();
            let kind = if col(r, 1) == "P" {
                RoutineKind::Procedure
            } else {
                RoutineKind::Function
            };
            Routine {
                name: col(r, 0),
                kind,
                arguments: args.join(", "),
                returns: ret
                    .or_else(|| (kind == RoutineKind::Function).then(|| "TABLE".to_owned())),
                definition: opt(r, 2).map(|s| s.trim().to_owned()),
            }
        })
        .collect();
    let sequences = pool
        .text_rows(
            "SELECT q.name, TYPE_NAME(q.user_type_id), CAST(q.start_value AS varchar(40)), \
             CAST(q.increment AS varchar(40)), CAST(q.minimum_value AS varchar(40)), \
             CAST(q.maximum_value AS varchar(40)), q.is_cycling, q.cache_size \
             FROM sys.sequences q JOIN sys.schemas s ON s.schema_id = q.schema_id \
             WHERE s.name = @p1 ORDER BY q.name",
            &[schema],
        )
        .await?
        .iter()
        .map(|r| Sequence {
            name: col(r, 0),
            definition: format!(
                "CREATE SEQUENCE {} AS {} START WITH {} INCREMENT BY {} MINVALUE {} \
                 MAXVALUE {}{}{}",
                qualified(schema, &col(r, 0), d),
                col(r, 1),
                col(r, 2),
                col(r, 3),
                col(r, 4),
                col(r, 5),
                if col(r, 6) == "1" { " CYCLE" } else { "" },
                opt(r, 7).map(|c| format!(" CACHE {c}")).unwrap_or_default()
            ),
        })
        .collect();

    Ok(SchemaModel {
        schema: schema.to_owned(),
        tables: tables.list,
        routines,
        sequences,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_the_definer() {
        assert_eq!(
            strip_definer("CREATE DEFINER=`root`@`%` FUNCTION `f`() RETURNS int RETURN 1"),
            "CREATE FUNCTION `f`() RETURNS int RETURN 1"
        );
        assert_eq!(
            strip_definer("CREATE DEFINER=`a b`@`local host` PROCEDURE p() BEGIN END"),
            "CREATE PROCEDURE p() BEGIN END"
        );
        assert_eq!(
            strip_definer("CREATE VIEW v AS SELECT 1"),
            "CREATE VIEW v AS SELECT 1"
        );
    }

    #[test]
    fn mysql_defaults_become_sql() {
        let def = |v: &str, extra: &str, ty: &str| mysql_default(Some(v.into()), extra, ty);
        assert_eq!(def("0", "", "int").as_deref(), Some("0"));
        assert_eq!(def("new", "", "varchar").as_deref(), Some("'new'"));
        assert_eq!(def("1", "", "varchar").as_deref(), Some("'1'"));
        assert_eq!(
            def("CURRENT_TIMESTAMP", "DEFAULT_GENERATED", "timestamp").as_deref(),
            Some("CURRENT_TIMESTAMP")
        );
        assert_eq!(
            def("uuid()", "DEFAULT_GENERATED", "char").as_deref(),
            Some("(uuid())")
        );
    }

    fn table(name: &str, refs: &[&str]) -> TableModel {
        let mut t = new_table(name.into(), TableKind::Table);
        t.columns.push(ColumnModel {
            name: "id".into(),
            data_type: "integer".into(),
            ..ColumnModel::default()
        });
        for (i, r) in refs.iter().enumerate() {
            t.constraints.push(Constraint {
                name: format!("{name}_fk{i}"),
                kind: ConstraintKind::ForeignKey,
                columns: vec!["id".into()],
                definition: format!("FOREIGN KEY (id) REFERENCES {r} (id)"),
                references: Some(Reference {
                    schema: "s".into(),
                    table: (*r).into(),
                    columns: vec!["id".into()],
                    on_update: "NO ACTION".into(),
                    on_delete: "NO ACTION".into(),
                }),
            });
        }
        t
    }

    #[test]
    fn referenced_tables_come_first_and_cycles_are_broken() {
        let m = SchemaModel {
            schema: "s".into(),
            tables: vec![
                table("a", &["b"]),
                table("b", &["c"]),
                table("c", &[]),
                table("x", &["y"]),
                table("y", &["x"]),
            ],
            ..SchemaModel::default()
        };
        let (order, deferred) = table_order(&m);
        let names: Vec<&str> = order.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["c", "b", "a", "y", "x"]);
        assert_eq!(deferred, [("y".to_owned(), "y_fk0".to_owned())]);
        let ddl = schema_ddl(&m, Dialect::Postgres);
        assert!(
            ddl.contains("ALTER TABLE \"s\".\"y\" ADD CONSTRAINT \"y_fk0\" FOREIGN KEY"),
            "{ddl}"
        );
    }
}
