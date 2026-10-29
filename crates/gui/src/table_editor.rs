//! Creating or changing a table from a form: columns, the primary key, indexes,
//! foreign keys and checks, with the SQL it takes shown before it runs.

use datadrop_core::diff::{self, Migration, TableEdit};
use datadrop_core::schema::{
    ColumnModel, Constraint, ConstraintKind, Index, Reference, TableModel, column_list,
    fk_definition,
};
use datadrop_core::sql::Dialect;
use datadrop_core::{Command, Output, TableKind};
use egui::RichText;
use egui_phosphor::regular as icon;

use crate::backend::{Backend, Task, poll};
use crate::kit::{self, ModalExt};
use crate::theme;

#[derive(Clone, Default)]
struct ColumnRow {
    /// Its name before editing (`None` for a new column).
    original: Option<ColumnModel>,
    name: String,
    data_type: String,
    nullable: bool,
    default: String,
    primary_key: bool,
}

#[derive(Clone, Default)]
struct IndexRow {
    original: Option<Index>,
    name: String,
    /// Comma-separated.
    columns: String,
    unique: bool,
}

#[derive(Clone, Default)]
struct ForeignKeyRow {
    original: Option<Constraint>,
    name: String,
    columns: String,
    table: String,
    ref_columns: String,
    on_delete: String,
}

#[derive(Clone, Default)]
struct CheckRow {
    original: Option<Constraint>,
    name: String,
    /// The whole `CHECK (...)`.
    definition: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Columns,
    Indexes,
    ForeignKeys,
    Checks,
}

pub struct TableEditor {
    pub profile: String,
    pub schema: String,
    dialect: Dialect,
    old: Option<TableModel>,
    name: String,
    comment: String,
    columns: Vec<ColumnRow>,
    indexes: Vec<IndexRow>,
    fks: Vec<ForeignKeyRow>,
    checks: Vec<CheckRow>,
    section: Section,
    applying: Option<Task>,
    error: Option<String>,
    /// Focus the name on the first frame.
    focus: bool,
}

pub enum EditorDone {
    Applied,
    Cancelled,
}

const RULES: [&str; 5] = [
    "NO ACTION",
    "RESTRICT",
    "CASCADE",
    "SET NULL",
    "SET DEFAULT",
];

/// A text box of a set width (in a grid cell, a plain one shrinks to the column).
fn text(ui: &mut egui::Ui, value: &mut String, width: f32, hint: &str) -> egui::Response {
    ui.add_sized(
        [width, ui.spacing().interact_size.y],
        egui::TextEdit::singleline(value).hint_text(hint),
    )
}

fn split_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|c| c.trim().to_owned())
        .filter(|c| !c.is_empty())
        .collect()
}

fn common_types(d: Dialect) -> &'static [&'static str] {
    match d {
        Dialect::Postgres => &[
            "integer",
            "bigint",
            "smallint",
            "numeric(10,2)",
            "real",
            "double precision",
            "boolean",
            "text",
            "varchar(255)",
            "date",
            "timestamp with time zone",
            "uuid",
            "jsonb",
            "bytea",
        ],
        Dialect::Mysql => &[
            "int",
            "bigint",
            "smallint",
            "decimal(10,2)",
            "double",
            "tinyint(1)",
            "varchar(255)",
            "text",
            "date",
            "datetime",
            "timestamp",
            "json",
            "blob",
        ],
        Dialect::Sqlite => &["INTEGER", "REAL", "TEXT", "BLOB", "NUMERIC"],
        Dialect::SqlServer => &[
            "int",
            "bigint",
            "smallint",
            "decimal(10,2)",
            "float",
            "bit",
            "nvarchar(255)",
            "nvarchar(max)",
            "varchar(255)",
            "date",
            "datetime2",
            "datetimeoffset",
            "uniqueidentifier",
            "varbinary(max)",
        ],
        Dialect::ClickHouse => &[
            "UInt64",
            "Int64",
            "Int32",
            "Float64",
            "Decimal(10, 2)",
            "Bool",
            "String",
            "Nullable(String)",
            "LowCardinality(String)",
            "Date",
            "DateTime",
            "DateTime64(3)",
            "UUID",
            "Array(String)",
        ],
    }
}

