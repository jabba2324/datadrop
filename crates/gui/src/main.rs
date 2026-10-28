//! DataDrop desktop app: an egui front-end over `datadrop-core`, running the same
//! commands as the CLI.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod backend;
mod browser;
mod diagram_view;
mod diff_dialog;
mod editor;
mod grid_edit;
mod history_view;
mod import_dialog;
mod params_dialog;
mod plan_view;
mod profile_form;
mod results;
mod settings;
mod table_editor;
mod theme;
#[cfg(test)]
mod ui_tests;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("DATADROP_LOG")
                .unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let config = match datadrop_core::ConfigDir::default_location() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("datadrop-gui: {e}");
            std::process::exit(1);
        }
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DataDrop")
            .with_app_id("datadrop")
            .with_inner_size([1200.0, 760.0])
            .with_min_inner_size([760.0, 480.0])
            .with_icon(theme::icon()),
        ..Default::default()
    };
    eframe::run_native(
        "DataDrop",
        options,
        Box::new(move |cc| {
            let mut app = app::App::new(&cc.egui_ctx, config);
            // Files given on the command line (or by the desktop's "Open with").
            for path in std::env::args_os().skip(1).map(std::path::PathBuf::from) {
                if path.is_file() {
                    app.open_path(&path);
                }
            }
            Ok(Box::new(app))
        }),
    )
}
