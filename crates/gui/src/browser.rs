//! The left pane: connection profiles and, under each, its schemas with their tables
//! (columns, keys, indexes, triggers), views, routines and sequences, loaded when
//! opened.

use std::collections::HashMap;

use std::sync::Arc;

use datadrop_core::schema::{ConstraintKind, RoutineKind};
use datadrop_core::{
    Command, ObjectKind, Output, ProfileSummary, SchemaModel, TableKind, TableModel,
};
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, RichText};
use egui_phosphor::regular as icon;

use crate::backend::{Backend, Task, poll};
use crate::profile_form::TrustPrompt;
use crate::{kit, theme};

/// `glyph  text`, the glyph in its own colour: one label, read as one string.
fn item(ui: &egui::Ui, glyph: &str, glyph_color: Color32, text: &str, strong: bool) -> LayoutJob {
    let p = theme::colors(ui);
    let family = if strong {
        theme::semibold()
    } else {
        egui::FontFamily::Proportional
    };
    let mut job = LayoutJob::default();
    job.append(
        glyph,
        0.0,
        TextFormat::simple(
            FontId::new(13.0, egui::FontFamily::Proportional),
            glyph_color,
        ),
    );
    job.append(
        &format!("  {text}"),
        0.0,
        TextFormat::simple(FontId::new(13.0, family), p.text),
    );
    job
}

/// A group's heading ("Tables (3)"), quieter than what's in it.
fn group(ui: &egui::Ui, text: String) -> RichText {
    RichText::new(text)
        .font(FontId::new(12.5, theme::medium()))
        .color(theme::colors(ui).muted)
}

enum Load<T> {
    Loading(Option<Task>),
    Ready(T),
    Failed(String),
}

#[derive(Default)]
pub struct Browser {
    /// An SSH server whose key isn't known came up while loading.
    pub untrusted: Option<TrustPrompt>,
    schemas: HashMap<String, Load<Vec<String>>>,
    models: HashMap<(String, String), Load<Arc<SchemaModel>>>,
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
    /// Show a schema's diagram.
    Diagram {
        profile: String,
        schema: String,
    },
    /// Compare a schema with another.
    Compare {
        profile: String,
        schema: String,
    },
    /// Open the table editor on a table, or (`None`) a new one.
    EditTable {
        profile: String,
        schema: String,
        table: Option<Box<TableModel>>,
    },
    /// Import a file into a table.
    Import {
        profile: String,
        schema: String,
        table: String,
    },
    /// Open a tab with the SQL that creates something.
    ShowDdl {
        profile: String,
        schema: String,
        kind: ObjectKind,
        name: String,
    },
}

/// Collect results of finished loads into `Ready` / `Failed`.
fn settle<K: Eq + std::hash::Hash, T>(
    map: &mut HashMap<K, Load<T>>,
    untrusted: &mut Option<TrustPrompt>,
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
                Some(Err(e)) => {
                    if let Some(t) = TrustPrompt::from_error(&e) {
                        *untrusted = Some(t);
                    }
                    *load = Load::Failed(e.to_string());
                }
            }
        }
    }
}

impl Browser {
    pub fn poll(&mut self) {
        settle(&mut self.schemas, &mut self.untrusted, |o| match o {
            Output::Schemas { schemas } => Some(schemas),
            _ => None,
        });
        settle(&mut self.models, &mut self.untrusted, |o| match o {
            Output::Schema(m) => Some(Arc::new(m)),
            _ => None,
        });
    }

    /// Drop everything loaded, for every profile.
    pub fn forget_all(&mut self) {
        self.schemas.clear();
        self.models.clear();
    }

