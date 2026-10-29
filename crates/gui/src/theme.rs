//! Fonts, icons, colours.

use datadrop_core::sql::TokenKind;
use egui::Color32;

use crate::settings::Theme;

pub const ACCENT: Color32 = Color32::from_rgb(0x3B, 0x9E, 0xF5);
/// Readable on light backgrounds too.
pub const ACCENT_DEEP: Color32 = Color32::from_rgb(0x1B, 0x6F, 0xC2);

pub fn install(ctx: &egui::Context, theme: Theme) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
    apply_theme(ctx, theme);
    for (t, accent) in [
        (egui::Theme::Dark, ACCENT),
        (egui::Theme::Light, ACCENT_DEEP),
    ] {
        ctx.style_mut_of(t, |style| {
            style.spacing.item_spacing = egui::vec2(8.0, 5.0);
            // An app, not a document: clicks on labels select rows, not words.
            style.interaction.selectable_labels = false;
            style.spacing.button_padding = egui::vec2(7.0, 3.0);
            let v = &mut style.visuals;
            v.hyperlink_color = accent;
            v.selection.bg_fill = accent.gamma_multiply(0.35);
            v.selection.stroke.color = accent;
        });
    }
}

pub fn apply_theme(ctx: &egui::Context, theme: Theme) {
    ctx.set_theme(match theme {
        Theme::System => egui::ThemePreference::System,
        Theme::Light => egui::ThemePreference::Light,
        Theme::Dark => egui::ThemePreference::Dark,
    });
}

/// Syntax colours, for the current light or dark theme.
pub fn token_color(kind: TokenKind, dark: bool) -> Color32 {
    use TokenKind::*;
    let (d, l) = match kind {
        Keyword => ((0xC6, 0x8C, 0xF5), (0x8A, 0x2B, 0xC9)),
        String => ((0x9B, 0xD1, 0x7B), (0x23, 0x7A, 0x1F)),
        Number => ((0xF0, 0xA4, 0x5D), (0xB3, 0x56, 0x00)),
        Comment => ((0x7F, 0x88, 0x94), (0x7A, 0x80, 0x88)),
        QuotedIdentifier => ((0x6C, 0xC4, 0xD8), (0x0E, 0x6E, 0x86)),
        Parameter => ((0xF0, 0x7C, 0x9A), (0xB0, 0x24, 0x52)),
        Operator | Punctuation => ((0xB8, 0xC0, 0xCC), (0x50, 0x58, 0x66)),
        Identifier | Whitespace => ((0xDD, 0xE1, 0xE6), (0x1F, 0x23, 0x28)),
    };
    let (r, g, b) = if dark { d } else { l };
    Color32::from_rgb(r, g, b)
}

/// Background for find matches (the current one is stronger).
pub fn match_bg(current: bool, dark: bool) -> Color32 {
    match (current, dark) {
        (true, true) => Color32::from_rgb(0x8A, 0x6D, 0x00),
        (false, true) => Color32::from_rgb(0x4A, 0x40, 0x10),
        (true, false) => Color32::from_rgb(0xFF, 0xC8, 0x3D),
        (false, false) => Color32::from_rgb(0xFF, 0xEE, 0xA8),
    }
}

/// The window icon (packaging/make-icon.py draws it).
pub fn icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../../../packaging/icons/256.png"))
        .expect("the icon is a valid PNG")
}
