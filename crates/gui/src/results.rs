//! The results pane under the editor.

use datadrop_core::{QueryOutput, StatementResult};
use egui::RichText;
use egui_extras::{Column, TableBuilder};
use egui_phosphor::regular as icon;

use crate::editor::{Document, ResultView};
use crate::grid_edit::{CellEditor, Editability, GridEdit};

/// Longest text shown in a cell; the full value is in its tooltip and copies.
const CELL_CHARS: usize = 120;

pub enum ResultsAction {
    None,
    Stop,
    /// Export this result (an index into the output's results).
    Export(usize),
    /// Find out whether this result can be edited.
    CheckEditable(usize),
    /// Show the SQL that would save the edits.
    Review(usize),
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
    // Switch between the last results and the last plan, when there are both.
    let has_plan = doc.plan.is_some() || doc.plan_error.is_some();
    if has_plan && doc.output.is_some() {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut doc.view, ResultView::Results, "Results");
            ui.selectable_value(&mut doc.view, ResultView::Plan, "Plan");
        });
    }
    if doc.view == ResultView::Plan && has_plan {
        if let Some(e) = &doc.plan_error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(ui.visuals().error_fg_color));
        } else if let Some(plan) = &doc.plan {
            crate::plan_view::plan_ui(ui, plan, &mut doc.plan_collapsed, doc.id);
        }
        return ResultsAction::None;
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
        if doc.grid.result != Some(i) {
            doc.grid = GridEdit::for_result(i);
        }
        if matches!(doc.grid.editability, Editability::Unknown) {
            action = ResultsAction::CheckEditable(i);
        }
        let rows = &out.results[i].rows;
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
            ui.separator();
            let grid = &mut doc.grid;
            match &grid.editability {
                Editability::Editable { source, .. } => {
                    if ui
                        .small_button(format!("{} Row", icon::PLUS))
                        .on_hover_text(format!("Add a row to {}", source.table))
                        .clicked()
                    {
                        grid.edits.inserted.push(Default::default());
                        let row = rows.len() + grid.edits.inserted.len() - 1;
                        grid.editing = Some(CellEditor {
                            row,
                            col: 0,
                            buffer: String::new(),
                            focus: true,
                        });
                    }
                    let n = grid.edits.count();
                    if n > 0 {
                        if ui
                            .small_button(format!("{} Revert", icon::ARROW_COUNTER_CLOCKWISE))
                            .on_hover_text("Forget the changes")
                            .clicked()
                        {
                            grid.edits = Default::default();
                            grid.editing = None;
                        }
                        if ui
                            .button(
                                RichText::new(format!(
                                    "{} Review and save ({n})",
                                    icon::FLOPPY_DISK
                                ))
                                .strong(),
                            )
                            .on_hover_text("See the SQL that saves your changes, then run it")
                            .clicked()
                        {
                            grid.commit(rows);
                            action = ResultsAction::Review(i);
                        }
                    } else {
                        ui.weak("Double-click a cell to edit it");
                    }
                }
                Editability::ReadOnly(why) => {
                    ui.weak(format!("{} Read-only", icon::LOCK_SIMPLE))
                        .on_hover_text(why);
                }
                _ => {}
            }
        });
        ui.separator();
        table(ui, &out.results[i], doc.id, &mut doc.grid);
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

