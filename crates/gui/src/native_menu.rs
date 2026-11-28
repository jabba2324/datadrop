//! The app's menus as commands. On macOS they go in the system menu bar, where Mac
//! apps keep them ([`NativeMenu`]); elsewhere egui draws them in the window
//! (`App::menu_bar`). Both send the same [`MenuCommand`]s.

use crate::settings::Theme;

/// What a menu item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCommand {
    NewTab,
    Open,
    Save,
    SaveAs,
    CloseTab,
    Quit,
    Find,
    Replace,
    FormatSql,
    Run,
    RunAll,
    Explain,
    Analyze,
    Stop,
    NewTable,
    Diagram,
    Compare,
    ShowDdl,
    AddConnection,
    OpenDatabaseFile,
    Refresh,
    Theme(Theme),
    /// The in-window menu has a number field for this instead.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    LargerText,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    SmallerText,
}

impl MenuCommand {
    /// Whether it still works with a dialog open (the rest wait for it to close).
    pub fn works_over_dialogs(self) -> bool {
        matches!(
            self,
            Self::Quit | Self::Theme(_) | Self::LargerText | Self::SmallerText
        )
    }
}

#[cfg(target_os = "macos")]
pub use mac::NativeMenu;

#[cfg(target_os = "macos")]
mod mac {
    use std::collections::HashMap;
    use std::sync::{Arc, mpsc};

    use muda::accelerator::Accelerator;
    use muda::{
        AboutMetadata, CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuId, MenuItem,
        PredefinedMenuItem, Submenu,
    };

    use super::MenuCommand;
    use crate::settings::Theme;

    /// The menu bar, installed for the whole app. Its items' shortcuts are the
    /// window's, so a key press goes to the menu (which highlights in the bar) and
    /// arrives here as a command.
    pub struct NativeMenu {
        /// Kept alive: the menu bar goes away with it.
        _menu: Menu,
        commands: HashMap<MenuId, MenuCommand>,
        events: mpsc::Receiver<MenuId>,
        /// Shown only when the tab has a connection.
        schema_items: Vec<MenuItem>,
        stop: MenuItem,
        themes: Vec<(Theme, CheckMenuItem)>,
        /// What the items show now, to change them only when it changes.
        shown: Option<(bool, bool, Theme)>,
    }

