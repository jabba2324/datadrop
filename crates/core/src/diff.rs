//! Turning one structure into another: the statements that change a table into its
//! edited version (the table editor), or a schema into another (schema diff).

use serde::{Deserialize, Serialize};

use crate::edit::literal;
use crate::schema::{
    ColumnModel, Constraint, ConstraintKind, Index, RoutineKind, SchemaModel, TableModel,
    column_list, column_sql, create_table, qualified, table_ddl, table_order,
};
use crate::sql::{Dialect, quote_ident};

/// The statements, in order, and what to know before running them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Migration {
    pub statements: Vec<String>,
    pub notes: Vec<String>,
}

impl Migration {
    fn push(&mut self, s: String) {
        self.statements.push(s);
    }

    fn note(&mut self, n: impl Into<String>) {
        let n = n.into();
        if !self.notes.contains(&n) {
            self.notes.push(n);
        }
    }

    /// As a script.
    pub fn script(&self) -> String {
        let mut s = String::new();
        for n in &self.notes {
            s.push_str(&format!("-- {n}\n"));
        }
        if !self.notes.is_empty() {
            s.push('\n');
        }
        for st in &self.statements {
            s.push_str(st.trim_end().trim_end_matches(';'));
            s.push_str(";\n");
        }
        s
    }
}

/// An edit of a table: what it was (`None` for a new table), what it should be, and
/// which columns were renamed (old name, new name).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableEdit {
    pub schema: String,
    #[serde(default)]
    pub old: Option<TableModel>,
    pub new: TableModel,
    #[serde(default)]
    pub renamed_columns: Vec<(String, String)>,
}

/// `CREATE [UNIQUE] INDEX name ON table (columns)`, for an index made in the editor.
pub fn index_definition(schema: &str, table: &str, i: &Index, d: Dialect) -> String {
    let index = if d == Dialect::Postgres || d == Dialect::Mysql {
        quote_ident(&i.name, d)
    } else {
        qualified(schema, &i.name, d)
    };
    format!(
        "CREATE {}INDEX {index} ON {} ({})",
        if i.unique { "UNIQUE " } else { "" },
        qualified(schema, table, d),
        column_list(&i.columns, d)
    )
}

pub fn table_edit(edit: &TableEdit, d: Dialect) -> Migration {
    let mut m = Migration::default();
    match &edit.old {
        None => {
            let mut t = edit.new.clone();
            for i in &mut t.indexes {
                if i.definition.is_empty() {
                    i.definition = index_definition(&edit.schema, &t.name, i, d);
                }
            }
            for s in split_ddl(&table_ddl(&edit.schema, &t, d)) {
                m.push(s);
            }
        }
        Some(old) => alter_table(
            &mut m,
            &edit.schema,
            old,
            &edit.new,
            &edit.renamed_columns,
            d,
        ),
    }
    m
}

/// `table_ddl`'s output as separate statements.
fn split_ddl(ddl: &str) -> Vec<String> {
    // Each statement ends a line with `;` (bodies with `;` inside are only in
    // triggers, which come last and whole).
    let mut out = vec![];
    let mut cur = String::new();
    for line in ddl.lines() {
        if line.starts_with("DELIMITER") {
            continue;
        }
        // SQL Server's batch separator ends the statement before it.
        if line.trim().eq_ignore_ascii_case("GO") {
            let s = cur.trim().trim_end_matches(';').to_owned();
            if !s.is_empty() {
                out.push(s);
            }
            cur.clear();
            continue;
        }
        cur.push_str(line);
        cur.push('\n');
        let t = line.trim_end();
        if t.ends_with(';') || t.ends_with("$$") {
            let s = cur
                .trim()
                .trim_end_matches("$$")
                .trim_end_matches(';')
                .to_owned();
            if !s.is_empty() {
                out.push(s);
            }
            cur.clear();
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().trim_end_matches(';').to_owned());
    }
    out
}

