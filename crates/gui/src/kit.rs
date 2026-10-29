//! The parts the screens share: buttons, pills, badges, banners and the frame of a
//! dialog. They follow the design in `design/0.3`.

use datadrop_core::Driver;
use egui::{Color32, CornerRadius, FontId, Margin, Response, RichText, Sense, Stroke, Ui};

use crate::theme::{self, alpha, colors};

/// The one main action of a screen or dialog: filled with the accent.
pub fn primary(ui: &Ui, text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).color(Color32::WHITE))
        .fill(colors(ui).accent)
        .stroke(Stroke::NONE)
}

/// An action that throws something away: red text.
pub fn danger(ui: &Ui, text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).color(colors(ui).bad))
}

/// A quiet action: no frame until it's hovered.
pub fn ghost(ui: &Ui, text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).color(colors(ui).muted)).frame_when_inactive(false)
}

/// A small, spaced-out caption over a group: "PENDING CHANGES".
pub fn caps(ui: &mut Ui, text: &str) -> Response {
    ui.label(
        RichText::new(text.to_uppercase())
            .font(FontId::new(10.5, theme::semibold()))
            .color(colors(ui).faint)
            .extra_letter_spacing(0.6),
    )
}

/// Text in a tinted, rounded pill: a count, a key, a state.
pub fn pill(ui: &mut Ui, text: impl Into<String>, color: Color32) -> Response {
    let galley =
        ui.painter()
            .layout_no_wrap(text.into(), FontId::new(11.0, theme::medium()), color);
    let size = egui::vec2(galley.size().x + 14.0, 18.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(9), alpha(color, 0x24));
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
    resp
}

/// Each driver's colour and two letters.
pub fn driver_style(driver: Driver, dark: bool) -> (Color32, &'static str) {
    let (hex, light_hex, letters) = match driver {
        Driver::Postgres => (0x5B8DEF, 0x3D6FD6, "Pg"),
        Driver::Mysql => (0xE48E2C, 0xC2700F, "My"),
        Driver::Sqlite => (0x5FA8D3, 0x2F7FAF, "Sq"),
        Driver::SqlServer => (0xE0525A, 0xC23A42, "Ms"),
        Driver::ClickHouse => (0xE7C53A, 0x9A7B00, "Ch"),
        Driver::DuckDb => (0xF2C94C, 0x9A7300, "Dk"),
    };
    let hex = if dark { hex } else { light_hex };
    (
        Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8),
        letters,
    )
}

/// The driver's badge: its two letters on a tinted square.
pub fn driver_badge(ui: &mut Ui, driver: Driver, size: f32) -> Response {
    let (color, letters) = driver_style(driver, ui.visuals().dark_mode);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius::same((size * 0.28) as u8),
        alpha(color, 0x30),
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        letters,
        FontId::new(size * 0.5, theme::semibold()),
        color,
    );
    resp.on_hover_text(driver.label())
}

/// Two choices side by side, the chosen one raised. Returns the one clicked, if it's
/// not the current one. Each half is a button labelled with its text.
pub fn segmented(ui: &mut Ui, first: bool, labels: [&str; 2]) -> Option<bool> {
    let p = colors(ui);
    let mut picked = None;
    egui::Frame::new()
        .fill(p.bg)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (i, label) in labels.into_iter().enumerate() {
                    let on = (i == 0) == first;
                    let text = RichText::new(label).color(if on { p.text } else { p.muted });
                    let b = if on {
                        egui::Button::new(text)
                            .fill(p.raised)
                            .stroke(Stroke::new(1.0, p.border2))
                    } else {
                        egui::Button::new(text).frame_when_inactive(false)
                    };
                    if ui.add(b.corner_radius(CornerRadius::same(5))).clicked() && !on {
                        picked = Some(i == 0);
                    }
                }
            });
        });
    picked
}

/// A full-width strip tinted with `color`: a transaction, an edit mode, a warning.
pub fn banner<R>(ui: &mut Ui, color: Color32, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(alpha(color, 0x18))
        .stroke(Stroke::new(1.0, alpha(color, 0x55)))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                dot(ui, color);
                add(ui)
            })
            .inner
        })
        .inner
}

/// A small filled circle: a state.
pub fn dot(ui: &mut Ui, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
    resp
}

/// A dialog's title, and the line under it saying what it's for.
pub fn sheet_header(ui: &mut Ui, title: &str, subtitle: Option<&str>) {
    ui.label(
        RichText::new(title)
            .font(FontId::new(18.0, theme::semibold()))
            .color(colors(ui).text),
    );
    if let Some(s) = subtitle {
        ui.add_space(2.0);
        ui.label(RichText::new(s).color(colors(ui).muted));
    }
    ui.add_space(12.0);
}

/// The bottom of a dialog: a rule, then its buttons laid out right to left (add the
/// main action first, so it lands bottom right).
pub fn sheet_footer<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.add_space(14.0);
    let p = colors(ui);
    let y = ui.cursor().top();
    let rect = ui.max_rect();
    ui.painter().hline(
        (rect.left() - 20.0)..=(rect.right() + 20.0),
        y,
        Stroke::new(1.0, p.border),
    );
    ui.add_space(14.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add)
        .inner
}

/// A rounded card in the window colour: a group of fields, a code excerpt.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = colors(ui);
    egui::Frame::new()
        .fill(p.bg)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(12))
        .show(ui, add)
        .inner
}

/// The look of an `egui::Modal`: the panel colour, rounded, with a shadow.
pub fn modal_frame(ctx: &egui::Context) -> egui::Frame {
    let dark = ctx.global_style().visuals.dark_mode;
    let p = theme::palette(dark);
    egui::Frame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0, p.border2))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(24))
        .shadow(ctx.global_style().visuals.window_shadow)
}

/// The dimmed window behind a dialog.
pub fn backdrop(ctx: &egui::Context) -> Color32 {
    if ctx.global_style().visuals.dark_mode {
        alpha(Color32::from_rgb(5, 7, 10), 0xB3)
    } else {
        alpha(Color32::from_rgb(15, 23, 42), 0x66)
    }
}

pub trait ModalExt {
    /// The design's frame and backdrop.
    fn styled(self, ctx: &egui::Context) -> Self;
}

impl ModalExt for egui::Modal {
    fn styled(self, ctx: &egui::Context) -> Self {
        self.frame(modal_frame(ctx)).backdrop_color(backdrop(ctx))
    }
}