fn table(ui: &mut egui::Ui, r: &StatementResult, doc_id: u64, grid: &mut GridEdit) {
    if r.columns.is_empty() {
        return;
    }
    let editable = grid.is_editable();
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 6.0;
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let null_color = ui.visuals().weak_text_color();
    let dark = ui.visuals().dark_mode;
    let changed_bg = if dark {
        egui::Color32::from_rgba_unmultiplied(0xE0, 0xB0, 0x30, 40)
    } else {
        egui::Color32::from_rgba_unmultiplied(0xF0, 0xC0, 0x30, 70)
    };
    let new_bg =
        egui::Color32::from_rgba_unmultiplied(0x40, 0xC0, 0x60, if dark { 35 } else { 50 });
    let deleted_bg =
        egui::Color32::from_rgba_unmultiplied(0xE0, 0x40, 0x40, if dark { 40 } else { 45 });
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
    let n = r.rows.len();
    let total = n + if editable {
        grid.edits.inserted.len()
    } else {
        0
    };
    let index_width = (total.max(1).to_string().len() as f32 * font.size * 0.62 + 30.0).max(40.0);

    // Changes made this frame, applied after the table (which borrows `grid`).
    let mut set: Option<(usize, usize, Option<String>)> = None;
    let mut start_edit: Option<(usize, usize, String)> = None;
    let mut toggle_delete: Option<usize> = None;
    let mut revert: Option<(usize, usize)> = None;
    let mut commit = false;
    let mut cancel = false;

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
                body.rows(row_height, total, |mut row| {
                    let i = row.index();
                    let is_new = i >= n;
                    let deleted = grid.edits.deleted.contains(&i);
                    let original = r.rows.get(i);
                    row.col(|ui| {
                        if deleted {
                            ui.painter().rect_filled(ui.max_rect(), 0.0, deleted_bg);
                        } else if is_new {
                            ui.painter().rect_filled(ui.max_rect(), 0.0, new_bg);
                        }
                        let label = if is_new {
                            "new".to_owned()
                        } else {
                            (i + 1).to_string()
                        };
                        let resp = ui.add(
                            egui::Label::new(RichText::new(label).weak())
                                .sense(egui::Sense::click()),
                        );
                        if editable && !is_new {
                            resp.context_menu(|ui| {
                                let text = if deleted {
                                    "Restore the row"
                                } else {
                                    "Delete the row"
                                };
                                if ui.button(text).clicked() {
                                    toggle_delete = Some(i);
                                }
                            });
                        }
                    });
                    for c in 0..r.columns.len() {
                        row.col(|ui| {
                            let edited = if is_new {
                                grid.edits.inserted.get(i - n).and_then(|m| m.get(&c))
                            } else {
                                grid.edits.cells.get(&(i, c))
                            };
                            let value: Option<&Option<String>> =
                                edited.or_else(|| original.map(|o| &o[c]));
                            if deleted {
                                ui.painter().rect_filled(ui.max_rect(), 0.0, deleted_bg);
                            } else if edited.is_some() {
                                ui.painter().rect_filled(ui.max_rect(), 0.0, changed_bg);
                            } else if is_new {
                                ui.painter().rect_filled(ui.max_rect(), 0.0, new_bg);
                            }

                            // The cell being typed in.
                            if let Some(e) =
                                grid.editing.as_mut().filter(|e| e.row == i && e.col == c)
                            {
                                let resp = ui.add(
                                    egui::TextEdit::singleline(&mut e.buffer)
                                        .font(font.clone())
                                        .desired_width(f32::INFINITY),
                                );
                                if e.focus {
                                    resp.request_focus();
                                    e.focus = false;
                                }
                                if ui.input(|inp| inp.key_pressed(egui::Key::Escape)) {
                                    cancel = true;
                                } else if resp.lost_focus() {
                                    commit = true;
                                }
                                return;
                            }

                            let mut text = match value {
                                Some(None) => RichText::new("NULL").italics().color(null_color),
                                Some(Some(s)) => RichText::new(shorten(s, CELL_CHARS).0),
                                None => RichText::new("default").italics().color(null_color),
                            }
                            .font(font.clone());
                            if deleted {
                                text = text.strikethrough();
                            }
                            let mut resp =
                                ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                            if let Some(Some(s)) = value
                                && shorten(s, CELL_CHARS).1
                            {
                                resp = resp.on_hover_text(s);
                            }
                            if editable && !deleted && resp.double_clicked() {
                                let current = value.cloned().flatten().unwrap_or_default();
                                start_edit = Some((i, c, current));
                            }
                            let values: Vec<Option<String>> = (0..r.columns.len())
                                .map(|k| {
                                    if is_new {
                                        grid.edits.inserted[i - n].get(&k).cloned().flatten()
                                    } else {
                                        grid.edits
                                            .cells
                                            .get(&(i, k))
                                            .cloned()
                                            .unwrap_or_else(|| r.rows[i][k].clone())
                                    }
                                })
                                .collect();
                            resp.context_menu(|ui| {
                                if editable && !deleted {
                                    if ui.button("Edit the value").clicked() {
                                        let current = value.cloned().flatten().unwrap_or_default();
                                        start_edit = Some((i, c, current));
                                    }
                                    if ui.button("Set to NULL").clicked() {
                                        set = Some((i, c, None));
                                    }
                                    if edited.is_some() && ui.button("Undo this change").clicked() {
                                        revert = Some((i, c));
                                    }
                                    if !is_new && ui.button("Delete the row").clicked() {
                                        toggle_delete = Some(i);
                                    }
                                    ui.separator();
                                } else if editable
                                    && deleted
                                    && ui.button("Restore the row").clicked()
                                {
                                    toggle_delete = Some(i);
                                }
                                if ui.button("Copy value").clicked() {
                                    ui.ctx()
                                        .copy_text(value.cloned().flatten().unwrap_or_default());
                                }
                                if ui.button("Copy row").clicked() {
                                    ui.ctx().copy_text(tsv_row(&values));
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

    if cancel {
        grid.editing = None;
    } else if commit {
        grid.commit(&r.rows);
    }
    if let Some((row, col, current)) = start_edit {
        grid.commit(&r.rows);
        grid.editing = Some(CellEditor {
            row,
            col,
            buffer: current,
            focus: true,
        });
    }
    if let Some((row, col, value)) = set {
        grid.set(row, col, value, &r.rows);
    }
    if let Some((row, col)) = revert {
        if row >= n {
            if let Some(m) = grid.edits.inserted.get_mut(row - n) {
                m.remove(&col);
            }
        } else {
            grid.edits.cells.remove(&(row, col));
        }
    }
    if let Some(row) = toggle_delete
        && !grid.edits.deleted.remove(&row)
    {
        grid.edits.deleted.insert(row);
    }
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
