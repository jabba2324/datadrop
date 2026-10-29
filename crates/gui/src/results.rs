//! The results pane under the editor.

use datadrop_core::sql::{self, Dialect};
use datadrop_core::{QueryOutput, StatementResult};
use egui::{Margin, RichText, Stroke};
use egui_extras::{Column, TableBuilder};
use egui_phosphor::regular as icon;

use crate::editor::{Document, ResultView};
use crate::grid_edit::{CellEditor, Editability, GridEdit};
use crate::kit;
use crate::theme;

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
    /// Run the query again with the filter typed for it.
    Filter(usize),
    /// Run the query as written again.
    ClearFilter,
    /// Sort by this column on the server (all the rows, not only those shown).
    Sort(usize, usize),
}

pub fn results_ui(
    ui: &mut egui::Ui,
    doc: &mut Document,
    dialect: Dialect,
    max_rows: &mut usize,
) -> ResultsAction {
    let p = theme::colors(ui);
    if let Some(task) = &doc.running {
        let secs = doc.run_started.map_or(0.0, |t| t.elapsed().as_secs_f32());
        let mut action = ResultsAction::None;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spinner();
            if task.is_cancelling() {
                ui.label(RichText::new("Stopping…").color(p.muted));
            } else {
                ui.label(RichText::new(format!("Running… {secs:.1} s")).color(p.muted));
                if ui
                    .add(kit::danger(ui, format!("{} Stop", icon::STOP)))
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
    let has_plan = doc.plan.is_some() || doc.plan_error.is_some();
    let sets: Vec<usize> = doc.output.as_ref().map_or(vec![], |out| {
        out.results
            .iter()
            .enumerate()
            .filter(|(_, r)| r.returns_rows)
            .map(|(i, _)| i)
            .collect()
    });
    if doc.shown_result >= sets.len() {
        doc.shown_result = sets.len().saturating_sub(1);
    }
    let showing_plan = doc.view == ResultView::Plan && has_plan;
    let shown = sets.get(doc.shown_result).copied();
    let mut action = ResultsAction::None;

    // The views, what ran, and what to do with the result.
    ui.horizontal(|ui| {
        // Switch between the last results and the last plan, when there are both.
        if has_plan && doc.output.is_some() {
            for (view, label) in [(ResultView::Results, "Results"), (ResultView::Plan, "Plan")] {
                let on = doc.view == view;
                let text = RichText::new(label).color(if on { p.text } else { p.muted });
                let b = if on {
                    egui::Button::new(text).fill(p.hover).stroke(Stroke::NONE)
                } else {
                    egui::Button::new(text).frame_when_inactive(false)
                };
                if ui.add(b).clicked() {
                    doc.view = view;
                }
            }
            ui.add_space(6.0);
        }
        if !showing_plan && let Some(out) = &doc.output {
            summary(ui, out);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !showing_plan && let (Some(out), Some(i)) = (&doc.output, shown) {
                if doc.exporting.is_some() {
                    ui.label(RichText::new("Exporting…").color(p.muted));
                    ui.spinner();
                } else if ui
                    .button(format!("{} Export…", icon::EXPORT))
                    .on_hover_text("Save this result as CSV, TSV or JSON")
                    .clicked()
                {
                    action = ResultsAction::Export(i);
                }
                if ui
                    .add(kit::ghost(ui, format!("{} Copy all", icon::COPY)))
                    .on_hover_text("Copy the rows shown, with headers, as tab-separated text")
                    .clicked()
                {
                    ui.ctx().copy_text(tsv_all(&out.results[i]));
                }
                ui.add_space(8.0);
            }
            ui.add(
                egui::DragValue::new(max_rows)
                    .range(1..=1_000_000)
                    .speed(10.0),
            )
            .on_hover_text("Rows kept per result; the rest are left out");
            ui.label(RichText::new("Row limit").color(p.faint));
        });
    });

    if showing_plan {
        if let Some(e) = &doc.plan_error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(ui.visuals().error_fg_color));
        } else if let Some(plan) = &doc.plan {
            crate::plan_view::plan_ui(ui, plan, &mut doc.plan_collapsed, doc.id);
        }
        return action;
    }
    let Some(out) = &doc.output else {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("Run a query to see results")
                    .font(egui::FontId::new(13.5, theme::semibold()))
                    .color(p.text),
            );
            ui.label(
                RichText::new(format!(
                    "Run a statement with {} (or the selection), or the whole tab with {}.",
                    shortcut("Enter"),
                    shortcut("Shift+Enter")
                ))
                .color(p.muted),
            );
        });
        return action;
    };
    error_box(ui, out);

    if sets.len() > 1 {
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            for (n, &i) in sets.iter().enumerate() {
                let r = &out.results[i];
                let label = format!("Result {} ({} rows)", n + 1, r.rows.len());
                let on = doc.shown_result == n;
                let b = egui::Button::new(RichText::new(label).color(if on {
                    p.text
                } else {
                    p.muted
                }))
                .fill(if on { p.bg } else { p.panel })
                .stroke(Stroke::new(1.0, if on { p.accent } else { p.border }))
                .corner_radius(7);
                if ui.add(b).on_hover_text(one_line(&r.sql, 200)).clicked() {
                    doc.shown_result = n;
                }
            }
        });
    }
    if let Some(i) = shown {
        if doc.grid.result != Some(i) {
            doc.grid = GridEdit::for_result(i);
        }
        if matches!(doc.grid.editability, Editability::Unknown) {
            action = ResultsAction::CheckEditable(i);
        }
        let rows = &out.results[i].rows;
        let grid = &mut doc.grid;
        let editable_table = match &grid.editability {
            Editability::Editable { source, .. } => Some(source.table.clone()),
            _ => None,
        };
        // The strip keeps its height whatever the state, so the grid doesn't jump when
        // the editability is known.
        let n = grid.edits.count();
        let editable = matches!(grid.editability, Editability::Editable { .. });
        ui.add_space(4.0);
        egui::Frame::new()
            .fill(if !editable {
                egui::Color32::TRANSPARENT
            } else if n > 0 {
                theme::alpha(p.warn, 0x14)
            } else {
                p.accent_soft
            })
            .corner_radius(8)
            .inner_margin(Margin::symmetric(10, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(28.0);
                ui.horizontal(|ui| match &grid.editability {
                    Editability::Editable { .. } => {
                        let table = editable_table.clone().unwrap_or_default();
                        ui.label(
                            RichText::new(format!("{} Editing {table}", icon::PENCIL_SIMPLE))
                                .font(egui::FontId::new(13.0, theme::semibold()))
                                .color(p.text),
                        );
                        if n == 0 {
                            ui.label(
                                RichText::new("Double-click a cell to edit it").color(p.muted),
                            );
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if n > 0 {
                                if ui
                                    .add(kit::primary(
                                        ui,
                                        format!("{} Review and save ({n})", icon::FLOPPY_DISK),
                                    ))
                                    .on_hover_text(
                                        "See the SQL that saves your changes, then run it",
                                    )
                                    .clicked()
                                {
                                    grid.commit(rows);
                                    action = ResultsAction::Review(i);
                                }
                                if ui
                                    .button(format!("{} Revert", icon::ARROW_COUNTER_CLOCKWISE))
                                    .on_hover_text("Forget the changes")
                                    .clicked()
                                {
                                    grid.edits = Default::default();
                                    grid.editing = None;
                                }
                            }
                            if ui
                                .button(format!("{} Row", icon::PLUS))
                                .on_hover_text(format!("Add a row to {table}"))
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
                        });
                    }
                    Editability::ReadOnly(why) => {
                        ui.label(
                            RichText::new(format!("{} Read-only", icon::LOCK_SIMPLE))
                                .color(p.faint),
                        )
                        .on_hover_text(why);
                    }
                    _ => {}
                });
            });
        let r = &out.results[i];
        // A filter is for the query it ran (or, before it's applied, for any).
        let stale = doc
            .filter
            .as_ref()
            .is_some_and(|f| !f.wrapped.is_empty() && f.wrapped != r.sql);
        if stale {
            doc.filter = None;
        }
        let filtered = doc.filter.as_ref().is_some_and(|f| f.wrapped == r.sql);
        if (filtered || sql::is_query(&r.sql, dialect))
            && let Some(a) = filter_bar(ui, doc, i)
        {
            action = a;
        }
        ui.add_space(4.0);
        let Some(out) = &doc.output else {
            return action;
        };
        let r = &out.results[i];
        let server_order = doc
            .filter
            .as_ref()
            .filter(|f| f.wrapped == r.sql)
            .and_then(|f| f.order.clone());
        let sort_on_server = r.truncated || server_order.is_some();
        if let Some(col) = table(ui, r, doc.id, &mut doc.grid, server_order.as_ref()) {
            if sort_on_server {
                action = ResultsAction::Sort(i, col);
            } else {
                doc.grid.cycle_sort(col);
            }
        }
    }
    action
}

