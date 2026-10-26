//! The History view in the left pane: queries run before, newest first.

use datadrop_core::history::{self, HistoryEntry};
use datadrop_core::{Command, Output};
use egui::RichText;
use egui_phosphor::regular as icon;

use crate::backend::{Backend, Task, poll};

#[derive(Default)]
pub struct HistoryView {
    pub search: String,
    pub entries: Vec<HistoryEntry>,
    task: Option<Task>,
    /// Reload before showing (a query finished, or the search changed).
    pub stale: bool,
    loaded: bool,
    selected: Option<usize>,
}

pub enum HistoryAction {
    OpenInTab(HistoryEntry),
    Insert(String),
    Clear,
}

impl HistoryView {
    pub fn poll(&mut self) {
        if let Some(r) = poll(&mut self.task) {
            if let Ok(Output::History { entries }) = r {
                self.entries = entries;
                self.selected = None;
            }
            self.loaded = true;
        }
    }

    fn reload(&mut self, backend: &Backend) {
        self.stale = false;
        self.task = Some(backend.run(Command::HistoryList {
            profile: None,
            search: Some(self.search.clone()),
            limit: Some(500),
        }));
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, backend: &Backend) -> Option<HistoryAction> {
        if (self.stale || !self.loaded) && self.task.is_none() {
            self.reload(backend);
        }
        let mut action = None;
        ui.horizontal(|ui| {
            let l = ui.label(icon::MAGNIFYING_GLASS);
            let field = ui
                .add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search history")
                        .desired_width(ui.available_width() - 30.0),
                )
                .labelled_by(l.id);
            if field.changed() {
                self.stale = true;
            }
            if ui
                .small_button(icon::TRASH)
                .on_hover_text("Clear the history")
                .clicked()
            {
                action = Some(HistoryAction::Clear);
            }
        });
        ui.separator();
        if self.entries.is_empty() {
            ui.add_space(8.0);
            ui.weak(if self.loaded && self.search.is_empty() {
                "Queries you run appear here."
            } else if self.loaded {
                "No queries match."
            } else {
                "Loading…"
            });
            return action;
        }
        let now = history::now();
        let mono = egui::TextStyle::Monospace.resolve(ui.style());
        egui::ScrollArea::vertical()
            .id_salt("history")
            .auto_shrink(false)
            .show(ui, |ui| {
                for (i, e) in self.entries.iter().enumerate() {
                    let selected = self.selected == Some(i);
                    let frame = egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(6, 4))
                        .corner_radius(4.0)
                        .fill(if selected {
                            ui.visuals().selection.bg_fill
                        } else {
                            egui::Color32::TRANSPARENT
                        });
                    let resp = frame
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let mut meta = format!(
                                "{} · {} · {} ms",
                                history::ago(e.at, now),
                                e.profile,
                                e.elapsed_ms
                            );
                            if e.error.is_none()
                                && let Some(n) = e.rows
                            {
                                meta.push_str(&format!(" · {n} rows"));
                            }
                            ui.horizontal(|ui| {
                                if e.error.is_some() {
                                    ui.label(
                                        RichText::new(icon::WARNING_CIRCLE)
                                            .color(ui.visuals().error_fg_color),
                                    );
                                }
                                ui.weak(RichText::new(meta).small());
                            });
                            let preview: String =
                                e.sql.lines().take(2).collect::<Vec<_>>().join("\n");
                            ui.add(
                                egui::Label::new(RichText::new(preview).font(mono.clone()))
                                    .truncate(),
                            );
                        })
                        .response
                        .interact(egui::Sense::click());
                    let mut tip = e.sql.clone();
                    if let Some(err) = &e.error {
                        tip.push_str(&format!("\n\nFailed: {err}"));
                    }
                    let resp = resp.on_hover_text(tip);
                    if resp.clicked() {
                        self.selected = Some(i);
                    }
                    if resp.double_clicked() {
                        action = Some(HistoryAction::OpenInTab(e.clone()));
                    }
                    resp.context_menu(|ui| {
                        if ui
                            .button(format!("{} Open in a new tab", icon::NOTE_PENCIL))
                            .clicked()
                        {
                            action = Some(HistoryAction::OpenInTab(e.clone()));
                        }
                        if ui
                            .button(format!("{} Insert at the cursor", icon::TEXT_INDENT))
                            .clicked()
                        {
                            action = Some(HistoryAction::Insert(e.sql.clone()));
                        }
                        if ui.button(format!("{} Copy", icon::COPY)).clicked() {
                            ui.ctx().copy_text(e.sql.clone());
                        }
                    });
                }
            });
        action
    }
}
