//! The left pane: connection profiles and, under each, its schemas, tables and
//! columns, loaded when opened.

use std::collections::HashMap;

use datadrop_core::{ColumnInfo, Command, Output, ProfileSummary, TableInfo, TableKind};
use egui::RichText;
use egui_phosphor::regular as icon;

use crate::backend::{Backend, Task, poll};

enum Load<T> {
    Loading(Option<Task>),
    Ready(T),
    Failed(String),
}

#[derive(Default)]
pub struct Browser {
    schemas: HashMap<String, Load<Vec<String>>>,
    tables: HashMap<(String, String), Load<Vec<TableInfo>>>,
    columns: HashMap<(String, String, String), Load<Vec<ColumnInfo>>>,
}

pub enum BrowserAction {
    NewTab(String),
    /// Open a tab showing the table's first rows.
    OpenTable {
        profile: String,
        schema: String,
        table: String,
    },
    Edit(String),
    Delete(String),
    Add,
}

/// Collect results of finished loads into `Ready` / `Failed`.
fn settle<K: Eq + std::hash::Hash, T>(
    map: &mut HashMap<K, Load<T>>,
    take: impl Fn(Output) -> Option<T>,
) {
    for load in map.values_mut() {
        if let Load::Loading(task) = load {
            match poll(task) {
                None => {}
                Some(Ok(out)) => {
                    *load = match take(out) {
                        Some(v) => Load::Ready(v),
                        None => Load::Failed("unexpected reply".into()),
                    }
                }
                Some(Err(e)) => *load = Load::Failed(e.to_string()),
            }
        }
    }
}

impl Browser {
    pub fn poll(&mut self) {
        settle(&mut self.schemas, |o| match o {
            Output::Schemas { schemas } => Some(schemas),
            _ => None,
        });
        settle(&mut self.tables, |o| match o {
            Output::Tables { tables, .. } => Some(tables),
            _ => None,
        });
        settle(&mut self.columns, |o| match o {
            Output::Columns { columns, .. } => Some(columns),
            _ => None,
        });
    }