fn alter_table(
    m: &mut Migration,
    schema: &str,
    old: &TableModel,
    new: &TableModel,
    renamed: &[(String, String)],
    d: Dialect,
) {
    let mut name = qualified(schema, &old.name, d);
    if old.name != new.name {
        m.push(match d {
            Dialect::Mysql => format!("RENAME TABLE {name} TO {}", qualified(schema, &new.name, d)),
            Dialect::SqlServer => format!(
                "EXEC sp_rename {}, {}",
                literal(Some(&name), d),
                literal(Some(&new.name), d)
            ),
            _ => format!("ALTER TABLE {name} RENAME TO {}", quote_ident(&new.name, d)),
        });
        name = qualified(schema, &new.name, d);
    }
    if d == Dialect::Sqlite && needs_rebuild(old, new, renamed) {
        rebuild_sqlite(m, schema, old, new, renamed);
        return;
    }
    // What each new column was called before (itself, unless renamed).
    let was = |c: &str| -> String {
        renamed
            .iter()
            .find(|(_, n)| n == c)
            .map(|(o, _)| o.clone())
            .unwrap_or_else(|| c.to_owned())
    };
    let kept: Vec<String> = new
        .columns
        .iter()
        .map(|c| was(&c.name))
        .filter(|o| old.column(o).is_some())
        .collect();

    // Constraints and indexes first out (they may use columns that go).
    let old_cons = &old.constraints;
    let new_cons = &new.constraints;
    for c in old_cons {
        let same = new_cons.iter().any(|n| same_constraint(c, n, renamed));
        if !same {
            m.push(drop_constraint(&name, c, d));
        }
    }
    for i in &old.indexes {
        if !new.indexes.iter().any(|n| same_index(i, n)) {
            m.push(drop_index(schema, &name, &i.name, d));
        }
    }
    for t in &old.triggers {
        if !new
            .triggers
            .iter()
            .any(|n| n.name == t.name && n.definition == t.definition)
        {
            m.push(match d {
                Dialect::Postgres => {
                    format!("DROP TRIGGER {} ON {name}", quote_ident(&t.name, d))
                }
                _ => format!("DROP TRIGGER {}", qualified(schema, &t.name, d)),
            });
        }
    }

    // Columns.
    for (o, n) in renamed {
        if old.column(o).is_some() && new.column(n).is_some() && o != n {
            m.push(if d == Dialect::SqlServer {
                format!(
                    "EXEC sp_rename {}, {}, N'COLUMN'",
                    literal(Some(&format!("{name}.{}", quote_ident(o, d))), d),
                    literal(Some(n), d)
                )
            } else {
                format!(
                    "ALTER TABLE {name} RENAME COLUMN {} TO {}",
                    quote_ident(o, d),
                    quote_ident(n, d)
                )
            });
        }
    }
    for c in &old.columns {
        if !kept.contains(&c.name) {
            if d == Dialect::SqlServer && c.default.is_some() {
                m.push(drop_default(&name, &c.name));
            }
            m.push(format!(
                "ALTER TABLE {name} DROP COLUMN {}",
                quote_ident(&c.name, d)
            ));
        }
    }
    for (pos, c) in new.columns.iter().enumerate() {
        let before = old.column(&was(&c.name));
        match before {
            None => {
                let mut add = if d == Dialect::SqlServer {
                    format!("ALTER TABLE {name} ADD {}", column_sql(c, d))
                } else {
                    format!("ALTER TABLE {name} ADD COLUMN {}", column_sql(c, d))
                };
                if d == Dialect::Mysql {
                    add.push_str(&match pos {
                        0 => " FIRST".to_owned(),
                        _ => format!(" AFTER {}", quote_ident(&new.columns[pos - 1].name, d)),
                    });
                }
                if !c.nullable && c.default.is_none() && c.identity.is_none() {
                    m.note(format!(
                        "{} is NOT NULL without a default: adding it fails if {} has rows",
                        c.name, new.name
                    ));
                }
                m.push(add);
            }
            Some(b) => alter_column(m, &name, b, c, d),
        }
    }
    if d == Dialect::Mysql && old.comment != new.comment {
        m.push(format!(
            "ALTER TABLE {name} COMMENT = {}",
            literal(Some(new.comment.as_deref().unwrap_or("")), d)
        ));
    }
    if d == Dialect::Postgres && old.comment != new.comment {
        m.push(format!(
            "COMMENT ON TABLE {name} IS {}",
            literal(new.comment.as_deref(), d)
        ));
    }

    // Then the new constraints, indexes and triggers.
    for c in new_cons {
        if !old_cons.iter().any(|o| same_constraint(o, c, renamed)) {
            m.push(add_constraint(&name, c, d));
        }
    }
    for i in &new.indexes {
        if !old.indexes.iter().any(|o| same_index(o, i)) {
            m.push(if i.definition.is_empty() {
                index_definition(schema, &new.name, i, d)
            } else {
                i.definition.clone()
            });
        }
    }
    for t in &new.triggers {
        if !old
            .triggers
            .iter()
            .any(|o| o.name == t.name && o.definition == t.definition)
        {
            m.push(t.definition.clone());
        }
    }
}

