//! The results pane under the editor.

use datadrop_core::{QueryOutput, StatementResult};
use egui::RichText;
use egui_extras::{Column, TableBuilder};
use egui_phosphor::regular as icon;

use crate::editor::Document;

/// Longest text shown in a cell; the full value is in its tooltip and copies.
const CELL_CHARS: usize = 120;

pub enum ResultsAction {
    None,
    Stop,
    /// Export this result (an index into the output's results).
    Export(usize),
}

pub fn results_ui(ui: &mut egui::Ui, doc: &mut Document) -> ResultsAction {
    if let Some(task) = &doc.running {
        let secs = doc.run_started.map_or(0.0, |t| t.elapsed().as_secs_f32());
        let mut action = ResultsAction::None;
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.spinner();
            if task.is_cancelling() {
                ui.label("Stopping…");
            } else {
                ui.label(format!("Running… {secs:.1} s"));
                if ui
                    .button(format!("{} Stop", icon::STOP))
                    .on_hover_text("Cancel the query on the server")
                    .clicked()
                {
                    action = ResultsAction::Stop;
                }
            }
        });
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
        return action;
    }
    let Some(out) = &doc.output else {
        ui.add_space(10.0);
        ui.weak(format!(
            "Run a statement with {} (or the selection), or the whole tab with {}.",
            shortcut("Enter"),
            shortcut("Shift+Enter")
        ));
        return ResultsAction::None;
    };

    let sets: Vec<usize> = out
        .results
        .iter()
        .enumerate()
        .filter(|(_, r)| r.returns_rows)
        .map(|(i, _)| i)
        .collect();
    if doc.shown_result >= sets.len() {
        doc.shown_result = sets.len().saturating_sub(1);
    }

    summary(ui, out);
    if sets.len() > 1 {
        ui.horizontal_wrapped(|ui| {
            for (n, &i) in sets.iter().enumerate() {
                let r = &out.results[i];
                let label = format!("Result {} ({} rows)", n + 1, r.rows.len());
                if ui
                    .selectable_label(doc.shown_result == n, label)
                    .on_hover_text(one_line(&r.sql, 200))
                    .clicked()
                {
                    doc.shown_result = n;
                }
            }
        });
    }
    let mut action = ResultsAction::None;
    if let Some(&i) = sets.get(doc.shown_result) {
        ui.horizontal(|ui| {
            let exporting = doc.exporting.is_some();
            if exporting {
                ui.spinner();
                ui.weak("Exporting…");
            } else if ui
                .small_button(format!("{} Export…", icon::EXPORT))
                .on_hover_text("Save this result as CSV, TSV or JSON")
                .clicked()
            {
                action = ResultsAction::Export(i);
            }
            if ui
                .small_button(format!("{} Copy all", icon::COPY))
                .on_hover_text("Copy the rows shown, with headers, as tab-separated text")
                .clicked()
            {
                ui.ctx().copy_text(tsv_all(&out.results[i]));
            }
        });
        ui.separator();
        table(ui, &out.results[i], doc.id);
    }
    action
}

fn shortcut(key: &str) -> String {
    let m = if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl+"
    };
    format!("{m}{key}")
}

/// One line about what ran: rows, timing, errors.
fn summary(ui: &mut egui::Ui, out: &QueryOutput) {
    ui.add_space(4.0);
    let total_ms: u64 = out.results.iter().map(|r| r.elapsed_ms).sum();
    let last = out.results.last();
    ui.horizontal_wrapped(|ui| {
        if out.results.len() > 1 || (out.error.is_some() && !out.results.is_empty()) {
            ui.label(format!(
                "{} statement{} ran",
                out.results.len(),
                if out.results.len() == 1 { "" } else { "s" }
            ));
            ui.weak("·");
        }
        if let Some(r) = last
            && out.error.is_none()
        {
            if r.returns_rows {
                let rows = r.rows.len();
                ui.label(format!("{rows} row{}", if rows == 1 { "" } else { "s" }));
                if r.truncated {
                    ui.label(
                        RichText::new(format!("{} more not shown", icon::WARNING))
                            .color(ui.visuals().warn_fg_color),
                    )
                    .on_hover_text("Raise the row limit in the toolbar to see more.");
                }
            } else if r.rows_affected > 0 {
                ui.label(format!(
                    "{} row{} affected",
                    r.rows_affected,
                    if r.rows_affected == 1 { "" } else { "s" }
                ));
            } else {
                ui.label(format!("{} Done", icon::CHECK));
            }
            ui.weak("·");
        }
        if !out.results.is_empty() {
            ui.weak(format!("{total_ms} ms"));
        }
    });
    if let Some(e) = &out.error {
        let color = if out.cancelled {
            ui.visuals().warn_fg_color
        } else {
            ui.visuals().error_fg_color
        };
        egui::Frame::new()
            .stroke(egui::Stroke::new(1.0, color))
            .corner_radius(4.0)
            .inner_margin(6.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let message = if out.cancelled {
                    "Stopped: the query was cancelled on the server."
                } else {
                    &e.message
                };
                ui.label(RichText::new(message).color(color));
                if !out.cancelled {
                    ui.weak(format!(
                        "in statement {}: {}",
                        e.index + 1,
                        one_line(&e.sql, 160)
                    ));
                }
            });
    }
}