    /// Drop everything loaded for a profile, so it's read again when shown.
    pub fn forget(&mut self, profile: &str) {
        self.schemas.remove(profile);
        self.tables.retain(|(p, _), _| p != profile);
        self.columns.retain(|(p, _, _), _| p != profile);
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        profiles: &[ProfileSummary],
        backend: &Backend,
    ) -> Option<BrowserAction> {
        let mut action = None;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.weak(match profiles.len() {
                0 => String::new(),
                1 => "1 connection".into(),
                n => format!("{n} connections"),
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button(icon::PLUS)
                    .on_hover_text("Add a connection")
                    .clicked()
                {
                    action = Some(BrowserAction::Add);
                }
            });
        });
        ui.separator();
        if profiles.is_empty() {
            ui.add_space(12.0);
            ui.weak("No connections yet.");
            ui.add_space(6.0);
            if ui
                .button(format!("{} Add a connection", icon::PLUS))
                .clicked()
            {
                action = Some(BrowserAction::Add);
            }
            return action;
        }
        egui::ScrollArea::both()
            .id_salt("browser")
            .auto_shrink(false)
            .show(ui, |ui| {
                for p in profiles {
                    if let Some(a) = self.profile_ui(ui, p, backend) {
                        action = Some(a);
                    }
                }
            });
        action
    }

    fn profile_ui(
        &mut self,
        ui: &mut egui::Ui,
        p: &ProfileSummary,
        backend: &Backend,
    ) -> Option<BrowserAction> {
        let mut action = None;
        let header = egui::CollapsingHeader::new(
            RichText::new(format!("{}  {}", icon::DATABASE, p.name)).strong(),
        )
        .id_salt(("profile", &p.name))
        .show(ui, |ui| {
            let load = self.schemas.entry(p.name.clone()).or_insert_with(|| {
                Load::Loading(Some(backend.run(Command::Schemas {
                    profile: p.name.clone(),
                })))
            });
            let schemas = match load {
                Load::Loading(_) => return spinner(ui),
                Load::Failed(e) => return failed(ui, e),
                Load::Ready(s) => s.clone(),
            };
            if schemas.is_empty() {
                ui.weak("No schemas you can see.");
            }
            for schema in schemas {
                if let Some(a) = self.schema_ui(ui, p, &schema, backend) {
                    action = Some(a);
                }
            }
        });
        header
            .header_response
            .on_hover_text(format!("{} · {}", p.driver.label(), p.target))
            .context_menu(|ui| {
                if ui
                    .button(format!("{} New query tab", icon::NOTE_PENCIL))
                    .clicked()
                {
                    action = Some(BrowserAction::NewTab(p.name.clone()));
                }
                if ui
                    .button(format!("{} Refresh", icon::ARROWS_CLOCKWISE))
                    .clicked()
                {
                    self.forget(&p.name);
                }
                ui.separator();
                if ui
                    .button(format!("{} Edit…", icon::PENCIL_SIMPLE))
                    .clicked()
                {
                    action = Some(BrowserAction::Edit(p.name.clone()));
                }
                if ui.button(format!("{} Delete…", icon::TRASH)).clicked() {
                    action = Some(BrowserAction::Delete(p.name.clone()));
                }
            });
        action
    }

    fn schema_ui(
        &mut self,
        ui: &mut egui::Ui,
        p: &ProfileSummary,
        schema: &str,
        backend: &Backend,
    ) -> Option<BrowserAction> {
        let mut action = None;
        egui::CollapsingHeader::new(format!("{}  {schema}", icon::FOLDER_SIMPLE))
            .id_salt(("schema", &p.name, schema))
            .show(ui, |ui| {
                let key = (p.name.clone(), schema.to_owned());
                let load = self.tables.entry(key).or_insert_with(|| {
                    Load::Loading(Some(backend.run(Command::Tables {
                        profile: p.name.clone(),
                        schema: Some(schema.to_owned()),
                    })))
                });
                let tables = match load {
                    Load::Loading(_) => return spinner(ui),
                    Load::Failed(e) => return failed(ui, e),
                    Load::Ready(t) => t.clone(),
                };
                if tables.is_empty() {
                    ui.weak("No tables.");
                }
                for t in tables {
                    if let Some(a) = self.table_ui(ui, p, schema, &t, backend) {
                        action = Some(a);
                    }
                }
            });
        action
    }

    fn table_ui(
        &mut self,
        ui: &mut egui::Ui,
        p: &ProfileSummary,
        schema: &str,
        t: &TableInfo,
        backend: &Backend,
    ) -> Option<BrowserAction> {
        let mut action = None;
        let glyph = match t.kind {
            TableKind::Table | TableKind::ForeignTable => icon::TABLE,
            TableKind::View | TableKind::MaterializedView => icon::EYE,
        };
        let header = egui::CollapsingHeader::new(format!("{glyph}  {}", t.name))
            .id_salt(("table", &p.name, schema, &t.name))
            .show(ui, |ui| {
                let key = (p.name.clone(), schema.to_owned(), t.name.clone());
                let load = self.columns.entry(key).or_insert_with(|| {
                    Load::Loading(Some(backend.run(Command::Columns {
                        profile: p.name.clone(),
                        schema: Some(schema.to_owned()),
                        table: t.name.clone(),
                    })))
                });
                let columns = match load {
                    Load::Loading(_) => return spinner(ui),
                    Load::Failed(e) => return failed(ui, e),
                    Load::Ready(c) => c,
                };
                for c in columns.iter() {
                    let key = if c.primary_key { icon::KEY } else { " " };
                    let resp = ui.horizontal(|ui| {
                        ui.label(format!("{key} {}", c.name));
                        ui.weak(&c.data_type);
                    });
                    let mut tip = format!(
                        "{} {}{}",
                        c.name,
                        c.data_type,
                        if c.nullable { "" } else { " NOT NULL" }
                    );
                    if let Some(d) = &c.default {
                        tip.push_str(&format!("\nDEFAULT {d}"));
                    }
                    resp.response.on_hover_text(tip);
                }
            });
        let kind = match t.kind {
            TableKind::Table => "Table",
            TableKind::View => "View",
            TableKind::MaterializedView => "Materialized view",
            TableKind::ForeignTable => "Foreign table",
        };
        let open = || BrowserAction::OpenTable {
            profile: p.name.clone(),
            schema: schema.to_owned(),
            table: t.name.clone(),
        };
        let resp = header
            .header_response
            .on_hover_text(format!("{kind}. Double-click to see its rows."));
        if resp.double_clicked() {
            action = Some(open());
        }
        resp.context_menu(|ui| {
            if ui.button(format!("{} Show rows", icon::ROWS)).clicked() {
                action = Some(open());
            }
            if ui.button(format!("{} Copy name", icon::COPY)).clicked() {
                ui.ctx().copy_text(t.name.clone());
            }
        });
        action
    }
}

fn spinner(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.weak("Loading…");
    });
}

fn failed(ui: &mut egui::Ui, e: &str) {
    ui.label(RichText::new(e).color(ui.visuals().error_fg_color).small());
}
