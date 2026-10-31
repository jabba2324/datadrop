use std::path::{Path, PathBuf};

use datadrop_core::Driver;
use egui::{
    Align, Color32, CornerRadius, FontId, Layout, RichText, Sense, Stroke, WidgetInfo, WidgetType,
};

use crate::kit;
use crate::theme::{self, colors};

const RECENT_SHOWN: usize = 5;
const CARD_GAP: f32 = 12.0;
const CARD_MAX: f32 = 232.0;
const CARD_PAD: f32 = 16.0;

#[derive(Debug, Clone)]
pub enum WelcomeAction {
    AddConnection,
    OpenDatabaseFile,
    ConnectUrl,
    OpenRecent(PathBuf),
}

pub fn ui(
    ui: &mut egui::Ui,
    recent: &[PathBuf],
    logo: &egui::TextureHandle,
) -> Option<WelcomeAction> {
    let mut action = None;
    let p = colors(ui);
    let shown: Vec<_> = recent.iter().take(RECENT_SHOWN).collect();
    let reserved = if shown.is_empty() { 0.0 } else { 160.0 };
    let top = ((ui.available_height() - 440.0 - reserved) * 0.4).max(16.0);
    ui.add_space(top);
    ui.vertical_centered(|ui| {
        let row_w = ui
            .available_width()
            .min(CARD_MAX * 3.0 + CARD_GAP * 2.0);
        let card_w = ((row_w - CARD_GAP * 2.0) / 3.0).max(0.0);
        let row_w = card_w * 3.0 + CARD_GAP * 2.0;
        logo_mark(ui, logo);
        ui.add_space(20.0);
        ui.label(
            RichText::new("Welcome to DataDrop")
                .font(FontId::new(28.0, theme::semibold()))
                .color(p.text),
        );
        ui.add_space(10.0);
        ui.scope(|ui| {
            ui.set_max_width(row_w);
            ui.label(
                RichText::new(
                    "A fast SQL client for Postgres, MySQL, SQLite, SQL Server, ClickHouse and DuckDB.",
                )
                .color(p.muted)
                .size(14.0),
            );
        });
        ui.add_space(32.0);
        let cards = [
            (
                "Add a connection",
                "A server, with SSH if you need it",
                Driver::Postgres,
                WelcomeAction::AddConnection,
            ),
            (
                "Open a database file",
                "SQLite or DuckDB on disk",
                Driver::DuckDb,
                WelcomeAction::OpenDatabaseFile,
            ),
            (
                "Connect with a URL",
                "Paste a connection string",
                Driver::Mysql,
                WelcomeAction::ConnectUrl,
            ),
        ];
        let row_h = cards
            .iter()
            .map(|(title, body, _, _)| card_height(ui, title, body, card_w))
            .fold(0.0_f32, f32::max);
        ui.allocate_ui_with_layout(
            egui::vec2(row_w, row_h),
            Layout::left_to_right(Align::Min),
            |ui| {
                ui.spacing_mut().item_spacing.x = CARD_GAP;
                for (title, body, driver, next) in cards {
                    if start_card(ui, title, body, driver, card_w, row_h).clicked() {
                        action = Some(next);
                    }
                }
            },
        );
        if !shown.is_empty() {
            ui.add_space(32.0);
            let h = ui.available_height();
            if h > 24.0 {
                ui.allocate_ui_with_layout(
                    egui::vec2(row_w, h),
                    Layout::top_down(Align::Min),
                    |ui| {
                        kit::caps(ui, "Recent files");
                        ui.add_space(8.0);
                        egui::ScrollArea::vertical()
                            .id_salt("welcome-recent")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for path in shown {
                                    if recent_row(ui, path).clicked() {
                                        action = Some(WelcomeAction::OpenRecent(path.clone()));
                                    }
                                }
                            });
                    },
                );
            }
        }
    });
    action
}

fn logo_mark(ui: &mut egui::Ui, logo: &egui::TextureHandle) {
    ui.add(
        egui::Image::new(egui::load::SizedTexture::new(
            logo.id(),
            egui::vec2(72.0, 72.0),
        ))
        .corner_radius(16.0),
    );
}

fn card_height(ui: &egui::Ui, title: &str, body: &str, width: f32) -> f32 {
    let text_w = (width - CARD_PAD * 2.0).max(24.0);
    let title_h = ui
        .painter()
        .layout(
            title.to_owned(),
            FontId::new(15.0, theme::semibold()),
            Color32::PLACEHOLDER,
            text_w,
        )
        .size()
        .y;
    let body_h = ui
        .painter()
        .layout(
            body.to_owned(),
            FontId::new(13.0, egui::FontFamily::Proportional),
            Color32::PLACEHOLDER,
            text_w,
        )
        .size()
        .y;
    CARD_PAD + 32.0 + 12.0 + title_h + 4.0 + body_h + CARD_PAD
}