    /// Drop everything loaded for a profile, so it's read again when shown.
    pub fn forget(&mut self, profile: &str) {
        self.schemas.remove(profile);
        self.models.retain(|(p, _), _| p != profile);
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        profiles: &[ProfileSummary],
        backend: &Backend,
    ) -> Option<BrowserAction> {
        let mut action = None;
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            kit::caps(
                ui,
                &match profiles.len() {
                    0 => String::new(),
                    1 => "1 connection".into(),
                    n => format!("{n} connections"),
                },
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(kit::ghost(ui, icon::PLUS))
                    .on_hover_text("Add a connection")
                    .clicked()
                {
                    action = Some(BrowserAction::Add);
                }
            });
        });
        ui.add_space(2.0);
        if profiles.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No connections yet.").color(theme::colors(ui).muted));
                ui.add_space(8.0);
                if ui
                    .add(kit::primary(ui, format!("{} Add a connection", icon::PLUS)))
                    .clicked()
                {
                    action = Some(BrowserAction::Add);
                }
            });
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
        let (color, _) = kit::driver_style(p.driver, ui.visuals().dark_mode);
        let header = egui::CollapsingHeader::new(item(ui, icon::DATABASE, color, &p.name, true))
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
        let faint = theme::colors(ui).faint;
        let header =
            egui::CollapsingHeader::new(item(ui, icon::FOLDER_SIMPLE, faint, schema, false))
                .id_salt(("schema", &p.name, schema))
                .show(ui, |ui| {
                    let key = (p.name.clone(), schema.to_owned());
                    let load = self.models.entry(key).or_insert_with(|| {
                        Load::Loading(Some(backend.run(Command::Schema {
                            profile: p.name.clone(),
                            schema: Some(schema.to_owned()),
                        })))
                    });
                    let m = match load {
                        Load::Loading(_) => return spinner(ui),
                        Load::Failed(e) => return failed(ui, e),
                        Load::Ready(m) => m.clone(),
                    };
                    let (views, tables): (Vec<&TableModel>, Vec<&TableModel>) =
                        m.tables.iter().partition(|t| t.is_view());
                    if m.tables.is_empty() && m.routines.is_empty() && m.sequences.is_empty() {
                        ui.weak("Nothing here yet.");
                    }
                    for (label, list) in [("Tables", &tables), ("Views", &views)] {
                        if list.is_empty() {
                            continue;
                        }
                        egui::CollapsingHeader::new(group(ui, format!("{label} ({})", list.len())))
                            .id_salt(("group", &p.name, schema, label))
                            .default_open(label == "Tables")
                            .show(ui, |ui| {
                                for t in list {
                                    if let Some(a) = table_ui(ui, p, schema, t) {
                                        action = Some(a);
                                    }
                                }
                            });
                    }
                    if !m.routines.is_empty() {
                        egui::CollapsingHeader::new(group(
                            ui,
                            format!("Routines ({})", m.routines.len()),
                        ))
                        .id_salt(("group", &p.name, schema, "routines"))
                        .show(ui, |ui| {
                            for r in &m.routines {
                                let glyph = match r.kind {
                                    RoutineKind::Function => icon::FUNCTION,
                                    RoutineKind::Procedure => icon::GEAR,
                                };
                                let amber = theme::colors(ui).amber;
                                let resp = ui
                                    .label(item(ui, glyph, amber, &r.name, false))
                                    .on_hover_text(match &r.returns {
                                        Some(ret) => {
                                            format!("{}({}) → {ret}", r.name, r.arguments)
                                        }
                                        None => format!("{}({})", r.name, r.arguments),
                                    });
                                resp.context_menu(|ui| {
                                    if ui.button(format!("{} Show DDL", icon::CODE)).clicked() {
                                        action = Some(BrowserAction::ShowDdl {
                                            profile: p.name.clone(),
                                            schema: schema.to_owned(),
                                            kind: ObjectKind::Routine,
                                            name: r.name.clone(),
                                        });
                                    }
                                });
                            }
                        });
                    }
                    if !m.sequences.is_empty() {
                        egui::CollapsingHeader::new(group(
                            ui,
                            format!("Sequences ({})", m.sequences.len()),
                        ))
                        .id_salt(("group", &p.name, schema, "sequences"))
                        .show(ui, |ui| {
                            for seq in &m.sequences {
                                let teal = theme::colors(ui).teal;
                                let resp =
                                    ui.label(item(ui, icon::LIST_NUMBERS, teal, &seq.name, false));
                                resp.context_menu(|ui| {
                                    if ui.button(format!("{} Show DDL", icon::CODE)).clicked() {
                                        action = Some(BrowserAction::ShowDdl {
                                            profile: p.name.clone(),
                                            schema: schema.to_owned(),
                                            kind: ObjectKind::Sequence,
                                            name: seq.name.clone(),
                                        });
                                    }
                                });
                            }
                        });
                    }
                });
        header.header_response.context_menu(|ui| {
            if ui.button(format!("{} New table…", icon::PLUS)).clicked() {
                action = Some(BrowserAction::EditTable {
                    profile: p.name.clone(),
                    schema: schema.to_owned(),
                    table: None,
                });
            }
            if ui.button(format!("{} Diagram", icon::GRAPH)).clicked() {
                action = Some(BrowserAction::Diagram {
                    profile: p.name.clone(),
                    schema: schema.to_owned(),
                });
            }
            if ui
                .button(format!("{} Compare with…", icon::ARROWS_LEFT_RIGHT))
                .clicked()
            {
                action = Some(BrowserAction::Compare {
                    profile: p.name.clone(),
                    schema: schema.to_owned(),
                });
            }
            if ui
                .button(format!("{} Show DDL of everything", icon::CODE))
                .clicked()
            {
                action = Some(BrowserAction::ShowDdl {
                    profile: p.name.clone(),
                    schema: schema.to_owned(),
                    kind: ObjectKind::Schema,
                    name: String::new(),
                });
            }
            if ui
                .button(format!("{} Refresh", icon::ARROWS_CLOCKWISE))
                .clicked()
            {
                self.models.remove(&(p.name.clone(), schema.to_owned()));
            }
        });
        action
    }
}