fn alter_column(m: &mut Migration, table: &str, b: &ColumnModel, c: &ColumnModel, d: Dialect) {
    let col = quote_ident(&c.name, d);
    let same = |x: &ColumnModel, y: &ColumnModel| {
        x.data_type == y.data_type
            && x.nullable == y.nullable
            && x.default == y.default
            && x.identity == y.identity
            && x.generated == y.generated
            && x.comment == y.comment
    };
    if same(b, c) {
        return;
    }
    match d {
        Dialect::Mysql => {
            // MODIFY says the whole column again.
            m.push(format!(
                "ALTER TABLE {table} MODIFY COLUMN {}",
                column_sql(c, d)
            ));
            if b.data_type != c.data_type {
                m.note(format!(
                    "changing {}'s type converts its values; it fails (or truncates) if they \
                     don't fit",
                    c.name
                ));
            }
        }
        Dialect::Postgres => {
            if b.generated != c.generated {
                m.push(format!("ALTER TABLE {table} DROP COLUMN {col}"));
                m.push(format!(
                    "ALTER TABLE {table} ADD COLUMN {}",
                    column_sql(c, d)
                ));
                m.note(format!(
                    "{} is dropped and added again to change how it's computed",
                    c.name
                ));
                return;
            }
            if b.data_type != c.data_type {
                m.push(format!(
                    "ALTER TABLE {table} ALTER COLUMN {col} TYPE {} USING {col}::{}",
                    c.data_type, c.data_type
                ));
                m.note(format!(
                    "changing {}'s type converts its values; it fails if they don't convert",
                    c.name
                ));
            }
            if b.identity != c.identity {
                if b.identity.is_some() {
                    m.push(format!(
                        "ALTER TABLE {table} ALTER COLUMN {col} DROP IDENTITY"
                    ));
                }
                if let Some(i) = &c.identity {
                    m.push(format!("ALTER TABLE {table} ALTER COLUMN {col} ADD {i}"));
                }
            }
            if b.default != c.default {
                m.push(match &c.default {
                    Some(v) => format!("ALTER TABLE {table} ALTER COLUMN {col} SET DEFAULT {v}"),
                    None => format!("ALTER TABLE {table} ALTER COLUMN {col} DROP DEFAULT"),
                });
            }
            if b.nullable != c.nullable {
                m.push(format!(
                    "ALTER TABLE {table} ALTER COLUMN {col} {} NOT NULL",
                    if c.nullable { "DROP" } else { "SET" }
                ));
            }
            if b.comment != c.comment {
                m.push(format!(
                    "COMMENT ON COLUMN {table}.{col} IS {}",
                    literal(c.comment.as_deref(), d)
                ));
            }
        }
        Dialect::SqlServer => {
            if b.generated != c.generated {
                m.push(format!("ALTER TABLE {table} DROP COLUMN {col}"));
                m.push(format!("ALTER TABLE {table} ADD {}", column_sql(c, d)));
                m.note(format!(
                    "{} is dropped and added again to change how it's computed",
                    c.name
                ));
                return;
            }
            if b.identity != c.identity {
                m.note(format!(
                    "SQL Server can't add or remove IDENTITY on {}: recreate the column for that",
                    c.name
                ));
            }
            if b.data_type != c.data_type || b.nullable != c.nullable {
                m.push(format!(
                    "ALTER TABLE {table} ALTER COLUMN {col} {} {}NULL",
                    c.data_type,
                    if c.nullable { "" } else { "NOT " }
                ));
                if b.data_type != c.data_type {
                    m.note(format!(
                        "changing {}'s type converts its values; it fails if they don't convert",
                        c.name
                    ));
                }
            }
            if b.default != c.default {
                if b.default.is_some() {
                    m.push(drop_default(table, &c.name));
                }
                if let Some(v) = &c.default {
                    let bare = table.rsplit('.').next().unwrap_or(table);
                    let constraint = format!("DF_{}_{}", bare.trim_matches(['[', ']']), c.name);
                    m.push(format!(
                        "ALTER TABLE {table} ADD CONSTRAINT {} DEFAULT {v} FOR {col}",
                        quote_ident(&constraint, d)
                    ));
                }
            }
            if b.comment != c.comment {
                m.note(format!(
                    "column descriptions (like {}'s) are changed with sp_updateextendedproperty",
                    c.name
                ));
            }
        }
        Dialect::Sqlite => unreachable!("SQLite changes columns by rebuilding the table"),
    }
}

