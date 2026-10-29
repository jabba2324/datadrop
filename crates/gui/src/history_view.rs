//! The History view in the left pane: queries run before, newest first.

use datadrop_core::history::{self, HistoryEntry};
use datadrop_core::{Command, Output};
use egui::RichText;
use egui_phosphor::regular as icon;

use crate::backend::{Backend, Task, poll};
use crate::{kit, theme};

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
        let p = theme::colors(ui);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let l = ui.label(RichText::new(icon::MAGNIFYING_GLASS).color(p.faint));
            let field = ui
                .add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search history")
                        // Room for the icon before it and the button after it.
                        .desired_width(ui.available_width() - 64.0),
                )
                .labelled_by(l.id);
            if field.changed() {
                self.stale = true;
            }
            if ui
                .add(kit::ghost(ui, icon::TRASH))
                .on_hover_text("Clear the history")
                .clicked()
            {
                action = Some(HistoryAction::Clear);
            }
        });
        ui.add_space(6.0);
        if self.entries.is_empty() {
            ui.add_space(32.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(if self.loaded && self.search.is_empty() {
                        "Queries you run appear here."
                    } else if self.loaded {
                        "No queries match."
                    } else {
                        "Loading…"
                    })
                    .color(p.muted),
                );
            });
            return action;
        }
        let now = history::now();
        let mono = egui::TextStyle::Monospace.resolve(ui.style());
        egui::ScrollArea::vertical()
            .id_salt("history")
            .auto_shrink([true, false])
            .show(ui, |ui| {
                for (i, e) in self.entries.iter().enumerate() {
                    let selected = self.selected == Some(i);
                    let frame = egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(10, 8))
                        .corner_radius(8)
                        .fill(if selected {
                            p.accent_soft
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
                            let preview: String =
                                e.sql.lines().take(2).collect::<Vec<_>>().join("\n");
                            // The state as a dot before the SQL, in one label so it truncates.
                            let mut job = egui::text::LayoutJob::default();
                            job.append(
                                "● ",
                                0.0,
                                egui::TextFormat::simple(
                                    egui::FontId::proportional(10.0),
                                    if e.error.is_some() { p.bad } else { p.ok },
                                ),
                            );
                            job.append(
                                &preview,
                                0.0,
                                egui::TextFormat::simple(mono.clone(), p.text),
                            );
                            // A job keeps its own wrapping: cut it to the panel, one line.
                            job.wrap = egui::text::TextWrapping {
                                max_width: ui.available_width(),
                                max_rows: 1,
                                break_anywhere: true,
                                overflow_character: Some('…'),
                            };
                            ui.add(egui::Label::new(job));
                            ui.horizontal(|ui| {
                                ui.add_space(12.0);
                                if e.error.is_some() {
                                    ui.label(
                                        RichText::new(icon::WARNING_CIRCLE).size(11.0).color(p.bad),
                                    );
                                }
                                ui.label(RichText::new(meta).size(11.0).color(p.faint));
                            });
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