fn table_ui(
    ui: &mut egui::Ui,
    p: &ProfileSummary,
    schema: &str,
    t: &TableModel,
) -> Option<BrowserAction> {
    let mut action = None;
    let c = theme::colors(ui);
    let (glyph, color) = match t.kind {
        TableKind::Table | TableKind::ForeignTable => (icon::TABLE, c.accent),
        TableKind::View | TableKind::MaterializedView => (icon::EYE, c.purple),
    };
    let pk: &[String] = t.primary_key().map_or(&[], |k| &k.columns);
    let fk_columns: Vec<&String> = t.foreign_keys().flat_map(|f| &f.columns).collect();
    let header = egui::CollapsingHeader::new(item(ui, glyph, color, &t.name, false))
        .id_salt(("table", &p.name, schema, &t.name))
        .show(ui, |ui| {
            for c in &t.columns {
                let p = theme::colors(ui);
                let (mark, mark_color) = if pk.contains(&c.name) {
                    (icon::KEY, p.amber)
                } else if fk_columns.contains(&&c.name) {
                    (icon::LINK, p.accent)
                } else {
                    (" ", p.faint)
                };
                let resp = ui.horizontal(|ui| {
                    ui.label(RichText::new(mark).color(mark_color));
                    ui.label(RichText::new(&c.name).color(p.text));
                    ui.label(RichText::new(&c.data_type).size(11.5).color(p.faint));
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
                for extra in [&c.identity, &c.generated, &c.comment]
                    .into_iter()
                    .flatten()
                {
                    tip.push_str(&format!("\n{extra}"));
                }
                resp.response.on_hover_text(tip);
            }
            let keys: Vec<_> = t
                .constraints
                .iter()
                .filter(|c| c.kind != ConstraintKind::ForeignKey)
                .collect();
            let fks: Vec<_> = t.foreign_keys().collect();
            if !keys.is_empty() {
                section(
                    ui,
                    ("keys", &p.name, schema, &t.name),
                    "Keys",
                    keys.len(),
                    |ui| {
                        for c in &keys {
                            let glyph = match c.kind {
                                ConstraintKind::PrimaryKey => icon::KEY,
                                ConstraintKind::Unique => icon::FINGERPRINT,
                                _ => icon::CHECK_SQUARE,
                            };
                            ui.label(format!("{glyph} {}", c.name))
                                .on_hover_text(&c.definition);
                        }
                    },
                );
            }
            if !fks.is_empty() {
                section(
                    ui,
                    ("fks", &p.name, schema, &t.name),
                    "Foreign keys",
                    fks.len(),
                    |ui| {
                        for c in &fks {
                            let target = c.references.as_ref().map_or(String::new(), |r| {
                                format!("→ {}({})", r.table, r.columns.join(", "))
                            });
                            ui.horizontal(|ui| {
                                ui.label(format!("{} {}", icon::LINK, c.columns.join(", ")));
                                ui.weak(target);
                            })
                            .response
                            .on_hover_text(format!("{}\n{}", c.name, c.definition));
                        }
                    },
                );
            }
            if !t.indexes.is_empty() {
                section(
                    ui,
                    ("indexes", &p.name, schema, &t.name),
                    "Indexes",
                    t.indexes.len(),
                    |ui| {
                        for i in &t.indexes {
                            ui.horizontal(|ui| {
                                ui.label(format!("{} {}", icon::LIGHTNING, i.name));
                                ui.weak(format!(
                                    "{}({})",
                                    if i.unique { "unique " } else { "" },
                                    i.columns.join(", ")
                                ));
                            })
                            .response
                            .on_hover_text(&i.definition);
                        }
                    },
                );
            }
            if !t.triggers.is_empty() {
                section(
                    ui,
                    ("triggers", &p.name, schema, &t.name),
                    "Triggers",
                    t.triggers.len(),
                    |ui| {
                        for tr in &t.triggers {
                            ui.label(format!("{} {}", icon::LIGHTNING_A, tr.name))
                                .on_hover_text(&tr.definition);
                        }
                    },
                );
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
    let mut tip = format!("{kind}. Double-click to see its rows.");
    if let Some(c) = &t.comment {
        tip = format!("{c}\n\n{tip}");
    }
    let resp = header.header_response.on_hover_text(tip);
    if resp.double_clicked() {
        action = Some(open());
    }
    resp.context_menu(|ui| {
        if ui.button(format!("{} Show rows", icon::ROWS)).clicked() {
            action = Some(open());
        }
        if ui.button(format!("{} Show DDL", icon::CODE)).clicked() {
            action = Some(BrowserAction::ShowDdl {
                profile: p.name.clone(),
                schema: schema.to_owned(),
                kind: ObjectKind::Table,
                name: t.name.clone(),
            });
        }
        if !t.is_view()
            && ui
                .button(format!("{} Import data…", icon::UPLOAD_SIMPLE))
                .clicked()
        {
            action = Some(BrowserAction::Import {
                profile: p.name.clone(),
                schema: schema.to_owned(),
                table: t.name.clone(),
            });
        }
        if ui.button(format!("{} Copy name", icon::COPY)).clicked() {
            ui.ctx().copy_text(t.name.clone());
        }
    });
    action
}

/// A collapsed group under a table (keys, indexes...).
fn section(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    n: usize,
    body: impl FnOnce(&mut egui::Ui),
) {
    egui::CollapsingHeader::new(group(ui, format!("{label} ({n})")))
        .id_salt(id)
        .show(ui, body);
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