    impl NativeMenu {
        /// Make the menu bar and install it. Must run on the main thread (eframe's
        /// app creator does).
        pub fn install(ctx: &egui::Context) -> Self {
            let mut commands = HashMap::new();
            let mut item = |text: &str, shortcut: Option<&str>, cmd: MenuCommand| {
                let accel = shortcut.map(|s| {
                    s.parse::<Accelerator>()
                        .unwrap_or_else(|e| panic!("menu shortcut {s}: {e}"))
                });
                let i = MenuItem::new(text, true, accel);
                commands.insert(i.id().clone(), cmd);
                i
            };
            let sep = PredefinedMenuItem::separator;

            let quit = item("Quit DataDrop", Some("cmd+q"), MenuCommand::Quit);
            let about = PredefinedMenuItem::about(
                Some("About DataDrop"),
                Some(AboutMetadata {
                    name: Some("DataDrop".into()),
                    version: Some(env!("CARGO_PKG_VERSION").into()),
                    ..Default::default()
                }),
            );
            let app_menu = submenu(
                "DataDrop",
                &[
                    &about,
                    &sep(),
                    &PredefinedMenuItem::services(None),
                    &sep(),
                    &PredefinedMenuItem::hide(Some("Hide DataDrop")),
                    &PredefinedMenuItem::hide_others(None),
                    &PredefinedMenuItem::show_all(None),
                    &sep(),
                    &quit,
                ],
            );

            let file = submenu(
                "File",
                &[
                    &item("New Tab", Some("cmd+n"), MenuCommand::NewTab),
                    &item("Open…", Some("cmd+o"), MenuCommand::Open),
                    &item("Open a Database File…", None, MenuCommand::OpenDatabaseFile),
                    &sep(),
                    &item("Save", Some("cmd+s"), MenuCommand::Save),
                    &item("Save As…", Some("cmd+shift+s"), MenuCommand::SaveAs),
                    &sep(),
                    &item("Close Tab", Some("cmd+w"), MenuCommand::CloseTab),
                ],
            );
            let edit = submenu(
                "Edit",
                &[
                    &item("Find…", Some("cmd+f"), MenuCommand::Find),
                    &item("Replace…", Some("cmd+r"), MenuCommand::Replace),
                    &sep(),
                    &item("Format SQL", Some("cmd+shift+f"), MenuCommand::FormatSql),
                ],
            );
            let stop = item("Stop", None, MenuCommand::Stop);
            let query = submenu(
                "Query",
                &[
                    &item("Run Statement", Some("cmd+enter"), MenuCommand::Run),
                    &item("Run All", Some("cmd+shift+enter"), MenuCommand::RunAll),
                    &sep(),
                    &item("Explain", Some("cmd+e"), MenuCommand::Explain),
                    &item("Explain Analyze", Some("cmd+shift+e"), MenuCommand::Analyze),
                    &sep(),
                    &stop,
                ],
            );
            let schema_items = vec![
                item("New Table…", None, MenuCommand::NewTable),
                item("Diagram", None, MenuCommand::Diagram),
                item("Compare With…", None, MenuCommand::Compare),
                item("Show DDL", None, MenuCommand::ShowDdl),
            ];
            let schema_refs: Vec<&dyn IsMenuItem> =
                schema_items.iter().map(|i| i as &dyn IsMenuItem).collect();
            let schema = submenu("Schema", &schema_refs);
            let connections = submenu(
                "Connections",
                &[
                    &item("Add a Connection…", None, MenuCommand::AddConnection),
                    &item("Open a Database File…", None, MenuCommand::OpenDatabaseFile),
                    &sep(),
                    &item("Refresh", None, MenuCommand::Refresh),
                ],
            );

            let themes: Vec<(Theme, CheckMenuItem)> = [
                (Theme::System, "System Appearance"),
                (Theme::Light, "Light"),
                (Theme::Dark, "Dark"),
            ]
            .into_iter()
            .map(|(t, text)| (t, CheckMenuItem::new(text, true, false, None)))
            .collect();
            let mut view_items: Vec<&dyn IsMenuItem> =
                themes.iter().map(|(_, i)| i as &dyn IsMenuItem).collect();
            let sep1 = sep();
            let larger = item("Larger Text", Some("cmd+="), MenuCommand::LargerText);
            let smaller = item("Smaller Text", Some("cmd+-"), MenuCommand::SmallerText);
            let sep2 = sep();
            let full_screen = PredefinedMenuItem::fullscreen(Some("Enter Full Screen"));
            view_items.extend([
                &sep1 as &dyn IsMenuItem,
                &larger,
                &smaller,
                &sep2,
                &full_screen,
            ]);
            let view = submenu("View", &view_items);
            for (t, i) in &themes {
                commands.insert(i.id().clone(), MenuCommand::Theme(*t));
            }

            let window = submenu(
                "Window",
                &[
                    &PredefinedMenuItem::minimize(None),
                    &PredefinedMenuItem::maximize(Some("Zoom")),
                ],
            );

            let menu = Menu::new();
            for m in [
                &app_menu,
                &file,
                &edit,
                &query,
                &schema,
                &connections,
                &view,
                &window,
            ] {
                menu.append(m).expect("menu");
            }
            menu.init_for_nsapp();
            window.set_as_windows_menu_for_nsapp();

            // Menu events come between frames: pass them on and wake the app.
            let (tx, events) = mpsc::channel();
            let ctx = ctx.clone();
            MenuEvent::set_event_handler(Some(Arc::new(move |e: MenuEvent| {
                let _ = tx.send(e.id);
                ctx.request_repaint();
            })));

            Self {
                _menu: menu,
                commands,
                events,
                schema_items,
                stop,
                themes,
                shown: None,
            }
        }

        /// The commands picked since the last frame.
        pub fn commands(&self) -> Vec<MenuCommand> {
            self.events
                .try_iter()
                .filter_map(|id| self.commands.get(&id).copied())
                .collect()
        }

        /// Match the items to the app: what can be used, and the theme's tick.
        pub fn sync(&mut self, has_schema: bool, running: bool, theme: Theme) {
            let now = (has_schema, running, theme);
            if self.shown == Some(now) {
                return;
            }
            self.shown = Some(now);
            for i in &self.schema_items {
                i.set_enabled(has_schema);
            }
            self.stop.set_enabled(running);
            for (t, i) in &self.themes {
                i.set_checked(*t == theme);
            }
        }
    }

    fn submenu(text: &str, items: &[&dyn IsMenuItem]) -> Submenu {
        Submenu::with_items(text, true, items).expect("menu")
    }
}