/// SQL Server keeps a column's default as a constraint with a generated name: find it
/// and drop it.
fn drop_default(table: &str, column: &str) -> String {
    let d = Dialect::SqlServer;
    format!(
        "DECLARE @df sysname = (SELECT dc.name FROM sys.default_constraints dc \
         JOIN sys.columns c ON c.object_id = dc.parent_object_id \
         AND c.column_id = dc.parent_column_id \
         WHERE dc.parent_object_id = OBJECT_ID({}) AND c.name = {}); \
         IF @df IS NOT NULL EXEC (N'ALTER TABLE {} DROP CONSTRAINT ' + QUOTENAME(@df))",
        literal(Some(table), d),
        literal(Some(column), d),
        table.replace('\'', "''")
    )
}

/// Whether SQLite needs the table rebuilt: it can only rename, add and drop columns
/// (and add or drop indexes and triggers) in place.
fn needs_rebuild(old: &TableModel, new: &TableModel, renamed: &[(String, String)]) -> bool {
    let was = |c: &str| {
        renamed
            .iter()
            .find(|(_, n)| n == c)
            .map(|(o, _)| o.as_str())
            .unwrap_or(c)
            .to_owned()
    };
    let changed_column = new.columns.iter().any(|c| {
        old.column(&was(&c.name)).is_some_and(|b| {
            b.data_type != c.data_type
                || b.nullable != c.nullable
                || b.default != c.default
                || b.generated != c.generated
        })
    });
    // Added columns with constraints of their own, or NOT NULL without a default,
    // can't be added in place either.
    let hard_add = new
        .columns
        .iter()
        .any(|c| old.column(&was(&c.name)).is_none() && !c.nullable && c.default.is_none());
    let constraints_changed = old.constraints.iter().any(|c| {
        !new.constraints
            .iter()
            .any(|n| same_constraint(c, n, renamed))
    }) || new.constraints.iter().any(|c| {
        !old.constraints
            .iter()
            .any(|o| same_constraint(o, c, renamed))
    });
    changed_column || hard_add || constraints_changed
}

/// SQLite's way to change a table: create the new one, copy the rows, drop the old,
/// rename the new; then its indexes and triggers again.
fn rebuild_sqlite(
    m: &mut Migration,
    schema: &str,
    old: &TableModel,
    new: &TableModel,
    renamed: &[(String, String)],
) {
    let d = Dialect::Sqlite;
    let tmp_name = format!("{}__datadrop_new", new.name);
    let mut tmp = new.clone();
    tmp.name = tmp_name.clone();
    tmp.indexes.clear();
    tmp.triggers.clear();
    tmp.definition = None;
    m.push(
        create_table(schema, &tmp, d)
            .trim_end()
            .trim_end_matches(';')
            .to_owned(),
    );
    let mut to = vec![];
    let mut from = vec![];
    for c in &new.columns {
        let o = renamed
            .iter()
            .find(|(_, n)| n == &c.name)
            .map(|(o, _)| o.clone())
            .unwrap_or_else(|| c.name.clone());
        if old.column(&o).is_some() && c.generated.is_none() {
            to.push(c.name.clone());
            from.push(o);
        }
    }
    if !to.is_empty() {
        m.push(format!(
            "INSERT INTO {} ({}) SELECT {} FROM {}",
            qualified(schema, &tmp_name, d),
            column_list(&to, d),
            column_list(&from, d),
            qualified(schema, &new.name, d)
        ));
    }
    m.push(format!("DROP TABLE {}", qualified(schema, &new.name, d)));
    m.push(format!(
        "ALTER TABLE {} RENAME TO {}",
        qualified(schema, &tmp_name, d),
        quote_ident(&new.name, d)
    ));
    for i in &new.indexes {
        m.push(if i.definition.is_empty() {
            index_definition(schema, &new.name, i, d)
        } else {
            i.definition.clone()
        });
    }
    for t in &new.triggers {
        m.push(t.definition.clone());
    }
    m.note(format!(
        "SQLite can't change {}'s columns or constraints in place, so the table is \
         recreated and its rows copied",
        new.name
    ));
}

