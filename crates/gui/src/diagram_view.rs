//! A schema's entity-relationship diagram in a window: tables as boxes, foreign keys
//! as lines between their columns. Drag a table to move it, drag the background to
//! pan, Ctrl+scroll (or pinch) to zoom. Saves as SVG or copies as Mermaid.

use std::collections::HashMap;
use std::sync::Arc;

use datadrop_core::schema::TableModel;
use datadrop_core::{Command, Output, SchemaModel, diagram};
use egui::{Color32, Pos2, Rect, Stroke, Vec2, pos2, vec2};
use egui_phosphor::regular as icon;

use crate::backend::{Backend, Task, poll};

const HEADER: f32 = 26.0;
const ROW: f32 = 19.0;
const FONT: f32 = 12.5;
/// Monospace character width at `FONT`, for sizing boxes.
const CHAR: f32 = FONT * 0.6;
const GAP_X: f32 = 110.0;
const GAP_Y: f32 = 36.0;

pub struct DiagramView {
    pub profile: String,
    pub schema: String,
    pub open: bool,
    load: Option<Task>,
    model: Option<Arc<SchemaModel>>,
    error: Option<String>,
    /// Each table's top-left corner, in diagram units.
    pos: HashMap<String, Pos2>,
    pan: Vec2,
    zoom: f32,
    dragging: Option<String>,
    /// Fit the view to the diagram on the next frame.
    fit: bool,
}

struct Layout {
    rects: HashMap<String, Rect>,
}

fn box_size(t: &TableModel) -> Vec2 {
    let widest = t
        .columns
        .iter()
        .map(|c| c.name.chars().count() + c.data_type.chars().count() + 7)
        .chain([t.name.chars().count() + 4])
        .max()
        .unwrap_or(10)
        .min(60);
    vec2(
        widest as f32 * CHAR + 20.0,
        HEADER + t.columns.len() as f32 * ROW + 6.0,
    )
}

impl DiagramView {
    pub fn new(backend: &Backend, profile: String, schema: String) -> Self {
        Self {
            load: Some(backend.run(Command::Schema {
                profile: profile.clone(),
                schema: Some(schema.clone()),
            })),
            profile,
            schema,
            open: true,
            model: None,
            error: None,
            pos: HashMap::new(),
            pan: Vec2::ZERO,
            zoom: 1.0,
            dragging: None,
            fit: true,
        }
    }

    /// Place every table: columns from the core layout, stacked top to bottom.
    fn auto_layout(&mut self) {
        let Some(m) = &self.model else { return };
        self.pos.clear();
        let mut x = 0.0;
        for col in diagram::columns(m) {
            let mut y = 0.0;
            let mut width: f32 = 0.0;
            for t in col {
                let size = box_size(t);
                self.pos.insert(t.name.clone(), pos2(x, y));
                y += size.y + GAP_Y;
                width = width.max(size.x);
            }
            x += width + GAP_X;
        }
        self.fit = true;
    }

    fn layout(&self) -> Layout {
        let mut rects = HashMap::new();
        if let Some(m) = &self.model {
            for t in &m.tables {
                let p = self.pos.get(&t.name).copied().unwrap_or_default();
                rects.insert(t.name.clone(), Rect::from_min_size(p, box_size(t)));
            }
        }
        Layout { rects }
    }