/// `WHERE [condition]`: filter the result's rows on the server.
fn filter_bar(ui: &mut egui::Ui, doc: &mut Document, result: usize) -> Option<ResultsAction> {
    let mut action = None;
    let f = doc.filter.get_or_insert_with(Default::default);
    let p = theme::colors(ui);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon::FUNNEL).color(if f.applied.is_empty() {
            p.faint
        } else {
            p.accent
        }))
        .on_hover_text("Filter the rows: the query runs again with this condition");
        ui.label(RichText::new("WHERE").monospace().color(p.faint));
        let has_clear = !f.applied.is_empty() || f.order.is_some();
        let resp = ui.add(
            egui::TextEdit::singleline(&mut f.text)
                .id_salt(("filter", doc.id))
                .hint_text("a condition, e.g. total > 100 AND status = 'open'")
                .font(egui::TextStyle::Monospace)
                .desired_width(ui.available_width() - if has_clear { 110.0 } else { 60.0 }),
        );
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button("Apply").clicked() || enter {
            action = Some(ResultsAction::Filter(result));
        }
        if has_clear
            && ui
                .add(kit::ghost(ui, format!("{} Clear", icon::X)))
                .on_hover_text("Show the query's rows as written")
                .clicked()
        {
            action = Some(ResultsAction::ClearFilter);
        }
    });
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