fn one_line(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    shorten(&flat, max).0
}

/// The text cut to `max` chars (on one line), and whether it was cut.
fn shorten(s: &str, max: usize) -> (String, bool) {
    let first_line = s.lines().next().unwrap_or("");
    let cut = first_line.len() != s.len() || first_line.chars().count() > max;
    if !cut {
        return (s.to_owned(), false);
    }
    let mut out: String = first_line.chars().take(max).collect();
    out.push('…');
    (out, true)
}

fn table(ui: &mut egui::Ui, r: &StatementResult, doc_id: u64) {
    if r.columns.is_empty() {
        return;
    }
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 6.0;
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let null_color = ui.visuals().weak_text_color();
    // Initial widths from the header and the first rows.
    let widths: Vec<f32> = (0..r.columns.len())
        .map(|c| {
            let longest = r
                .rows
                .iter()
                .take(50)
                .map(|row| row[c].as_deref().map_or(4, |v| v.chars().count().min(40)))
                .chain([r.columns[c].name.chars().count() + 2])
                .max()
                .unwrap_or(4);
            (longest as f32 * font.size * 0.62 + 16.0).clamp(50.0, 420.0)
        })
        .collect();
    let index_width =
        (r.rows.len().max(1).to_string().len() as f32 * font.size * 0.62 + 14.0).max(32.0);

    egui::ScrollArea::horizontal()
        .id_salt(("results-h", doc_id))
        .auto_shrink(false)
        .show(ui, |ui| {
            let mut t = TableBuilder::new(ui)
                .id_salt(("results", doc_id, r.sql.as_str()))
                .striped(true)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .auto_shrink(false)
                .column(Column::exact(index_width));
            for w in &widths {
                t = t.column(Column::initial(*w).at_least(36.0).clip(true));
            }
            t.header(row_height + 2.0, |mut h| {
                h.col(|_| {});
                for c in &r.columns {
                    h.col(|ui| {
                        ui.strong(&c.name).on_hover_text(&c.type_name);
                    });
                }
            })
            .body(|body| {
                body.rows(row_height, r.rows.len(), |mut row| {
                    let i = row.index();
                    let values = &r.rows[i];
                    row.col(|ui| {
                        ui.weak((i + 1).to_string());
                    });
                    for (c, v) in values.iter().enumerate() {
                        row.col(|ui| {
                            let resp = match v {
                                None => ui.label(
                                    RichText::new("NULL")
                                        .italics()
                                        .color(null_color)
                                        .font(font.clone()),
                                ),
                                Some(s) => {
                                    let (shown, cut) = shorten(s, CELL_CHARS);
                                    let resp = ui.label(RichText::new(shown).font(font.clone()));
                                    if cut { resp.on_hover_text(s) } else { resp }
                                }
                            };
                            resp.context_menu(|ui| {
                                if ui.button("Copy value").clicked() {
                                    ui.ctx().copy_text(v.clone().unwrap_or_default());
                                }
                                if ui.button("Copy row").clicked() {
                                    ui.ctx().copy_text(tsv_row(values));
                                }
                                if ui.button("Copy all rows (with headers)").clicked() {
                                    ui.ctx().copy_text(tsv_all(r));
                                }
                                if ui
                                    .button(format!("Copy column name ({})", r.columns[c].name))
                                    .clicked()
                                {
                                    ui.ctx().copy_text(r.columns[c].name.clone());
                                }
                            });
                        });
                    }
                });
            });
        });
}

fn tsv_field(v: &Option<String>) -> String {
    v.as_deref().unwrap_or("").replace(['\t', '\n', '\r'], " ")
}

fn tsv_row(values: &[Option<String>]) -> String {
    values.iter().map(tsv_field).collect::<Vec<_>>().join("\t")
}

/// The whole result as tab-separated text: pastes into a spreadsheet.
pub fn tsv_all(r: &StatementResult) -> String {
    let mut s = r
        .columns
        .iter()
        .map(|c| c.name.replace(['\t', '\n'], " "))
        .collect::<Vec<_>>()
        .join("\t");
    for row in &r.rows {
        s.push('\n');
        s.push_str(&tsv_row(row));
    }
    s
}