impl TableEditor {
    /// Editing `table`, or (with `None`) making a new one.
    pub fn new(
        profile: String,
        schema: String,
        dialect: Dialect,
        table: Option<TableModel>,
    ) -> Self {
        let mut e = Self {
            profile,
            schema,
            dialect,
            old: table.clone(),
            name: String::new(),
            comment: String::new(),
            columns: vec![],
            indexes: vec![],
            fks: vec![],
            checks: vec![],
            section: Section::Columns,
            applying: None,
            error: None,
            focus: true,
        };
        match table {
            None => {
                e.columns.push(ColumnRow {
                    name: "id".into(),
                    data_type: match dialect {
                        Dialect::Postgres => "bigint",
                        Dialect::Mysql => "bigint",
                        Dialect::Sqlite => "INTEGER",
                        Dialect::SqlServer => "bigint",
                        Dialect::ClickHouse => "UInt64",
                    }
                    .into(),
                    primary_key: true,
                    ..ColumnRow::default()
                });
            }
            Some(t) => {
                let pk: Vec<String> = t
                    .primary_key()
                    .map(|k| k.columns.clone())
                    .unwrap_or_default();
                e.name = t.name.clone();
                e.comment = t.comment.clone().unwrap_or_default();
                e.columns = t
                    .columns
                    .iter()
                    .map(|c| ColumnRow {
                        original: Some(c.clone()),
                        name: c.name.clone(),
                        data_type: c.data_type.clone(),
                        nullable: c.nullable,
                        default: c.default.clone().unwrap_or_default(),
                        primary_key: pk.contains(&c.name),
                    })
                    .collect();
                e.indexes = t
                    .indexes
                    .iter()
                    .map(|i| IndexRow {
                        original: Some(i.clone()),
                        name: i.name.clone(),
                        columns: i.columns.join(", "),
                        unique: i.unique,
                    })
                    .collect();
                for c in &t.constraints {
                    match c.kind {
                        ConstraintKind::ForeignKey => e.fks.push(ForeignKeyRow {
                            original: Some(c.clone()),
                            name: c.name.clone(),
                            columns: c.columns.join(", "),
                            table: c
                                .references
                                .as_ref()
                                .map(|r| r.table.clone())
                                .unwrap_or_default(),
                            ref_columns: c
                                .references
                                .as_ref()
                                .map(|r| r.columns.join(", "))
                                .unwrap_or_default(),
                            on_delete: c
                                .references
                                .as_ref()
                                .map(|r| r.on_delete.clone())
                                .unwrap_or_default(),
                        }),
                        ConstraintKind::Check => e.checks.push(CheckRow {
                            original: Some(c.clone()),
                            name: c.name.clone(),
                            definition: c.definition.clone(),
                        }),
                        _ => {}
                    }
                }
            }
        }
        e
    }

    /// Add a column, as the form does (for tests).
    #[cfg(test)]
    pub fn add_column(&mut self, name: &str, data_type: &str, default: &str) {
        self.columns.push(ColumnRow {
            name: name.into(),
            data_type: data_type.into(),
            nullable: true,
            default: default.into(),
            ..ColumnRow::default()
        });
    }

    pub fn is_new(&self) -> bool {
        self.old.is_none()
    }