/// One line about what ran: rows and timing.
fn summary(ui: &mut egui::Ui, out: &QueryOutput) {
    let p = theme::colors(ui);
    let total_ms: u64 = out.results.iter().map(|r| r.elapsed_ms).sum();
    let last = out.results.last();
    if out.results.len() > 1 || (out.error.is_some() && !out.results.is_empty()) {
        ui.label(
            RichText::new(format!(
                "{} statement{} ran",
                out.results.len(),
                if out.results.len() == 1 { "" } else { "s" }
            ))
            .color(p.muted),
        );
        ui.label(RichText::new("·").color(p.faint));
    }
    if let Some(r) = last
        && out.error.is_none()
    {
        if r.returns_rows {
            let rows = r.rows.len();
            kit::pill(
                ui,
                format!("{rows} row{}", if rows == 1 { "" } else { "s" }),
                p.muted,
            );
            if r.truncated {
                ui.label(RichText::new(format!("{} more not shown", icon::WARNING)).color(p.warn))
                    .on_hover_text("Raise the row limit to see more.");
            }
        } else if r.rows_affected > 0 {
            ui.label(
                RichText::new(format!(
                    "{} row{} affected",
                    r.rows_affected,
                    if r.rows_affected == 1 { "" } else { "s" }
                ))
                .color(p.muted),
            );
        } else {
            ui.label(RichText::new(format!("{} Done", icon::CHECK)).color(p.ok));
        }
        ui.label(RichText::new("·").color(p.faint));
    }
    if !out.results.is_empty() {
        ui.label(RichText::new(format!("{total_ms} ms")).color(p.faint));
    }
}

/// The error a run stopped on (or that it was cancelled).
fn error_box(ui: &mut egui::Ui, out: &QueryOutput) {
    let Some(e) = &out.error else { return };
    let p = theme::colors(ui);
    let color = if out.cancelled { p.warn } else { p.bad };
    ui.add_space(6.0);
    egui::Frame::new()
        .fill(theme::alpha(color, 0x14))
        .stroke(Stroke::new(1.0, theme::alpha(color, 0x66)))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let message = if out.cancelled {
                "Stopped: the query was cancelled on the server."
            } else {
                &e.message
            };
            ui.horizontal(|ui| {
                kit::dot(ui, color);
                ui.label(
                    RichText::new(message)
                        .font(egui::FontId::new(13.0, theme::semibold()))
                        .color(p.text),
                );
            });
            if !out.cancelled {
                ui.label(
                    RichText::new(format!(
                        "in statement {}: {}",
                        e.index + 1,
                        one_line(&e.sql, 160)
                    ))
                    .color(p.muted),
                );
            }
        });
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