fn start_card(
    ui: &mut egui::Ui,
    title: &str,
    body: &str,
    driver: Driver,
    width: f32,
    height: f32,
) -> egui::Response {
    let p = colors(ui);
    let dark = ui.visuals().dark_mode;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), Sense::click());
    let hovered = resp.hovered();
    ui.painter().add(
        egui::Shadow {
            offset: [0, 10],
            blur: 28,
            spread: 0,
            color: theme::alpha(Color32::BLACK, if dark { 90 } else { 28 }),
        }
        .as_shape(rect, 12.0),
    );
    ui.painter().rect(
        rect,
        CornerRadius::same(12),
        if hovered { p.hover } else { p.raised },
        Stroke::new(1.0, if hovered { p.accent } else { p.border2 }),
        egui::StrokeKind::Inside,
    );
    let mut badge = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
        rect.min + egui::vec2(CARD_PAD, CARD_PAD),
        egui::vec2(32.0, 32.0),
    )));
    kit::driver_badge(&mut badge, driver, 32.0);
    let text_w = (width - CARD_PAD * 2.0).max(24.0);
    let title_g = ui.painter().layout(
        title.to_owned(),
        FontId::new(15.0, theme::semibold()),
        p.text,
        text_w,
    );
    let title_pos = egui::pos2(rect.left() + CARD_PAD, rect.top() + CARD_PAD + 32.0 + 12.0);
    ui.painter().galley(title_pos, title_g.clone(), p.text);
    let body_g = ui.painter().layout(
        body.to_owned(),
        FontId::new(13.0, egui::FontFamily::Proportional),
        p.muted,
        text_w,
    );
    ui.painter().galley(
        egui::pos2(title_pos.x, title_pos.y + title_g.size().y + 4.0),
        body_g,
        p.muted,
    );
    resp.clone()
        .widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, title));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn recent_row(ui: &mut egui::Ui, path: &Path) -> egui::Response {
    let p = colors(ui);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let dir = parent_display(path);
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), p.hover);
    }
    let left = rect.left() + 12.0;
    let right = rect.right() - 12.0;
    let y = rect.center().y;
    ui.painter().text(
        egui::pos2(left, y),
        egui::Align2::LEFT_CENTER,
        egui_phosphor::regular::NOTE,
        FontId::new(13.0, egui::FontFamily::Proportional),
        p.faint,
    );
    let text_left = left + 22.0;
    let available = (right - text_left).max(0.0);
    let gap = 12.0;
    let dir_w = if dir.is_empty() {
        0.0
    } else {
        (available * 0.42).min(available)
    };
    let name_w = (available - dir_w - if dir.is_empty() { 0.0 } else { gap }).max(0.0);
    let prev = ui.clip_rect();
    ui.set_clip_rect(rect);
    let name_g = elided(
        ui,
        &name,
        FontId::new(13.0, egui::FontFamily::Proportional),
        p.text,
        name_w,
    );
    ui.painter().galley(
        egui::pos2(text_left, y - name_g.size().y / 2.0),
        name_g,
        p.text,
    );
    if dir_w > 0.0 {
        let dir_g = elided(
            ui,
            &dir,
            FontId::new(12.0, egui::FontFamily::Proportional),
            p.faint,
            dir_w,
        );
        ui.painter().galley(
            egui::pos2(right - dir_g.size().x, y - dir_g.size().y / 2.0),
            dir_g,
            p.faint,
        );
    }
    ui.set_clip_rect(prev);
    resp.clone()
        .widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, name.clone()));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn elided(
    ui: &egui::Ui,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::text::TextFormat::simple(font, color),
    );
    job.wrap.max_width = width;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    ui.ctx().fonts_mut(|fonts| fonts.layout_job(job))
}

fn parent_display(path: &Path) -> String {
    let Some(parent) = path.parent() else {
        return String::new();
    };
    if parent.as_os_str().is_empty() {
        return String::new();
    }
    if let Some(home) = std::env::home_dir()
        && let Ok(rel) = parent.strip_prefix(&home)
    {
        if rel.as_os_str().is_empty() {
            return "~".into();
        }
        return format!("~/{}", rel.display());
    }
    parent.display().to_string()
}

pub fn profile_name_from_path(path: &Path) -> String {
    let raw = path
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "database".into());
    let mut out = String::new();
    for c in raw.chars() {
        if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
            out.push(c);
        } else if c.is_whitespace() && !out.ends_with(' ') {
            out.push(' ');
        }
    }
    let out = out.trim().trim_matches('.').trim_start_matches('.');
    let out = if out.is_empty() {
        "database".to_owned()
    } else {
        out.chars().take(64).collect()
    };
    if datadrop_core::profile::validate_name(&out).is_ok() {
        out
    } else {
        "database".into()
    }
}

pub fn driver_for_path(path: &Path) -> Driver {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("duckdb" | "ddb") => Driver::DuckDb,
        _ => Driver::Sqlite,
    }
}

pub fn is_database_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref(),
        Some("db" | "sqlite" | "sqlite3" | "duckdb" | "ddb")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn database_files_from_extension() {
        assert!(is_database_file(Path::new("/tmp/shop.sqlite")));
        assert!(is_database_file(Path::new("notes.duckdb")));
        assert_eq!(driver_for_path(Path::new("notes.duckdb")), Driver::DuckDb);
        assert_eq!(driver_for_path(Path::new("shop.db")), Driver::Sqlite);
        assert!(!is_database_file(Path::new("orders.sql")));
    }

    #[test]
    fn profile_name_from_an_ugly_file() {
        assert_eq!(
            profile_name_from_path(Path::new("/tmp/my (backup).sqlite")),
            "my backup"
        );
        assert_eq!(profile_name_from_path(Path::new(".hidden.db")), "hidden");
        assert_eq!(profile_name_from_path(Path::new("@@@.sqlite")), "database");
    }

    #[test]
    fn home_in_recent_path() {
        let home = std::env::home_dir().unwrap();
        let path = home.join("work/orders.sql");
        assert_eq!(parent_display(&path), "~/work");
    }
}
