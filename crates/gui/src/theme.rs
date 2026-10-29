//! Fonts, colours and the look of every widget: the design tokens of
//! `design/0.3` (`ddkit.py`'s DARK and LIGHT), set on egui's visuals.

use std::sync::Arc;

use datadrop_core::sql::TokenKind;
use egui::{
    Color32, CornerRadius, FontData, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle,
};

use crate::settings::Theme;

/// The design's colours, for one theme.
#[derive(Clone, Copy)]
pub struct Palette {
    /// The window: editor, grid, text fields.
    pub bg: Color32,
    /// Bars and side panels.
    pub panel: Color32,
    /// Buttons, cards and menus.
    pub raised: Color32,
    pub hover: Color32,
    pub border: Color32,
    /// Borders of controls.
    pub border2: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    /// The accent, faintly: selected rows and items.
    pub accent_soft: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub bad: Color32,
    pub purple: Color32,
    pub amber: Color32,
    pub teal: Color32,
    /// Every other grid row.
    pub zebra: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// `color` at `alpha` (0-255).
pub fn alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            bg: rgb(0x0D1015),
            panel: rgb(0x12161C),
            raised: rgb(0x181D25),
            hover: rgb(0x1E242E),
            border: rgb(0x232A35),
            border2: rgb(0x2C3441),
            text: rgb(0xE6EAF0),
            muted: rgb(0x8A94A4),
            faint: rgb(0x5B6575),
            accent: rgb(0x3B9EF5),
            accent_soft: alpha(rgb(0x3B9EF5), 0x24),
            ok: rgb(0x3FB97A),
            warn: rgb(0xE3A945),
            bad: rgb(0xF06A6A),
            purple: rgb(0xA78BFA),
            amber: rgb(0xF0A45D),
            teal: rgb(0x4FC1D6),
            zebra: alpha(Color32::WHITE, 5),
        }
    } else {
        Palette {
            bg: rgb(0xF6F7F9),
            panel: rgb(0xFFFFFF),
            raised: rgb(0xFFFFFF),
            hover: rgb(0xEEF1F5),
            border: rgb(0xE3E7ED),
            border2: rgb(0xD5DBE3),
            text: rgb(0x1A1F27),
            muted: rgb(0x5F6877),
            faint: rgb(0x97A0AD),
            accent: rgb(0x1B6FC2),
            accent_soft: alpha(rgb(0x1B6FC2), 0x1A),
            ok: rgb(0x1F9D5C),
            warn: rgb(0xB7791F),
            bad: rgb(0xD14343),
            purple: rgb(0x7C4DDB),
            amber: rgb(0xC2620A),
            teal: rgb(0x0E7C93),
            zebra: alpha(rgb(0x0F172A), 5),
        }
    }
}

/// The palette for what `ui` shows.
pub fn colors(ui: &egui::Ui) -> Palette {
    palette(ui.visuals().dark_mode)
}

/// Inter, three weights: the UI. JetBrains Mono: SQL, values and numbers.
const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
const MONO: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");

/// Labels on buttons, tabs and headings.
pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

/// Headings and titles.
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

fn fonts() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    for (name, bytes) in [
        ("Inter", INTER),
        ("Inter Medium", INTER_MEDIUM),
        ("Inter SemiBold", INTER_SEMIBOLD),
        ("JetBrains Mono", MONO),
    ] {
        fonts
            .font_data
            .insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    // egui's own fonts (and the icons) stay as fallbacks for symbols these don't have.
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .expect("egui defines it")
        .insert(0, "Inter".into());
    fonts
        .families
        .get_mut(&FontFamily::Monospace)
        .expect("egui defines it")
        .insert(0, "JetBrains Mono".into());
    for (family, first) in [(medium(), "Inter Medium"), (semibold(), "Inter SemiBold")] {
        let mut list = vec![first.to_owned()];
        list.extend(fallback.iter().cloned());
        fonts.families.insert(family, list);
    }
    fonts
}

pub fn install(ctx: &egui::Context, theme: Theme) {
    ctx.set_fonts(fonts());
    apply_theme(ctx, theme);
    for (t, dark) in [(egui::Theme::Dark, true), (egui::Theme::Light, false)] {
        ctx.style_mut_of(t, |style| style_for(style, dark));
    }
}

fn style_for(style: &mut egui::Style, dark: bool) {
    let p = palette(dark);
    style.text_styles = [
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(13.0, medium())),
        (TextStyle::Heading, FontId::new(18.0, semibold())),
        (
            TextStyle::Monospace,
            FontId::new(12.5, FontFamily::Monospace),
        ),
    ]
    .into();
    // An app, not a document: clicks on labels select rows, not words.
    style.interaction.selectable_labels = false;

    let s = &mut style.spacing;
    s.item_spacing = egui::vec2(8.0, 6.0);
    s.button_padding = egui::vec2(10.0, 4.0);
    s.interact_size = egui::vec2(28.0, 26.0);
    s.window_margin = Margin::same(20);
    s.menu_margin = Margin::same(6);
    s.indent = 16.0;
    s.icon_spacing = 6.0;
    s.combo_height = 320.0;

    let v = &mut style.visuals;
    v.dark_mode = dark;
    v.override_text_color = None;
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.window_stroke = Stroke::new(1.0, p.border2);
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    let shadow = alpha(Color32::BLACK, if dark { 130 } else { 46 });
    v.window_shadow = Shadow {
        offset: [0, 20],
        blur: 56,
        spread: 0,
        color: shadow,
    };
    v.popup_shadow = Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: shadow,
    };
    v.extreme_bg_color = p.bg;
    v.text_edit_bg_color = Some(p.bg);
    v.faint_bg_color = p.zebra;
    v.code_bg_color = p.raised;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.bad;
    v.weak_text_color = Some(p.muted);
    v.selection.bg_fill = alpha(p.accent, if dark { 0x55 } else { 0x40 });
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    v.striped = false;
    v.indent_has_left_vline = false;
    v.collapsing_header_frame = false;

    let radius = CornerRadius::same(6);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = p.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = radius;
    w.inactive.bg_fill = p.raised;
    w.inactive.weak_bg_fill = p.raised;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border2);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = radius;
    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.border2);
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = p.hover;
    w.active.weak_bg_fill = p.hover;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;
    w.open = w.hovered;
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
        Keyword => (0xC68CF5, 0x8A2BC9),
        String => (0x9BD17B, 0x237A1F),
        Number => (0xF0A45D, 0xB35600),
        Comment => (0x6B7585, 0x8A929E),
        QuotedIdentifier => (0x6CC4D8, 0x0E6E86),
        Parameter => (0xF07C9A, 0xB02452),
        Operator | Punctuation => (0x9AA3B0, 0x505866),
        Identifier | Whitespace => (0xDDE1E6, 0x1F2328),
    };
    rgb(if dark { d } else { l })
}

/// Background for find matches (the current one is stronger).
pub fn match_bg(current: bool, dark: bool) -> Color32 {
    let warn = palette(dark).warn;
    alpha(warn, if current { 0x66 } else { 0x2A })
}

/// The window icon (packaging/make-icon.py draws it).
pub fn icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../../../packaging/icons/256.png"))
        .expect("the icon is a valid PNG")
}