/// The grid. Returns the column whose header was clicked.
fn table(
    ui: &mut egui::Ui,
    r: &StatementResult,
    doc_id: u64,
    grid: &mut GridEdit,
    server_order: Option<&(String, bool)>,
) -> Option<usize> {
    if r.columns.is_empty() {
        return None;
    }
    let mut clicked = None;
    // Display position to row of the result.
    let order: Vec<usize> = (0..r.rows.len()).map(|d| grid.row_at(&r.rows, d)).collect();
    let editable = grid.is_editable();
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 6.0;
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let p = theme::colors(ui);
    let null_color = p.faint;
    let changed_bg = theme::alpha(p.warn, 0x2A);
    let new_bg = theme::alpha(p.ok, 0x22);
    let deleted_bg = theme::alpha(p.bad, 0x22);
    // Numbers line up on the right.
    // (Without a type, as for SQLite expressions: when every value shown is a number.)
    let numeric: Vec<bool> = r
        .columns
        .iter()
        .enumerate()
        .map(|(k, c)| {
            if known_type(&c.type_name) {
                is_numeric(&c.type_name)
            } else {
                let mut values = r.rows.iter().take(200).filter_map(|row| row[k].as_deref());
                let mut any = false;
                values.all(|v| {
                    any = true;
                    v.parse::<f64>().is_ok()
                }) && any
            }
        })
        .collect();
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

    egui::Frame::new()
        .fill(p.bg)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(4, 2))
        .show(ui, |ui| {
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
                    let typed = r.columns.iter().any(|c| known_type(&c.type_name));
                    t.header(row_height + if typed { 14.0 } else { 4.0 }, |mut h| {
                        h.col(|_| {});
                        for (k, c) in r.columns.iter().enumerate() {
                            h.col(|ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                let arrow = match (grid.sort, server_order) {
                                    (Some((col, desc)), _) if col == k => Some(desc),
                                    (_, Some((name, desc))) if *name == c.name => Some(*desc),
                                    _ => None,
                                };
                                let text = match arrow {
                                    Some(false) => format!("{} {}", c.name, icon::CARET_UP),
                                    Some(true) => format!("{} {}", c.name, icon::CARET_DOWN),
                                    None => c.name.clone(),
                                };
                                let resp = ui
                                    .vertical(|ui| {
                                        let resp = ui.add(
                                            egui::Label::new(
                                                RichText::new(text)
                                                    .font(egui::FontId::new(
                                                        12.5,
                                                        theme::semibold(),
                                                    ))
                                                    .color(p.text),
                                            )
                                            .sense(egui::Sense::click()),
                                        );
                                        if known_type(&c.type_name) {
                                            ui.label(
                                                RichText::new(&c.type_name)
                                                    .size(10.5)
                                                    .color(p.faint),
                                            );
                                        }
                                        resp
                                    })
                                    .inner
                                    .on_hover_text(format!("{}\nClick to sort", c.type_name));
                                if resp.clicked() {
                                    clicked = Some(k);
                                }
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(row_height, total, |mut row| {
                            let shown = row.index();
                            let i = order.get(shown).copied().unwrap_or(shown);
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
                                    (shown + 1).to_string()
                                };
                                let resp = ui.add(
                                    egui::Label::new(
                                        RichText::new(label).color(p.faint).font(font.clone()),
                                    )
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
                                        Some(None) => RichText::new("NULL").color(null_color),
                                        Some(Some(s)) => RichText::new(shorten(s, CELL_CHARS).0),
                                        None => RichText::new("default").color(null_color),
                                    }
                                    .font(font.clone());
                                    if deleted {
                                        text = text.strikethrough().color(p.muted);
                                    }
                                    let label = egui::Label::new(text).sense(egui::Sense::click());
                                    let mut resp = if numeric[c] {
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| ui.add(label),
                                        )
                                        .inner
                                    } else {
                                        ui.add(label)
                                    };
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
                                                grid.edits.inserted[i - n]
                                                    .get(&k)
                                                    .cloned()
                                                    .flatten()
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
                                                let current =
                                                    value.cloned().flatten().unwrap_or_default();
                                                start_edit = Some((i, c, current));
                                            }
                                            if ui.button("Set to NULL").clicked() {
                                                set = Some((i, c, None));
                                            }
                                            if edited.is_some()
                                                && ui.button("Undo this change").clicked()
                                            {
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
                                            ui.ctx().copy_text(
                                                value.cloned().flatten().unwrap_or_default(),
                                            );
                                        }
                                        if ui.button("Copy row").clicked() {
                                            ui.ctx().copy_text(tsv_row(&values));
                                        }
                                        if ui.button("Copy all rows (with headers)").clicked() {
                                            ui.ctx().copy_text(tsv_all(r));
                                        }
                                        if ui
                                            .button(format!(
                                                "Copy column name ({})",
                                                r.columns[c].name
                                            ))
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
    clicked
}

/// Whether the database said what a column holds (SQLite expressions have no type).
fn known_type(type_name: &str) -> bool {
    !type_name.is_empty() && !type_name.eq_ignore_ascii_case("null")
}

/// Whether a column holds numbers (they're shown on the right).
fn is_numeric(type_name: &str) -> bool {
    let t = type_name.to_ascii_lowercase();
    let t = t
        .trim_start_matches("nullable(")
        .trim_start_matches("lowcardinality(");
    [
        "int",
        "integer",
        "bigint",
        "smallint",
        "tinyint",
        "mediumint",
        "int2",
        "int4",
        "int8",
        "serial",
        "bigserial",
        "numeric",
        "decimal",
        "real",
        "float",
        "float4",
        "float8",
        "double",
        "double precision",
        "money",
        "number",
        "uint",
        "hugeint",
        "ubigint",
    ]
    .iter()
    .any(|n| t == *n || t.starts_with(&format!("{n}(")) || t.starts_with(&format!("{n} ")))
        || t.starts_with("int") && t[3..].chars().all(|c| c.is_ascii_digit())
        || t.starts_with("uint") && t[4..].chars().all(|c| c.is_ascii_digit())
        || t.starts_with("float") && t[5..].chars().all(|c| c.is_ascii_digit())
        || t.starts_with("decimal")
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