fn same_constraint(old: &Constraint, new: &Constraint, renamed: &[(String, String)]) -> bool {
    if old.name != new.name || old.kind != new.kind {
        return false;
    }
    if old.definition == new.definition {
        return true;
    }
    // Columns renamed: the old definition's names don't match, so compare the parts.
    let cols: Vec<String> = old
        .columns
        .iter()
        .map(|c| {
            renamed
                .iter()
                .find(|(o, _)| o == c)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| c.clone())
        })
        .collect();
    old.kind != ConstraintKind::Check
        && cols == new.columns
        && old
            .references
            .as_ref()
            .map(|r| (&r.table, &r.columns, &r.on_delete, &r.on_update))
            == new
                .references
                .as_ref()
                .map(|r| (&r.table, &r.columns, &r.on_delete, &r.on_update))
}

fn same_index(old: &Index, new: &Index) -> bool {
    old.name == new.name
        && old.unique == new.unique
        && old.columns == new.columns
        && (new.definition.is_empty() || old.definition == new.definition)
}

fn drop_constraint(table: &str, c: &Constraint, d: Dialect) -> String {
    let name = quote_ident(&c.name, d);
    match (d, c.kind) {
        (Dialect::Mysql, ConstraintKind::PrimaryKey) => {
            format!("ALTER TABLE {table} DROP PRIMARY KEY")
        }
        (Dialect::Mysql, ConstraintKind::ForeignKey) => {
            format!("ALTER TABLE {table} DROP FOREIGN KEY {name}")
        }
        (Dialect::Mysql, ConstraintKind::Check) => format!("ALTER TABLE {table} DROP CHECK {name}"),
        (Dialect::Mysql, ConstraintKind::Unique) => {
            format!("ALTER TABLE {table} DROP INDEX {name}")
        }
        _ => format!("ALTER TABLE {table} DROP CONSTRAINT {name}"),
    }
}

fn add_constraint(table: &str, c: &Constraint, d: Dialect) -> String {
    if d == Dialect::Mysql && c.kind == ConstraintKind::PrimaryKey {
        return format!("ALTER TABLE {table} ADD {}", c.definition);
    }
    format!(
        "ALTER TABLE {table} ADD CONSTRAINT {} {}",
        quote_ident(&c.name, d),
        c.definition
    )
}

fn drop_index(schema: &str, table: &str, index: &str, d: Dialect) -> String {
    match d {
        Dialect::Mysql | Dialect::SqlServer => {
            format!("DROP INDEX {} ON {table}", quote_ident(index, d))
        }
        _ => format!("DROP INDEX {}", qualified(schema, index, d)),
    }
}

