//! Importing a CSV, TSV or JSON file into a table: which file column goes where.

use std::path::PathBuf;

use datadrop_core::import::ImportPreview;
use datadrop_core::{Command, Output};
use egui::RichText;

use crate::backend::{Backend, Task, poll};

pub struct ImportDialog {
    pub profile: String,
    pub schema: String,
    pub table: String,
    pub path: PathBuf,
    header: bool,
    empty_as_null: bool,
    /// The table's columns.
    columns: Vec<String>,
    columns_task: Option<Task>,
    preview: Option<ImportPreview>,
    preview_task: Option<Task>,
    /// For each file column, the table column it goes into.
    pub targets: Vec<Option<String>>,
    importing: Option<Task>,
    error: Option<String>,
}

pub enum ImportDone {
    Imported(u64),
    Cancelled,
}

impl ImportDialog {
    pub fn new(
        backend: &Backend,
        profile: String,
        schema: String,
        table: String,
        path: PathBuf,
    ) -> Self {
        let mut d = Self {
            columns_task: Some(backend.run(Command::Columns {
                profile: profile.clone(),
                schema: Some(schema.clone()),
                table: table.clone(),
            })),
            profile,
            schema,
            table,
            path,
            header: true,
            empty_as_null: true,
            columns: vec![],
            preview: None,
            preview_task: None,
            targets: vec![],
            importing: None,
            error: None,
        };
        d.load_preview(backend);
        d
    }

    fn load_preview(&mut self, backend: &Backend) {
        self.preview_task = Some(backend.run(Command::ImportPreview {
            path: self.path.clone(),
            format: None,
            header: self.header,
        }));
    }

    /// Match file columns to table columns by name (ignoring case), or in order when
    /// the file has no header.
    fn guess_targets(&mut self) {
        let Some(p) = &self.preview else { return };
        self.targets = p
            .columns
            .iter()
            .enumerate()
            .map(|(i, name)| {
                if self.header {
                    self.columns
                        .iter()
                        .find(|c| c.eq_ignore_ascii_case(name.trim()))
                        .cloned()
                } else {
                    self.columns.get(i).cloned()
                }
            })
            .collect();
    }

    pub fn ui(&mut self, ctx: &egui::Context, backend: &Backend) -> Option<ImportDone> {
        if let Some(r) = poll(&mut self.columns_task) {
            match r {
                Ok(Output::Columns { columns, .. }) => {
                    self.columns = columns.into_iter().map(|c| c.name).collect();
                    self.guess_targets();
                }
                Ok(_) => {}
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        if let Some(r) = poll(&mut self.preview_task) {
            match r {
                Ok(Output::ImportPreview(p)) => {
                    self.preview = Some(p);
                    self.error = None;
                    self.guess_targets();
                }
                Ok(_) => {}
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        let mut done = None;
        if let Some(r) = poll(&mut self.importing) {
            match r {
                Ok(Output::Imported { rows }) => done = Some(ImportDone::Imported(rows)),
                Ok(_) => {}
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        let busy = self.importing.is_some();
        egui::Modal::new(egui::Id::new("import")).show(ctx, |ui| {
            ui.set_width(640.0);
            ui.heading(format!("Import into {}", self.table));
            ui.weak(self.path.display().to_string());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .checkbox(&mut self.header, "The first row names the columns")
                    .changed()
                {
                    self.load_preview(backend);
                }
                ui.checkbox(&mut self.empty_as_null, "Empty fields are NULL");
            });
            ui.add_space(6.0);
            match &self.preview {
                None if self.preview_task.is_some() => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak("Reading the file…");
                    });
                }
                None => {}
                Some(p) => {
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .show(ui, |ui| {
                            egui::Grid::new("import-map")
                                .num_columns(3)
                                .striped(true)
                                .spacing([12.0, 4.0])
                                .show(ui, |ui| {
                                    ui.strong("In the file");
                                    ui.strong("Goes into");
                                    ui.strong("First values");
                                    ui.end_row();
                                    for (i, name) in p.columns.iter().enumerate() {
                                        ui.label(RichText::new(name).monospace());
                                        let target = &mut self.targets[i];
                                        egui::ComboBox::from_id_salt(("import-target", i))
                                            .selected_text(match target {
                                                Some(c) => RichText::new(c.as_str()),
                                                None => RichText::new("(left out)").weak(),
                                            })
                                            .width(180.0)
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(target, None, "(left out)");
                                                for c in &self.columns {
                                                    ui.selectable_value(
                                                        target,
                                                        Some(c.clone()),
                                                        c.as_str(),
                                                    );
                                                }
                                            });
                                        let sample: Vec<String> = p
                                            .rows
                                            .iter()
                                            .take(3)
                                            .map(|r| match r.get(i).cloned().flatten() {
                                                Some(v) => v.chars().take(24).collect(),
                                                None => "NULL".into(),
                                            })
                                            .collect();
                                        ui.weak(sample.join(", "));
                                        ui.end_row();
                                    }
                                });
                        });
                }
            }
            if let Some(e) = &self.error {
                ui.add_space(6.0);
                ui.label(RichText::new(e).color(ui.visuals().error_fg_color));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ready = self.preview.is_some() && self.targets.iter().any(Option::is_some);
                ui.add_enabled_ui(ready && !busy, |ui| {
                    if ui.button(RichText::new("Import").strong()).clicked() {
                        self.error = None;
                        self.importing = Some(backend.run(Command::Import {
                            profile: self.profile.clone(),
                            schema: Some(self.schema.clone()),
                            table: self.table.clone(),
                            path: self.path.clone(),
                            format: None,
                            header: self.header,
                            empty_as_null: self.empty_as_null,
                            columns: Some(self.targets.clone()),
                        }));
                    }
                });
                if busy {
                    ui.spinner();
                    ui.weak("Importing… (in one transaction: all or nothing)");
                }
                if ui.button("Cancel").clicked() {
                    if let Some(t) = &self.importing {
                        t.cancel();
                    }
                    done = Some(ImportDone::Cancelled);
                }
            });
        });
        done
    }
}
