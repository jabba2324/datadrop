//! The query plan (Explain) in the results pane: a tree of steps, with where the
//! time went when it was analyzed.

use std::collections::HashSet;

use datadrop_core::{Plan, PlanNode};
use egui::RichText;
use egui_extras::{Column, TableBuilder};
use egui_phosphor::regular as icon;

use crate::{kit, theme};

/// A step, flattened for the table.
struct Row<'a> {
    node: &'a PlanNode,
    depth: usize,
    path: String,
    has_children: bool,
}

fn flatten<'a>(
    nodes: &'a [PlanNode],
    depth: usize,
    prefix: &str,
    collapsed: &HashSet<String>,
    out: &mut Vec<Row<'a>>,
) {
    for (i, n) in nodes.iter().enumerate() {
        let path = format!("{prefix}{i}/");
        out.push(Row {
            node: n,
            depth,
            path: path.clone(),
            has_children: !n.children.is_empty(),
        });
        if !collapsed.contains(&path) {
            flatten(&n.children, depth + 1, &path, collapsed, out);
        }
    }
}

fn num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else if v >= 100.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.2}")
    }
}

pub fn plan_ui(ui: &mut egui::Ui, plan: &Plan, collapsed: &mut HashSet<String>, id: u64) {
    let p = theme::colors(ui);
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        if plan.analyzed {
            kit::pill(ui, format!("{} Query plan, measured", icon::TIMER), p.ok);
        } else {
            kit::pill(
                ui,
                format!("{} Query plan (estimates)", icon::TREE_STRUCTURE),
                p.muted,
            );
        }
        if let Some(ms) = plan.planning_ms {
            ui.label(RichText::new(format!("· planning {} ms", num(ms))).color(p.muted));
        }
        if let Some(ms) = plan.execution_ms {
            ui.label(RichText::new(format!("· execution {} ms", num(ms))).color(p.muted));
        }
        if ui
            .add(kit::ghost(ui, format!("{} Copy", icon::COPY)))
            .on_hover_text("Copy the plan as the database returned it")
            .clicked()
        {
            ui.ctx().copy_text(plan.raw.clone());
        }
    });
    ui.add_space(6.0);

    let total_ms = plan.execution_ms.or_else(|| {
        plan.roots
            .iter()
            .filter_map(|r| r.actual_ms)
            .reduce(f64::max)
    });
    let mut rows = vec![];
    flatten(&plan.roots, 0, "", collapsed, &mut rows);
    let analyzed = plan.analyzed;
    let has_cost = rows.iter().any(|r| r.node.estimated_cost.is_some());
    let has_estimate = rows.iter().any(|r| r.node.estimated_rows.is_some());
    let row_height = ui.text_style_height(&egui::TextStyle::Body) + 14.0;
    let hot = p.warn;
    let mut toggle = None;
    let head = |ui: &mut egui::Ui, text: &str| {
        ui.label(
            RichText::new(text)
                .font(egui::FontId::new(12.0, theme::semibold()))
                .color(p.muted),
        )
    };

    egui::Frame::new()
        .fill(p.bg)
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            let mut t = TableBuilder::new(ui)
                .id_salt(("plan", id))
                .striped(true)
                .resizable(true)
                .auto_shrink(false)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::remainder().at_least(240.0).clip(true));
            if has_estimate {
                t = t.column(Column::auto().at_least(70.0));
            }
            if analyzed {
                t = t
                    .column(Column::auto().at_least(70.0))
                    .column(Column::auto().at_least(140.0));
            }
            if has_cost {
                t = t.column(Column::auto().at_least(70.0));
            }
            t.header(row_height, |mut h| {
                h.col(|ui| {
                    head(ui, "Step");
                });
                if has_estimate {
                    h.col(|ui| {
                        head(ui, "Est. rows").on_hover_text("How many rows the planner expected");
                    });
                }
                if analyzed {
                    h.col(|ui| {
                        head(ui, "Rows").on_hover_text("Rows produced (per loop)");
                    });
                    h.col(|ui| {
                        head(ui, "Time (ms)").on_hover_text(
                    "Time in this step and the ones under it; the bar is this step's own share",
                );
                    });
                }
                if has_cost {
                    h.col(|ui| {
                        head(ui, "Cost").on_hover_text("The planner's estimate, in its own units");
                    });
                }
            })
            .body(|body| {
                body.rows(row_height, rows.len(), |mut row| {
                    let r = &rows[row.index()];
                    let n = r.node;
                    let share = match (n.own_ms(), total_ms) {
                        (Some(own), Some(total)) if total > 0.0 => {
                            (own / total).clamp(0.0, 1.0) as f32
                        }
                        _ => 0.0,
                    };
                    let slow = share >= 0.3;
                    row.col(|ui| {
                        if slow {
                            ui.painter()
                                .rect_filled(ui.max_rect(), 0.0, theme::alpha(hot, 0x14));
                            let r = ui.max_rect();
                            ui.painter().vline(
                                r.left() + 1.0,
                                r.y_range(),
                                egui::Stroke::new(2.0, hot),
                            );
                        }
                        ui.add_space(r.depth as f32 * 18.0);
                        if r.has_children {
                            let open = !collapsed.contains(&r.path);
                            let arrow = if open {
                                icon::CARET_DOWN
                            } else {
                                icon::CARET_RIGHT
                            };
                            if ui
                                .add(
                                    egui::Button::new(RichText::new(arrow).color(p.muted))
                                        .frame_when_inactive(false),
                                )
                                .clicked()
                            {
                                toggle = Some(r.path.clone());
                            }
                        } else {
                            ui.add_space(20.0);
                        }
                        let mut text = RichText::new(&n.label)
                            .font(egui::FontId::new(13.0, theme::medium()))
                            .color(p.text);
                        if slow {
                            text = text.color(hot);
                        }
                        let resp = ui.label(text);
                        let mut tip = n.label.clone();
                        for (k, v) in &n.details {
                            tip.push_str(&format!("\n{k}: {v}"));
                        }
                        if let Some(l) = n.loops.filter(|l| *l > 1.0) {
                            tip.push_str(&format!("\nLoops: {}", num(l)));
                        }
                        resp.on_hover_text(tip);
                        if let Some((_, v)) = n
                            .details
                            .iter()
                            .find(|(_, v)| !n.label.contains(v.as_str()))
                        {
                            ui.label(RichText::new(v).monospace().color(p.muted));
                        }
                    });
                    if has_estimate {
                        row.col(|ui| {
                            ui.label(
                                RichText::new(n.estimated_rows.map(num).unwrap_or_default())
                                    .monospace(),
                            );
                        });
                    }
                    if analyzed {
                        row.col(|ui| {
                            let mut t = RichText::new(n.actual_rows.map(num).unwrap_or_default())
                                .monospace();
                            // A far-off estimate is often why a plan is slow.
                            if let (Some(a), Some(e)) = (n.actual_rows, n.estimated_rows)
                                && (a > e * 10.0 + 10.0 || e > a * 10.0 + 10.0)
                            {
                                t = t.color(hot);
                            }
                            ui.label(t).on_hover_text(
                                "Highlighted when it's far from the estimate, which often means \
                         out-of-date statistics",
                            );
                        });
                        row.col(|ui| {
                            // This step's own share of the time.
                            let (rect, _) =
                                ui.allocate_exact_size(egui::vec2(90.0, 6.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 3, p.hover);
                            let mut fill = rect;
                            fill.set_width(rect.width() * share.max(0.02));
                            ui.painter()
                                .rect_filled(fill, 3, if slow { hot } else { p.accent });
                            ui.label(
                                RichText::new(n.actual_ms.map(num).unwrap_or_default()).monospace(),
                            );
                        });
                    }
                    if has_cost {
                        row.col(|ui| {
                            ui.label(
                                RichText::new(n.estimated_cost.map(num).unwrap_or_default())
                                    .monospace()
                                    .color(p.muted),
                            );
                        });
                    }
                });
            });
        });
    if let Some(p) = toggle
        && !collapsed.remove(&p)
    {
        collapsed.insert(p);
    }
}