/// The statements that make `target` (a schema on one server) like `source` (one on
/// another, or the same): tables, columns, constraints, indexes, triggers, views,
/// routines and sequences that differ are created, changed or dropped.
pub fn schema_diff(source: &SchemaModel, target: &SchemaModel, d: Dialect) -> Migration {
    let mut m = Migration::default();
    let schema = &target.schema;
    // Definitions name their schema; the source's name becomes the target's.
    let rename = |sql: &str| -> String {
        if source.schema == target.schema {
            return sql.to_owned();
        }
        sql.replace(
            &format!("{}.", quote_ident(&source.schema, d)),
            &format!("{}.", quote_ident(schema, d)),
        )
        .replace(&format!("{}.", source.schema), &format!("{schema}."))
    };
    let moved = |t: &TableModel| -> TableModel {
        let mut t = t.clone();
        for c in &mut t.constraints {
            c.definition = rename(&c.definition);
            if let Some(r) = &mut c.references
                && r.schema == source.schema
            {
                r.schema = schema.clone();
            }
        }
        for i in &mut t.indexes {
            i.definition = rename(&i.definition);
        }
        for tr in &mut t.triggers {
            tr.definition = rename(&tr.definition);
        }
        t.definition = t.definition.as_deref().map(rename);
        t
    };

    for s in &source.sequences {
        if !target.sequences.iter().any(|t| t.name == s.name) {
            m.push(rename(&s.definition));
        }
    }
    // Views come out first and go back last: they may use what changes.
    let views_changed: Vec<&TableModel> = source
        .tables
        .iter()
        .filter(|v| v.is_view())
        .filter(|v| {
            target.table(&v.name).is_none_or(|t| {
                t.definition.as_deref().map(rename) != v.definition.as_deref().map(rename)
                    || !t.is_view()
            })
        })
        .collect();
    for t in target.tables.iter().filter(|t| t.is_view()) {
        let gone = source.table(&t.name).is_none_or(|s| !s.is_view());
        let changed = views_changed.iter().any(|v| v.name == t.name);
        if gone || changed {
            m.push(format!("DROP VIEW {}", qualified(schema, &t.name, d)));
        }
    }
    let (order, _) = table_order(source);
    for s in &order {
        let s = moved(s);
        match target.table(&s.name).filter(|t| !t.is_view()) {
            None => {
                for st in split_ddl(&table_ddl(schema, &s, d)) {
                    m.push(st);
                }
            }
            Some(t) => {
                let mut tm = Migration::default();
                alter_table(&mut tm, schema, t, &s, &[], d);
                m.statements.extend(tm.statements);
                for n in tm.notes {
                    m.note(n);
                }
            }
        }
    }
    let (target_order, _) = table_order(target);
    for t in target_order.iter().rev() {
        if source.table(&t.name).is_none_or(|s| s.is_view()) {
            m.push(format!("DROP TABLE {}", qualified(schema, &t.name, d)));
            m.note(format!("{} is dropped, with its rows", t.name));
        }
    }
    for r in &target.routines {
        let keep = source
            .routines
            .iter()
            .any(|s| s.name == r.name && s.arguments == r.arguments && s.returns == r.returns);
        let replaced_in_place = d == Dialect::Postgres
            && source
                .routines
                .iter()
                .any(|s| s.name == r.name && s.arguments == r.arguments && s.returns == r.returns);
        if !keep || (d == Dialect::Mysql && changed_routine(source, r, &rename)) {
            let what = match r.kind {
                RoutineKind::Function => "FUNCTION",
                RoutineKind::Procedure => "PROCEDURE",
            };
            let args = if d == Dialect::Postgres {
                format!("({})", r.arguments)
            } else {
                String::new()
            };
            if !replaced_in_place {
                m.push(format!(
                    "DROP {what} {}{args}",
                    qualified(schema, &r.name, d)
                ));
            }
        }
    }
    for s in &source.routines {
        let same = target.routines.iter().any(|t| {
            t.name == s.name
                && t.arguments == s.arguments
                && t.definition.as_deref().map(rename) == s.definition.as_deref().map(rename)
        });
        if !same && let Some(def) = &s.definition {
            m.push(rename(def));
        }
    }
    for v in views_changed {
        if let Some(def) = &v.definition {
            m.push(rename(def));
        }
    }
    for s in &target.sequences {
        if !source.sequences.iter().any(|x| x.name == s.name) {
            m.push(format!("DROP SEQUENCE {}", qualified(schema, &s.name, d)));
        }
    }
    m
}

