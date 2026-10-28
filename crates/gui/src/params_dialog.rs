//! Asking for the values of a query's `:name` / `?` / `$1` placeholders before it runs.

use std::collections::HashMap;

use datadrop_core::params::{self, Param, ParamMode};
use datadrop_core::sql::Dialect;
use egui::RichText;

/// What to do with the SQL once it has its values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Then {
    Run,
    Explain { analyze: bool },
}

pub struct ParamPrompt {
    /// The tab it's for.
    pub doc: u64,
    pub then: Then,
    pub profile: String,
    pub sql: String,
    pub dialect: Dialect,
    pub params: Vec<Param>,
    /// Value and how it's written, per placeholder.
    pub values: Vec<(String, ParamMode)>,
    focus: bool,
}

pub enum ParamsDone {
    /// The SQL with its values in, and the values to remember.
    Run(String, HashMap<String, (String, ParamMode)>),
    Cancel,
}

impl ParamPrompt {
    /// A prompt for `sql`'s placeholders, or `None` if it has none. `remembered` are
    /// the values given last time in this tab.
    pub fn new(
        doc: u64,
        then: Then,
        profile: String,
        sql: String,
        dialect: Dialect,
        remembered: &HashMap<String, (String, ParamMode)>,
    ) -> Option<Self> {
        let params = params::parameters(&sql, dialect);
        if params.is_empty() {
            return None;
        }
        let values = params
            .iter()
            .map(|p| remembered.get(&p.name).cloned().unwrap_or_default())
            .collect();
        Some(Self {
            doc,
            then,
            profile,
            sql,
            dialect,
            params,
            values,
            focus: true,
        })
    }

    pub fn bound(&self) -> String {
        let values = self
            .params
            .iter()
            .zip(&self.values)
            .map(|(p, (v, mode))| (p.name.clone(), params::value_sql(v, *mode, self.dialect)))
            .collect();
        params::bind(&self.sql, &self.params, &values)
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Option<ParamsDone> {
        let mut done = None;
        // The Enter that ran the query opened this; it doesn't also confirm it.
        let first_frame = self.focus;
        egui::Modal::new(egui::Id::new("query-params")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading("Values for the query");
            ui.add_space(4.0);
            ui.weak("Numbers go in as they are, null as NULL, anything else as text.");
            ui.add_space(8.0);
            egui::Grid::new("params-grid")
                .num_columns(3)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    for (i, (p, (value, mode))) in
                        self.params.iter().zip(&mut self.values).enumerate()
                    {
                        let l = ui.label(RichText::new(&p.name).monospace().strong());
                        let resp = ui.add(
                            egui::TextEdit::singleline(value)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(300.0),
                        );
                        let resp = resp.labelled_by(l.id);
                        if self.focus && i == 0 {
                            resp.request_focus();
                        }
                        egui::ComboBox::from_id_salt(("param-mode", i))
                            .selected_text(mode_label(*mode))
                            .width(70.0)
                            .show_ui(ui, |ui| {
                                for m in [ParamMode::Auto, ParamMode::Text, ParamMode::Sql] {
                                    ui.selectable_value(mode, m, mode_label(m));
                                }
                            })
                            .response
                            .on_hover_text(
                                "Auto: numbers as they are, null as NULL, the rest as text. \
                                 Text: always quoted. SQL: written in as it is.",
                            );
                        ui.end_row();
                    }
                });
            self.focus = false;
            ui.add_space(8.0);
            let preview: String = self
                .bound()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let short: String = preview.chars().take(160).collect();
            ui.label(
                RichText::new(if short.len() < preview.len() {
                    format!("{short}…")
                } else {
                    short
                })
                .monospace()
                .weak()
                .small(),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let label = match self.then {
                    Then::Run => "Run",
                    Then::Explain { analyze: false } => "Explain",
                    Then::Explain { analyze: true } => "Explain analyze",
                };
                let enter = !first_frame && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button(RichText::new(label).strong()).clicked() || enter {
                    let remember = self
                        .params
                        .iter()
                        .zip(&self.values)
                        .map(|(p, v)| (p.name.clone(), v.clone()))
                        .collect();
                    done = Some(ParamsDone::Run(self.bound(), remember));
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    done = Some(ParamsDone::Cancel);
                }
            });
        });
        done
    }
}

fn mode_label(m: ParamMode) -> &'static str {
    match m {
        ParamMode::Auto => "Auto",
        ParamMode::Text => "Text",
        ParamMode::Sql => "SQL",
    }
}