    pub fn ui(&mut self, ctx: &egui::Context, toasts: &mut egui_notify::Toasts) {
        if let Some(r) = poll(&mut self.load) {
            match r {
                Ok(Output::Schema(m)) => {
                    self.model = Some(Arc::new(m));
                    self.auto_layout();
                }
                Ok(_) => {}
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        let mut open = self.open;
        egui::Window::new(format!(
            "{}  {} · {}",
            icon::GRAPH,
            self.schema,
            self.profile
        ))
        .id(egui::Id::new(("diagram", &self.profile, &self.schema)))
        .open(&mut open)
        .default_size([960.0, 620.0])
        .resizable(true)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button(format!("{} Fit", icon::ARROWS_OUT)).clicked() {
                    self.fit = true;
                }
                if ui
                    .button(format!("{} Tidy", icon::SQUARES_FOUR))
                    .on_hover_text("Lay the tables out again")
                    .clicked()
                {
                    self.auto_layout();
                }
                ui.separator();
                if ui
                    .button(format!("{} Copy as Mermaid", icon::COPY))
                    .clicked()
                    && let Some(m) = &self.model
                {
                    ui.ctx().copy_text(diagram::mermaid(m));
                    toasts.info("Copied: paste it into Markdown in a ```mermaid block");
                }
                if ui
                    .button(format!("{} Save as SVG…", icon::DOWNLOAD_SIMPLE))
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("SVG", &["svg"])
                        .set_file_name(format!("{}.svg", self.schema))
                        .save_file()
                {
                    match std::fs::write(&path, self.svg()) {
                        Ok(()) => {
                            toasts.success(format!("Saved {}", path.display()));
                        }
                        Err(e) => {
                            toasts.error(format!("Couldn't save: {e}"));
                        }
                    }
                }
                ui.weak("Drag tables to move them, the background to pan; Ctrl+scroll to zoom");
            });
            ui.separator();
            if let Some(e) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
                return;
            }
            if self.model.is_none() {
                ui.spinner();
                return;
            }
            self.canvas(ui);
        });
        self.open = open;
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let area = resp.rect;
        let layout = self.layout();
        if self.fit {
            self.fit = false;
            let bounds = layout
                .rects
                .values()
                .fold(Rect::NOTHING, |a, r| a.union(*r));
            if bounds.is_positive() {
                let z = (area.width() / (bounds.width() + 40.0))
                    .min(area.height() / (bounds.height() + 40.0))
                    .clamp(0.2, 1.5);
                self.zoom = z;
                self.pan =
                    area.center().to_vec2() - bounds.center().to_vec2() * z - area.min.to_vec2();
            }
        }
        // The view before this frame's input.
        let (z0, origin0) = (self.zoom, area.min.to_vec2() + self.pan);
        let to_diagram = move |p: Pos2| -> Pos2 { ((p.to_vec2() - origin0) / z0).to_pos2() };

        // Zoom around the pointer; drag tables or the view.
        if resp.hovered() {
            let (zoom, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
            if zoom != 1.0
                && let Some(p) = ui.input(|i| i.pointer.hover_pos())
            {
                let before = to_diagram(p);
                self.zoom = (self.zoom * zoom).clamp(0.15, 3.0);
                let after =
                    (before.to_vec2() * self.zoom + area.min.to_vec2() + self.pan).to_pos2();
                self.pan += p - after;
            } else if scroll != Vec2::ZERO {
                self.pan += scroll;
            }
        }
        if resp.drag_started()
            && let Some(p) = resp.interact_pointer_pos()
        {
            let d = to_diagram(p);
            self.dragging = layout
                .rects
                .iter()
                .find(|(_, r)| r.contains(d))
                .map(|(n, _)| n.clone());
        }
        if resp.dragged() {
            let delta = resp.drag_delta();
            match &self.dragging {
                Some(name) => {
                    if let Some(p) = self.pos.get_mut(name) {
                        *p += delta / self.zoom;
                    }
                }
                None => self.pan += delta,
            }
        }
        if resp.drag_stopped() {
            self.dragging = None;
        }
        let layout = self.layout();
        let Some(m) = self.model.clone() else { return };
        let (zoom, origin) = (self.zoom, area.min.to_vec2() + self.pan);
        let to_screen = move |p: Pos2| -> Pos2 { (p.to_vec2() * zoom + origin).to_pos2() };

        let v = ui.visuals();
        let fill = v.extreme_bg_color;
        let header = v.selection.bg_fill.gamma_multiply(0.35);
        let border = v.widgets.noninteractive.bg_stroke.color;
        let text = v.text_color();
        let weak = v.weak_text_color();
        let line = v.selection.stroke.color.gamma_multiply(0.8);
        let font = egui::FontId::monospace(FONT * self.zoom);
        let bold = egui::FontId::proportional((FONT + 1.0) * self.zoom);
        let painter = painter.with_clip_rect(area);

        // Lines first, under the boxes.
        for t in &m.tables {
            let Some(from) = layout.rects.get(&t.name) else {
                continue;
            };
            for fk in t.foreign_keys() {
                let Some(r) = &fk.references else { continue };
                let Some(to) = layout.rects.get(&r.table) else {
                    continue;
                };
                let target = m.table(&r.table);
                let row_y = |table: &TableModel, rect: &Rect, col: Option<&String>| -> f32 {
                    let i = col
                        .and_then(|c| table.columns.iter().position(|x| &x.name == c))
                        .unwrap_or(0);
                    rect.min.y + HEADER + (i as f32 + 0.5) * ROW
                };
                let y1 = row_y(t, from, fk.columns.first());
                let y2 = target.map_or(to.center().y, |tt| {
                    row_y(
                        tt,
                        to,
                        r.columns
                            .first()
                            .or(tt.primary_key().and_then(|k| k.columns.first())),
                    )
                });
                // From whichever sides face each other.
                let (x1, x2, dir) = if to.center().x < from.center().x {
                    (from.min.x, to.max.x, -1.0)
                } else {
                    (from.max.x, to.min.x, 1.0)
                };
                let a = to_screen(pos2(x1, y1));
                let b = to_screen(pos2(x2, y2));
                let bend = (GAP_X * 0.5 * self.zoom).max((b.x - a.x).abs() * 0.4);
                let curve = egui::epaint::CubicBezierShape::from_points_stroke(
                    [a, a + vec2(dir * bend, 0.0), b - vec2(dir * bend, 0.0), b],
                    false,
                    Color32::TRANSPARENT,
                    Stroke::new(1.4 * self.zoom.max(0.6), line),
                );
                painter.add(curve);
                // Many (a fork) at the referring end, one (a bar) at the other.
                let s = 7.0 * self.zoom;
                for dy in [-s * 0.6, 0.0, s * 0.6] {
                    painter.line_segment(
                        [a + vec2(dir * s, 0.0), a + vec2(0.0, dy)],
                        Stroke::new(1.2, line),
                    );
                }
                painter.line_segment(
                    [
                        b - vec2(dir * s * 0.8, s * 0.6),
                        b - vec2(dir * s * 0.8, -s * 0.6),
                    ],
                    Stroke::new(1.4, line),
                );
            }
        }

        for t in &m.tables {
            let Some(r) = layout.rects.get(&t.name) else {
                continue;
            };
            let rect = Rect::from_min_max(to_screen(r.min), to_screen(r.max));
            if !rect.intersects(area) {
                continue;
            }
            let radius = 5.0 * self.zoom;
            painter.rect(
                rect,
                radius,
                fill,
                Stroke::new(1.0, if t.is_view() { weak } else { border }),
                egui::StrokeKind::Inside,
            );
            let head = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER * self.zoom));
            painter.rect_filled(
                head,
                egui::CornerRadius {
                    nw: radius as u8,
                    ne: radius as u8,
                    sw: 0,
                    se: 0,
                },
                header,
            );
            let glyph = if t.is_view() { icon::EYE } else { icon::TABLE };
            painter.text(
                head.left_center() + vec2(8.0 * self.zoom, 0.0),
                egui::Align2::LEFT_CENTER,
                format!("{glyph} {}", t.name),
                bold.clone(),
                text,
            );
            let pk: &[String] = t.primary_key().map_or(&[], |k| &k.columns);
            let fks: Vec<&String> = t.foreign_keys().flat_map(|f| &f.columns).collect();
            for (i, c) in t.columns.iter().enumerate() {
                let y = rect.min.y + (HEADER + (i as f32 + 0.5) * ROW) * self.zoom;
                let mark = if pk.contains(&c.name) {
                    Some(icon::KEY)
                } else if fks.contains(&&c.name) {
                    Some(icon::LINK)
                } else {
                    None
                };
                // The icon font is only in the proportional family.
                if let Some(mark) = mark {
                    painter.text(
                        pos2(rect.min.x + 6.0 * self.zoom, y),
                        egui::Align2::LEFT_CENTER,
                        mark,
                        egui::FontId::proportional(FONT * self.zoom),
                        weak,
                    );
                }
                painter.text(
                    pos2(rect.min.x + 22.0 * self.zoom, y),
                    egui::Align2::LEFT_CENTER,
                    &c.name,
                    font.clone(),
                    text,
                );
                painter.text(
                    pos2(rect.max.x - 8.0 * self.zoom, y),
                    egui::Align2::RIGHT_CENTER,
                    &c.data_type,
                    font.clone(),
                    weak,
                );
            }
        }
    }

    /// The diagram as it's laid out now, as SVG.
    pub fn svg(&self) -> String {
        let Some(m) = &self.model else {
            return String::new();
        };
        let layout = self.layout();
        let bounds = layout
            .rects
            .values()
            .fold(Rect::NOTHING, |a, r| a.union(*r))
            .expand(20.0);
        let esc = |s: &str| {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
        };
        let mut s = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\" \
             font-family=\"Menlo, Consolas, monospace\" font-size=\"{FONT}\">\n\
             <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#ffffff\"/>\n",
            bounds.min.x,
            bounds.min.y,
            bounds.width(),
            bounds.height(),
            bounds.min.x,
            bounds.min.y,
            bounds.width(),
            bounds.height()
        );
        for t in &m.tables {
            let Some(from) = layout.rects.get(&t.name) else {
                continue;
            };
            for fk in t.foreign_keys() {
                let Some(r) = &fk.references else { continue };
                let Some(to) = layout.rects.get(&r.table) else {
                    continue;
                };
                let i = fk
                    .columns
                    .first()
                    .and_then(|c| t.columns.iter().position(|x| &x.name == c))
                    .unwrap_or(0);
                let y1 = from.min.y + HEADER + (i as f32 + 0.5) * ROW;
                let y2 = to.min.y + HEADER + 0.5 * ROW;
                let (x1, x2, dir) = if to.center().x < from.center().x {
                    (from.min.x, to.max.x, -1.0)
                } else {
                    (from.max.x, to.min.x, 1.0)
                };
                let bend = (GAP_X * 0.5).max((x2 - x1).abs() * 0.4);
                s.push_str(&format!(
                    "<path d=\"M{x1} {y1} C{} {y1} {} {y2} {x2} {y2}\" fill=\"none\" \
                     stroke=\"#5b7ba8\" stroke-width=\"1.4\"/>\n",
                    x1 + dir * bend,
                    x2 - dir * bend
                ));
            }
        }
        for t in &m.tables {
            let Some(r) = layout.rects.get(&t.name) else {
                continue;
            };
            s.push_str(&format!(
                "<g><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"5\" fill=\"#ffffff\" \
                 stroke=\"#9aa5b1\"/>\n<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{HEADER}\" \
                 rx=\"5\" fill=\"#dde6f0\"/>\n<text x=\"{}\" y=\"{}\" font-family=\"Helvetica, \
                 Arial, sans-serif\" font-weight=\"bold\">{}</text>\n",
                r.min.x,
                r.min.y,
                r.width(),
                r.height(),
                r.min.x,
                r.min.y,
                r.width(),
                r.min.x + 8.0,
                r.min.y + HEADER * 0.65,
                esc(&t.name)
            ));
            let pk: &[String] = t.primary_key().map_or(&[], |k| &k.columns);
            for (i, c) in t.columns.iter().enumerate() {
                let y = r.min.y + HEADER + (i as f32 + 0.72) * ROW;
                s.push_str(&format!(
                    "<text x=\"{}\" y=\"{y}\"{}>{}</text><text x=\"{}\" y=\"{y}\" \
                     text-anchor=\"end\" fill=\"#6b7480\">{}</text>\n",
                    r.min.x + 8.0,
                    if pk.contains(&c.name) {
                        " font-weight=\"bold\""
                    } else {
                        ""
                    },
                    esc(&c.name),
                    r.max.x - 8.0,
                    esc(&c.data_type)
                ));
            }
            s.push_str("</g>\n");
        }
        s.push_str("</svg>\n");
        s
    }

    #[cfg(test)]
    pub fn loaded(&self) -> bool {
        self.model.is_some()
    }
}