fn changed_routine(
    source: &SchemaModel,
    r: &crate::schema::Routine,
    rename: &impl Fn(&str) -> String,
) -> bool {
    source
        .routines
        .iter()
        .find(|s| s.name == r.name)
        .is_some_and(|s| s.definition.as_deref().map(rename) != r.definition.as_deref().map(rename))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::TableKind;

    fn col(name: &str, ty: &str, nullable: bool) -> ColumnModel {
        ColumnModel {
            name: name.into(),
            data_type: ty.into(),
            nullable,
            ..ColumnModel::default()
        }
    }

    fn table(name: &str, cols: Vec<ColumnModel>) -> TableModel {
        TableModel {
            name: name.into(),
            kind: TableKind::Table,
            comment: None,
            columns: cols,
            constraints: vec![Constraint {
                name: format!("{name}_pkey"),
                kind: ConstraintKind::PrimaryKey,
                columns: vec!["id".into()],
                definition: "PRIMARY KEY (id)".into(),
                references: None,
            }],
            indexes: vec![],
            triggers: vec![],
            definition: None,
        }
    }

    #[test]
    fn postgres_alters_in_place() {
        let old = table(
            "pets",
            vec![
                col("id", "integer", false),
                col("name", "text", true),
                col("age", "integer", true),
            ],
        );
        let mut new = old.clone();
        new.name = "animals".into();
        new.columns[1].name = "label".into();
        new.columns[1].nullable = false;
        new.columns[2].data_type = "bigint".into();
        new.columns[2].default = Some("0".into());
        new.columns.push(col("born", "date", true));
        new.indexes.push(Index {
            name: "animals_label".into(),
            columns: vec!["label".into()],
            unique: false,
            definition: String::new(),
            inline: None,
        });
        let m = table_edit(
            &TableEdit {
                schema: "s".into(),
                old: Some(old),
                new,
                renamed_columns: vec![("name".into(), "label".into())],
            },
            Dialect::Postgres,
        );
        assert_eq!(
            m.statements,
            [
                "ALTER TABLE \"s\".\"pets\" RENAME TO \"animals\"",
                "ALTER TABLE \"s\".\"animals\" RENAME COLUMN \"name\" TO \"label\"",
                "ALTER TABLE \"s\".\"animals\" ALTER COLUMN \"label\" SET NOT NULL",
                "ALTER TABLE \"s\".\"animals\" ALTER COLUMN \"age\" TYPE bigint USING \"age\"::bigint",
                "ALTER TABLE \"s\".\"animals\" ALTER COLUMN \"age\" SET DEFAULT 0",
                "ALTER TABLE \"s\".\"animals\" ADD COLUMN \"born\" date",
                "CREATE INDEX \"animals_label\" ON \"s\".\"animals\" (\"label\")",
            ]
        );
        assert_eq!(m.notes.len(), 1, "{:?}", m.notes);
    }

    #[test]
    fn mysql_modifies_whole_columns() {
        let old = table("t", vec![col("id", "int", false), col("a", "int", true)]);
        let mut new = old.clone();
        new.columns[1].data_type = "bigint".into();
        new.columns.insert(1, col("b", "varchar(10)", true));
        let m = table_edit(
            &TableEdit {
                schema: "db".into(),
                old: Some(old),
                new,
                renamed_columns: vec![],
            },
            Dialect::Mysql,
        );
        assert_eq!(
            m.statements,
            [
                "ALTER TABLE `db`.`t` ADD COLUMN `b` varchar(10) AFTER `id`",
                "ALTER TABLE `db`.`t` MODIFY COLUMN `a` bigint",
            ]
        );
    }

    #[test]
    fn sqlite_rebuilds_when_it_must() {
        let old = table(
            "t",
            vec![col("id", "INTEGER", false), col("a", "TEXT", true)],
        );
        // Adding a nullable column: in place.
        let mut add = old.clone();
        add.columns.push(col("b", "TEXT", true));
        let m = table_edit(
            &TableEdit {
                schema: "main".into(),
                old: Some(old.clone()),
                new: add,
                renamed_columns: vec![],
            },
            Dialect::Sqlite,
        );
        assert_eq!(m.statements, ["ALTER TABLE \"t\" ADD COLUMN \"b\" TEXT"]);
        // Making a column NOT NULL: rebuilt.
        let mut strict = old.clone();
        strict.columns[1].nullable = false;
        let m = table_edit(
            &TableEdit {
                schema: "main".into(),
                old: Some(old),
                new: strict,
                renamed_columns: vec![],
            },
            Dialect::Sqlite,
        );
        assert_eq!(m.statements.len(), 4, "{:#?}", m.statements);
        assert!(m.statements[0].starts_with("CREATE TABLE \"t__datadrop_new\""));
        assert_eq!(
            m.statements[1],
            "INSERT INTO \"t__datadrop_new\" (\"id\", \"a\") SELECT \"id\", \"a\" FROM \"t\""
        );
        assert_eq!(
            m.statements[3],
            "ALTER TABLE \"t__datadrop_new\" RENAME TO \"t\""
        );
    }

    #[test]
    fn new_tables_are_created() {
        let new = table("t", vec![col("id", "integer", false)]);
        let m = table_edit(
            &TableEdit {
                schema: "public".into(),
                old: None,
                new,
                renamed_columns: vec![],
            },
            Dialect::Postgres,
        );
        assert_eq!(
            m.statements,
            [
                "CREATE TABLE \"public\".\"t\" (\n    \"id\" integer NOT NULL,\n    CONSTRAINT \"t_pkey\" PRIMARY KEY (id)\n)"
            ]
        );
    }
}
