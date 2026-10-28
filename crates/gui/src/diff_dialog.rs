//! Comparing two schemas: choose them, and the SQL that makes the target like the
//! source opens in a tab on the target, to look over and run.

use datadrop_core::{Command, Output, ProfileSummary};
use egui::RichText;

use crate::backend::{Backend, Task, poll};

pub struct DiffDialog {
    pub source_profile: String,
    pub source_schema: String,
    pub target_profile: String,
    pub target_schema: String,
    /// Schemas of each side's profile, to choose from.
    source_schemas: (String, Vec<String>, Option<Task>),
    target_schemas: (String, Vec<String>, Option<Task>),
    comparing: Option<Task>,
    error: Option<String>,
}

pub enum DiffDone {
    /// The script, for a tab on the target profile.
    Script {
        profile: String,
        title: String,
        sql: String,
    },
    Cancelled,
}

impl DiffDialog {
    pub fn new(source_profile: String, source_schema: String) -> Self {
        Self {
            target_profile: source_profile.clone(),
            target_schema: String::new(),
            source_profile,
            source_schema,
            source_schemas: Default::default(),
            target_schemas: Default::default(),
            comparing: None,
            error: None,
        }
    }

    fn refresh(side: &mut (String, Vec<String>, Option<Task>), profile: &str, backend: &Backend) {
        if side.0 != profile {
            side.0 = profile.to_owned();
            side.1.clear();
            side.2 = Some(backend.run(Command::Schemas {
                profile: profile.to_owned(),
            }));
        }
        if let Some(Ok(Output::Schemas { schemas })) = poll(&mut side.2) {
            side.1 = schemas;
        }
    }

    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        backend: &Backend,
        profiles: &[ProfileSummary],
    ) -> Option<DiffDone> {
        Self::refresh(&mut self.source_schemas, &self.source_profile, backend);
        Self::refresh(&mut self.target_schemas, &self.target_profile, backend);
        let mut done = None;
        if let Some(r) = poll(&mut self.comparing) {
            match r {
                Ok(Output::Migration { migration, .. }) => {
                    let sql = if migration.statements.is_empty() {
                        format!(
                            "-- {}.{} already matches {}.{}: nothing to change.\n",
                            self.target_profile,
                            self.target_schema,
                            self.source_profile,
                            self.source_schema
                        )
                    } else {
                        format!(
                            "-- Makes {}.{} like {}.{}. Look it over before running it.\n{}",
                            self.target_profile,
                            self.target_schema,
                            self.source_profile,
                            self.source_schema,
                            migration.script()
                        )
                    };
                    done = Some(DiffDone::Script {
                        profile: self.target_profile.clone(),
                        title: format!("{} (changes)", self.target_schema),
                        sql,
                    });
                }
                Ok(_) => {}
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        egui::Modal::new(egui::Id::new("schema-diff")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading("Compare schemas");
            ui.weak("The SQL that makes the target like the source opens in a tab.");
            ui.add_space(8.0);
            egui::Grid::new("diff-sides")
                .num_columns(3)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    side_ui(
                        ui,
                        "Source",
                        "diff-source",
                        profiles,
                        &mut self.source_profile,
                        &mut self.source_schema,
                        &self.source_schemas.1,
                    );
                    side_ui(
                        ui,
                        "Target",
                        "diff-target",
                        profiles,
                        &mut self.target_profile,
                        &mut self.target_schema,
                        &self.target_schemas.1,
                    );
                });
            if let Some(e) = &self.error {
                ui.add_space(6.0);
                ui.label(RichText::new(e).color(ui.visuals().error_fg_color));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ready = !self.source_schema.is_empty() && !self.target_schema.is_empty();
                ui.add_enabled_ui(ready && self.comparing.is_none(), |ui| {
                    if ui.button(RichText::new("Compare").strong()).clicked() {
                        self.error = None;
                        self.comparing = Some(backend.run(Command::SchemaDiff {
                            source_profile: self.source_profile.clone(),
                            source_schema: Some(self.source_schema.clone()),
                            target_profile: self.target_profile.clone(),
                            target_schema: Some(self.target_schema.clone()),
                            apply: false,
                        }));
                    }
                });
                if self.comparing.is_some() {
                    ui.spinner();
                }
                if ui.button("Cancel").clicked() {
                    done = Some(DiffDone::Cancelled);
                }
            });
        });
        done
    }
}

fn side_ui(
    ui: &mut egui::Ui,
    label: &str,
    id: &str,
    profiles: &[ProfileSummary],
    profile: &mut String,
    schema: &mut String,
    schemas: &[String],
) {
    ui.label(label);
    egui::ComboBox::from_id_salt((id, "profile"))
        .selected_text(profile.as_str())
        .width(180.0)
        .show_ui(ui, |ui| {
            for p in profiles {
                ui.selectable_value(profile, p.name.clone(), &p.name);
            }
        });
    egui::ComboBox::from_id_salt((id, "schema"))
        .selected_text(if schema.is_empty() {
            "choose a schema"
        } else {
            schema.as_str()
        })
        .width(180.0)
        .show_ui(ui, |ui| {
            for s in schemas {
                ui.selectable_value(schema, s.clone(), s);
            }
        });
    ui.end_row();
}