    /// The table as the form has it, and the columns renamed.
    fn edit(&self) -> Result<TableEdit, String> {
        let d = self.dialect;
        let name = self.name.trim().to_owned();
        if name.is_empty() {
            return Err("Give the table a name.".into());
        }
        let mut columns = vec![];
        let mut renamed = vec![];
        for r in &self.columns {
            let n = r.name.trim().to_owned();
            if n.is_empty() {
                return Err("Every column needs a name.".into());
            }
            if r.data_type.trim().is_empty() {
                return Err(format!("Give {n} a type."));
            }
            if columns.iter().any(|c: &ColumnModel| c.name == n) {
                return Err(format!("There are two columns called {n}."));
            }
            let mut c = r.original.clone().unwrap_or_default();
            if let Some(o) = &r.original
                && o.name != n
            {
                renamed.push((o.name.clone(), n.clone()));
            }
            // A column joining the key can't be NULL; one already in it stays as the
            // database describes it (SQLite reports its INTEGER PRIMARY KEY nullable).
            let was_key = r.original.as_ref().is_some_and(|o| {
                self.old
                    .as_ref()
                    .and_then(|t| t.primary_key())
                    .is_some_and(|k| k.columns.contains(&o.name))
            });
            c.name = n;
            c.data_type = r.data_type.trim().to_owned();
            c.nullable = r.nullable && (was_key || !r.primary_key);
            c.default = Some(r.default.trim().to_owned()).filter(|s| !s.is_empty());
            columns.push(c);
        }
        if columns.is_empty() {
            return Err("A table needs at least one column.".into());
        }
        let renamed_to = |c: &str| -> String {
            renamed
                .iter()
                .find(|(o, _)| o == c)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| c.to_owned())
        };
        let mut constraints = vec![];
        let pk: Vec<String> = self
            .columns
            .iter()
            .filter(|r| r.primary_key)
            .map(|r| r.name.trim().to_owned())
            .collect();
        let old_pk = self.old.as_ref().and_then(|t| t.primary_key());
        if !pk.is_empty() {
            constraints.push(match old_pk {
                Some(k) if k.columns.iter().map(|c| renamed_to(c)).collect::<Vec<_>>() == pk => {
                    k.clone()
                }
                _ => Constraint {
                    name: old_pk.map(|k| k.name.clone()).unwrap_or_else(|| match d {
                        Dialect::Mysql => "PRIMARY".into(),
                        Dialect::SqlServer => format!("PK_{name}"),
                        _ => format!("{name}_pkey"),
                    }),
                    kind: ConstraintKind::PrimaryKey,
                    definition: format!("PRIMARY KEY ({})", column_list(&pk, d)),
                    columns: pk,
                    references: None,
                },
            });
        }
        // Unique constraints aren't edited here: they stay as they are.
        if let Some(t) = &self.old {
            constraints.extend(
                t.constraints
                    .iter()
                    .filter(|c| c.kind == ConstraintKind::Unique)
                    .cloned(),
            );
        }
        for r in &self.checks {
            let def = r.definition.trim();
            if def.is_empty() {
                continue;
            }
            constraints.push(match &r.original {
                Some(o) if o.name == r.name && o.definition == def => o.clone(),
                _ => Constraint {
                    name: r.name.trim().to_owned(),
                    kind: ConstraintKind::Check,
                    columns: vec![],
                    definition: def.to_owned(),
                    references: None,
                },
            });
        }
        for r in &self.fks {
            let columns = split_list(&r.columns);
            let ref_columns = split_list(&r.ref_columns);
            if columns.is_empty() || r.table.trim().is_empty() {
                return Err(format!(
                    "Foreign key {} needs its columns and the table it refers to.",
                    r.name
                ));
            }
            let reference = Reference {
                schema: self.schema.clone(),
                table: r.table.trim().to_owned(),
                columns: ref_columns,
                on_update: "NO ACTION".into(),
                on_delete: if r.on_delete.is_empty() {
                    "NO ACTION".into()
                } else {
                    r.on_delete.clone()
                },
            };
            let unchanged = r.original.as_ref().is_some_and(|o| {
                o.name == r.name
                    && o.columns.iter().map(|c| renamed_to(c)).collect::<Vec<_>>() == columns
                    && o.references.as_ref().is_some_and(|x| {
                        x.table == reference.table
                            && x.columns == reference.columns
                            && x.on_delete == reference.on_delete
                    })
            });
            if unchanged && let Some(o) = &r.original {
                let mut o = o.clone();
                o.columns = columns;
                constraints.push(o);
                continue;
            }
            let mut c = Constraint {
                name: r.name.trim().to_owned(),
                kind: ConstraintKind::ForeignKey,
                columns,
                definition: String::new(),
                references: Some(reference),
            };
            c.definition = fk_definition(&c, &self.schema, d);
            constraints.push(c);
        }
        let mut indexes = vec![];
        for r in &self.indexes {
            let cols = split_list(&r.columns);
            if r.name.trim().is_empty() || cols.is_empty() {
                return Err("Every index needs a name and its columns.".into());
            }
            indexes.push(match &r.original {
                Some(o)
                    if o.name == r.name
                        && o.unique == r.unique
                        && o.columns.iter().map(|c| renamed_to(c)).collect::<Vec<_>>() == cols =>
                {
                    let mut o = o.clone();
                    o.columns = cols;
                    o
                }
                _ => Index {
                    name: r.name.trim().to_owned(),
                    columns: cols,
                    unique: r.unique,
                    definition: String::new(),
                    inline: None,
                },
            });
        }
        let new = TableModel {
            name,
            kind: TableKind::Table,
            comment: Some(self.comment.trim().to_owned()).filter(|s| !s.is_empty()),
            columns,
            constraints,
            indexes,
            triggers: self
                .old
                .as_ref()
                .map(|t| t.triggers.clone())
                .unwrap_or_default(),
            definition: None,
        };
        Ok(TableEdit {
            schema: self.schema.clone(),
            old: self.old.clone().map(|mut t| {
                // SQLite's own CREATE TABLE text isn't used to change it.
                t.definition = None;
                t
            }),
            new,
            renamed_columns: renamed,
        })
    }

    pub fn migration(&self) -> Result<Migration, String> {
        Ok(diff::table_edit(&self.edit()?, self.dialect))
    }

    pub fn ui(&mut self, ctx: &egui::Context, backend: &Backend) -> Option<EditorDone> {
        let mut done = None;
        if let Some(r) = poll(&mut self.applying) {
            match r {
                Ok(Output::Migration { .. }) => done = Some(EditorDone::Applied),
                Ok(_) => {}
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        let migration = self.migration();
        egui::Modal::new(egui::Id::new("table-editor"))
            .styled(ctx)
            .show(ctx, |ui| {
                ui.set_width(860.0);
                let p = theme::colors(ui);
                let title = match &self.old {
                    None => "New table".to_owned(),
                    Some(t) => format!("Edit {}", t.name),
                };
                kit::sheet_header(
                    ui,
                    &title,
                    Some(&format!("in {} on {}", self.schema, self.profile)),
                );
                egui::Grid::new("table-head")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        let l = ui.label("Name");
                        let name = text(ui, &mut self.name, 300.0, "").labelled_by(l.id);
                        if std::mem::take(&mut self.focus) && self.old.is_none() {
                            name.request_focus();
                        }
                        ui.end_row();
                        if self.dialect != Dialect::Sqlite {
                            let l = ui.label("Comment");
                            text(ui, &mut self.comment, 500.0, "optional").labelled_by(l.id);
                            ui.end_row();
                        }
                    });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 18.0;
                    for (s, label) in [
                        (
                            Section::Columns,
                            format!("Columns ({})", self.columns.len()),
                        ),
                        (
                            Section::Indexes,
                            format!("Indexes ({})", self.indexes.len()),
                        ),
                        (
                            Section::ForeignKeys,
                            format!("Foreign keys ({})", self.fks.len()),
                        ),
                        (Section::Checks, format!("Checks ({})", self.checks.len())),
                    ] {
                        let on = self.section == s;
                        let resp = ui.add(
                            egui::Button::selectable(
                                on,
                                RichText::new(label)
                                    .font(egui::FontId::new(13.0, theme::medium()))
                                    .color(if on { p.text } else { p.muted }),
                            )
                            .frame(false),
                        );
                        if resp.clicked() {
                            self.section = s;
                        }
                        if on {
                            ui.painter().hline(
                                resp.rect.x_range(),
                                resp.rect.bottom() + 5.0,
                                egui::Stroke::new(2.0, p.accent),
                            );
                        }
                    }
                });
                ui.add_space(4.0);
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("table-editor-body")
                    .max_height(260.0)
                    .show(ui, |ui| match self.section {
                        Section::Columns => self.columns_ui(ui),
                        Section::Indexes => self.indexes_ui(ui),
                        Section::ForeignKeys => self.fks_ui(ui),
                        Section::Checks => self.checks_ui(ui),
                    });
                ui.add_space(8.0);
                kit::caps(ui, "Pending changes");
                ui.add_space(2.0);
                match &migration {
                    Err(e) => {
                        ui.label(RichText::new(e).color(ui.visuals().warn_fg_color));
                    }
                    Ok(m) if m.statements.is_empty() => {
                        ui.weak("No changes yet.");
                    }
                    Ok(m) => {
                        kit::card(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("table-editor-sql")
                                .max_height(160.0)
                                .show(ui, |ui| {
                                    let mut script = m.script();
                                    ui.add(
                                        egui::TextEdit::multiline(&mut script)
                                            .font(egui::TextStyle::Monospace)
                                            .frame(egui::Frame::NONE)
                                            .desired_width(f32::INFINITY)
                                            .interactive(false),
                                    );
                                });
                        });
                    }
                }
                if let Some(e) = &self.error {
                    ui.label(RichText::new(e).color(ui.visuals().error_fg_color));
                }
                kit::sheet_footer(ui, |ui| {
                    let ready = migration.as_ref().is_ok_and(|m| !m.statements.is_empty());
                    let busy = self.applying.is_some();
                    ui.add_enabled_ui(ready && !busy, |ui| {
                        let label = if self.old.is_none() {
                            "Create"
                        } else {
                            "Apply"
                        };
                        if ui.add(kit::primary(ui, label)).clicked()
                            && let Ok(edit) = self.edit()
                        {
                            self.error = None;
                            self.applying = Some(backend.run(Command::AlterTable {
                                profile: self.profile.clone(),
                                edit,
                                apply: true,
                            }));
                        }
                    });
                    if ui.add(kit::ghost(ui, "Cancel")).clicked() {
                        done = Some(EditorDone::Cancelled);
                    }
                    if busy {
                        ui.spinner();
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        if ui.button(format!("{} Copy SQL", icon::COPY)).clicked()
                            && let Ok(m) = &migration
                        {
                            ui.ctx().copy_text(m.script());
                        }
                    });
                });
            });
        done
    }

    fn columns_ui(&mut self, ui: &mut egui::Ui) {
        let types = common_types(self.dialect);
        let mut remove = None;
        let mut swap = None;
        egui::Grid::new("table-columns")
            .num_columns(7)
            .striped(true)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                ui.strong("Name");
                ui.strong("Type");
                ui.strong("Key").on_hover_text("In the primary key");
                ui.strong("NULL").on_hover_text("Can be NULL");
                ui.strong("Default");
                ui.label("");
                ui.label("");
                ui.end_row();
                let n = self.columns.len();
                for (i, c) in self.columns.iter_mut().enumerate() {
                    text(ui, &mut c.name, 150.0, "");
                    ui.horizontal(|ui| {
                        text(ui, &mut c.data_type, 150.0, "");
                        ui.menu_button(icon::CARET_DOWN, |ui| {
                            for t in types {
                                if ui.button(*t).clicked() {
                                    c.data_type = (*t).into();
                                    ui.close();
                                }
                            }
                        });
                    });
                    ui.checkbox(&mut c.primary_key, "");
                    ui.add_enabled(
                        !c.primary_key,
                        egui::Checkbox::without_text(&mut c.nullable),
                    );
                    text(ui, &mut c.default, 150.0, "none");
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(i > 0, egui::Button::new(icon::ARROW_UP).small())
                            .clicked()
                        {
                            swap = Some(i - 1);
                        }
                        if ui
                            .add_enabled(i + 1 < n, egui::Button::new(icon::ARROW_DOWN).small())
                            .clicked()
                        {
                            swap = Some(i);
                        }
                    });
                    if ui
                        .small_button(icon::TRASH)
                        .on_hover_text("Remove the column")
                        .clicked()
                    {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = swap {
            self.columns.swap(i, i + 1);
        }
        if let Some(i) = remove {
            self.columns.remove(i);
        }
        if ui.button(format!("{} Column", icon::PLUS)).clicked() {
            self.columns.push(ColumnRow {
                nullable: true,
                ..ColumnRow::default()
            });
        }
    }

    fn indexes_ui(&mut self, ui: &mut egui::Ui) {
        let mut remove = None;
        egui::Grid::new("table-indexes")
            .num_columns(4)
            .striped(true)
            .show(ui, |ui| {
                ui.strong("Name");
                ui.strong("Columns");
                ui.strong("Unique");
                ui.label("");
                ui.end_row();
                for (i, r) in self.indexes.iter_mut().enumerate() {
                    text(ui, &mut r.name, 200.0, "");
                    text(ui, &mut r.columns, 300.0, "a, b");
                    ui.checkbox(&mut r.unique, "");
                    if ui.small_button(icon::TRASH).clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = remove {
            self.indexes.remove(i);
        }
        if ui.button(format!("{} Index", icon::PLUS)).clicked() {
            let n = self.indexes.len() + 1;
            self.indexes.push(IndexRow {
                name: format!("{}_idx{n}", self.name.trim()),
                ..IndexRow::default()
            });
        }
    }

    fn fks_ui(&mut self, ui: &mut egui::Ui) {
        let mut remove = None;
        egui::Grid::new("table-fks")
            .num_columns(6)
            .striped(true)
            .show(ui, |ui| {
                ui.strong("Name");
                ui.strong("Columns");
                ui.strong("Refers to table");
                ui.strong("Its columns");
                ui.strong("On delete");
                ui.label("");
                ui.end_row();
                for (i, r) in self.fks.iter_mut().enumerate() {
                    text(ui, &mut r.name, 140.0, "");
                    text(ui, &mut r.columns, 120.0, "");
                    text(ui, &mut r.table, 120.0, "");
                    text(ui, &mut r.ref_columns, 120.0, "its key");
                    egui::ComboBox::from_id_salt(("fk-delete", i))
                        .selected_text(if r.on_delete.is_empty() {
                            "NO ACTION"
                        } else {
                            r.on_delete.as_str()
                        })
                        .show_ui(ui, |ui| {
                            for rule in RULES {
                                ui.selectable_value(&mut r.on_delete, rule.to_owned(), rule);
                            }
                        });
                    if ui.small_button(icon::TRASH).clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = remove {
            self.fks.remove(i);
        }
        if ui.button(format!("{} Foreign key", icon::PLUS)).clicked() {
            let n = self.fks.len() + 1;
            self.fks.push(ForeignKeyRow {
                name: format!("{}_fk{n}", self.name.trim()),
                ..ForeignKeyRow::default()
            });
        }
    }

    fn checks_ui(&mut self, ui: &mut egui::Ui) {
        let mut remove = None;
        egui::Grid::new("table-checks")
            .num_columns(3)
            .striped(true)
            .show(ui, |ui| {
                ui.strong("Name");
                ui.strong("Check");
                ui.label("");
                ui.end_row();
                for (i, r) in self.checks.iter_mut().enumerate() {
                    text(ui, &mut r.name, 160.0, "");
                    ui.add_sized(
                        [420.0, ui.spacing().interact_size.y],
                        egui::TextEdit::singleline(&mut r.definition)
                            .font(egui::TextStyle::Monospace)
                            .hint_text("CHECK (price >= 0)"),
                    );
                    if ui.small_button(icon::TRASH).clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = remove {
            self.checks.remove(i);
        }
        if ui.button(format!("{} Check", icon::PLUS)).clicked() {
            let n = self.checks.len() + 1;
            self.checks.push(CheckRow {
                name: format!("{}_check{n}", self.name.trim()),
                definition: "CHECK ()".into(),
                ..CheckRow::default()
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_table_from_the_form() {
        let mut e = TableEditor::new("p".into(), "public".into(), Dialect::Postgres, None);
        e.name = "pets".into();
        e.columns.push(ColumnRow {
            name: "name".into(),
            data_type: "text".into(),
            nullable: false,
            default: "'?'".into(),
            ..ColumnRow::default()
        });
        e.indexes.push(IndexRow {
            name: "pets_name".into(),
            columns: "name".into(),
            unique: true,
            ..IndexRow::default()
        });
        let m = e.migration().unwrap();
        assert_eq!(
            m.statements,
            [
                "CREATE TABLE \"public\".\"pets\" (\n    \"id\" bigint NOT NULL,\n    \
                 \"name\" text NOT NULL DEFAULT '?',\n    \
                 CONSTRAINT \"pets_pkey\" PRIMARY KEY (\"id\")\n)",
                "CREATE UNIQUE INDEX \"pets_name\" ON \"public\".\"pets\" (\"name\")",
            ]
        );
        e.name.clear();
        assert!(e.migration().is_err());
    }
}
