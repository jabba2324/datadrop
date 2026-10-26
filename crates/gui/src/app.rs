use std::path::PathBuf;
use std::time::Instant;

use datadrop_core::export;
use datadrop_core::sql::{self, Dialect};
use datadrop_core::{
    Command, ConfigDir, CoreError, Driver, ExportFormat, Output, ProfileSummary, QueryOutput,
    StatementError,
};
use egui::{Key, KeyboardShortcut, Modifiers, RichText};
use egui_phosphor::regular as icon;

use crate::backend::{Backend, Task, poll};
use crate::browser::{Browser, BrowserAction};
use crate::editor::{self, Document, Find};
use crate::history_view::{HistoryAction, HistoryView};
use crate::profile_form::{FormDone, ProfileForm};
use crate::results::{self, ResultsAction};
use crate::settings::{Settings, Theme};
use crate::theme;

pub enum ConfirmKind {
    CloseTab(u64),
    Quit,
    DeleteProfile(String),
    ClearHistory,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LeftView {
    Connections,
    History,
}

pub struct Confirm {
    pub kind: ConfirmKind,
    pub title: String,
    pub text: String,
}

pub struct App {
    pub backend: Backend,
    pub settings: Settings,
    saved_settings: Settings,
    pub profiles: Vec<ProfileSummary>,
    profiles_task: Option<Task>,
    pub docs: Vec<Document>,
    pub active: usize,
    next_id: u64,
    untitled_count: usize,
    pub browser: Browser,
    pub left: LeftView,
    pub history: HistoryView,
    pub find: Find,
    pub form: Option<ProfileForm>,
    pub confirm: Option<Confirm>,
    delete_task: Option<Task>,
    pub toasts: egui_notify::Toasts,
    quitting: bool,
}

const RUN_ALL: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Enter);
const RUN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Enter);
const SAVE_AS: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
const CLOSE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::W);
const FIND: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);
const REPLACE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::R);
const FIND_PREV: KeyboardShortcut = KeyboardShortcut::new(Modifiers::SHIFT, Key::F3);
const FIND_NEXT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F3);

enum Shortcut {
    RunAll,
    Run,
    SaveAs,
    Save,
    Open,
    New,
    Close,
    Find,
    Replace,
    FindPrev,
    FindNext,
}

impl App {
    pub fn new(ctx: &egui::Context, config: ConfigDir) -> Self {
        let settings = Settings::load(config.root());
        theme::install(ctx, settings.theme);
        let backend = Backend::new(ctx.clone(), config);
        let mut app = Self {
            backend,
            saved_settings: settings.clone(),
            settings,
            profiles: vec![],
            profiles_task: None,
            docs: vec![],
            active: 0,
            next_id: 1,
            untitled_count: 0,
            browser: Browser::default(),
            left: LeftView::Connections,
            history: HistoryView::default(),
            find: Find::default(),
            form: None,
            confirm: None,
            delete_task: None,
            toasts: egui_notify::Toasts::default(),
            quitting: false,
        };
        app.refresh_profiles();
        for path in app.settings.open_files.clone() {
            if path.exists() {
                app.open_path(&path);
            }
        }
        if app.docs.is_empty() {
            app.new_tab(None);
        }
        app.active = 0;
        app
    }

    pub fn refresh_profiles(&mut self) {
        self.profiles_task = Some(self.backend.run(Command::ProfileList));
    }

    fn next_doc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn default_profile(&self) -> Option<String> {
        let known = |n: &String| self.profiles.iter().any(|p| &p.name == n);
        self.settings
            .last_profile
            .clone()
            .filter(known)
            .or_else(|| self.profiles.first().map(|p| p.name.clone()))
            .or_else(|| self.settings.last_profile.clone())
    }

    pub fn new_tab(&mut self, profile: Option<String>) -> usize {
        self.untitled_count += 1;
        let id = self.next_doc_id();
        let profile = profile.or_else(|| self.default_profile());
        self.docs
            .push(Document::untitled(id, self.untitled_count, profile));
        self.active = self.docs.len() - 1;
        self.active
    }

    pub fn open_path(&mut self, path: &std::path::Path) {
        if let Some(i) = self
            .docs
            .iter()
            .position(|d| d.path.as_deref() == Some(path))
        {
            self.active = i;
            return;
        }
        let id = self.next_doc_id();
        match Document::open(id, path, self.default_profile()) {
            Ok(doc) => {
                // Opening into an empty, untouched tab replaces it.
                if self
                    .docs
                    .get(self.active)
                    .is_some_and(Document::is_pristine)
                {
                    self.docs[self.active] = doc;
                } else {
                    self.docs.push(doc);
                    self.active = self.docs.len() - 1;
                }
                self.remember_open_files();
            }
            Err(e) => {
                self.toasts
                    .error(format!("Couldn't open {}: {e}", path.display()));
            }
        }
    }

    fn remember_open_files(&mut self) {
        self.settings.open_files = self.docs.iter().filter_map(|d| d.path.clone()).collect();
    }

    fn open_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .add_filter("SQL", &["sql"])
            .add_filter("All files", &["*"]);
        if let Some(dir) = self.current_dir() {
            dialog = dialog.set_directory(dir);
        }
        if let Some(files) = dialog.pick_files() {
            for f in files {
                self.open_path(&f);
            }
        }
    }

    fn current_dir(&self) -> Option<PathBuf> {
        self.docs
            .get(self.active)
            .and_then(|d| d.path.as_ref())
            .and_then(|p| p.parent())
            .map(|p| p.to_owned())
    }

    /// Save a tab; returns whether it was saved.
    pub fn save(&mut self, i: usize, choose_path: bool) -> bool {
        let path = match (&self.docs[i].path, choose_path) {
            (Some(p), false) => p.clone(),
            _ => {
                let mut dialog = rfd::FileDialog::new()
                    .add_filter("SQL", &["sql"])
                    .set_file_name(format!(
                        "{}.sql",
                        self.docs[i].title().trim_end_matches(".sql")
                    ));
                if let Some(dir) = self.current_dir() {
                    dialog = dialog.set_directory(dir);
                }
                match dialog.save_file() {
                    Some(p) => p,
                    None => return false,
                }
            }
        };
        match self.docs[i].save_to(&path) {
            Ok(()) => {
                self.remember_open_files();
                true
            }
            Err(e) => {
                self.toasts
                    .error(format!("Couldn't save {}: {e}", path.display()));
                false
            }
        }
    }

    pub fn request_close_tab(&mut self, i: usize) {
        let doc = &self.docs[i];
        if doc.dirty() {
            self.active = i;
            self.confirm = Some(Confirm {
                kind: ConfirmKind::CloseTab(doc.id),
                title: format!("Save changes to {}?", doc.title()),
                text: "Your changes will be lost if you don't save them.".into(),
            });
        } else {
            self.close_tab(i);
        }
    }

    fn close_tab(&mut self, i: usize) {
        let doc = self.docs.remove(i);
        if let Some(t) = &doc.running {
            t.cancel();
        }
        self.backend.fire(Command::SessionClose {
            session: doc.session(),
        });
        if self.docs.is_empty() {
            self.new_tab(None);
        }
        if self.active >= self.docs.len() || (self.active > i) {
            self.active = self.active.saturating_sub(1).min(self.docs.len() - 1);
        }
        self.remember_open_files();
    }

    fn dialect_of(&self, profile: Option<&str>) -> Dialect {
        profile
            .and_then(|n| self.profiles.iter().find(|p| p.name == n))
            .map_or(Driver::Postgres, |p| p.driver)
            .into()
    }

    /// Run the selection or the statement under the cursor (or the whole tab).
    pub fn run(&mut self, ctx: &egui::Context, whole: bool) {
        let i = self.active;
        let dialect = self.dialect_of(self.docs[i].profile.as_deref());
        let doc = &mut self.docs[i];
        if doc.running.is_some() {
            return;
        }
        let Some(profile) = doc.profile.clone() else {
            self.toasts
                .warning("Choose a connection for this tab first.");
            return;
        };
        let sql = if whole {
            Some(doc.text.clone())
        } else if let Some(sel) = doc.selection(ctx) {
            Some(doc.text[sel].to_owned())
        } else {
            let cursor = doc.cursor_range(ctx).map_or(0, |r| r.start);
            sql::statement_at(&doc.text, dialect, cursor).map(|r| doc.text[r].to_owned())
        };
        let Some(sql) = sql.filter(|s| !sql::split_statements(s, dialect).is_empty()) else {
            self.toasts.info("There's no SQL to run here.");
            return;
        };
        doc.running = Some(self.backend.run(Command::Query {
            profile: profile.clone(),
            sql,
            session: Some(doc.session()),
            max_rows: Some(self.settings.max_rows),
        }));
        doc.run_started = Some(Instant::now());
        self.settings.last_profile = Some(profile);
    }

    fn poll_tasks(&mut self) {
        if let Some(r) = poll(&mut self.profiles_task) {
            match r {
                Ok(Output::Profiles { profiles }) => {
                    self.profiles = profiles;
                    // Tabs without a connection get one once there is one.
                    let default = self.default_profile();
                    for d in &mut self.docs {
                        if d.profile.is_none() {
                            d.profile = default.clone();
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    self.toasts
                        .error(format!("Couldn't read the connections: {e}"));
                }
            }
        }
        if let Some(r) = poll(&mut self.delete_task) {
            match r {
                Ok(Output::ProfileRemoved { name }) => {
                    self.browser.forget(&name);
                    self.toasts.success(format!("Deleted {name}"));
                }
                Ok(_) => {}
                Err(e) => {
                    self.toasts.error(e.to_string());
                }
            }
            self.refresh_profiles();
        }
        for doc in &mut self.docs {
            match poll(&mut doc.exporting) {
                Some(Ok(Output::Exported { path, rows })) => {
                    self.toasts.success(format!(
                        "Saved {rows} rows to {}",
                        file_name(std::path::Path::new(&path))
                    ));
                }
                Some(Err(e)) => {
                    self.toasts.error(format!("Export failed: {e}"));
                }
                _ => {}
            }
        }
        let mut changed_schema = vec![];
        for doc in &mut self.docs {
            let Some(r) = poll(&mut doc.running) else {
                continue;
            };
            doc.shown_result = usize::MAX; // the last result set
            self.history.stale = true;
            doc.output = Some(match r {
                Ok(Output::Query(q)) => {
                    let dialect = Dialect::Postgres;
                    let ddl = q.results.iter().any(|r| {
                        matches!(
                            sql::first_keyword(&r.sql, dialect).as_deref(),
                            Some("CREATE" | "DROP" | "ALTER" | "RENAME")
                        )
                    });
                    if ddl && let Some(p) = &doc.profile {
                        changed_schema.push(p.clone());
                    }
                    q
                }
                Ok(_) => QueryOutput::default(),
                Err(e) => QueryOutput {
                    results: vec![],
                    cancelled: matches!(e, CoreError::Cancelled),
                    error: Some(StatementError {
                        index: 0,
                        sql: String::new(),
                        message: e.to_string(),
                    }),
                },
            });
        }
        for p in changed_schema {
            self.browser.forget(&p);
        }
        self.browser.poll();
        self.history.poll();
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let modal_open = self.form.is_some() || self.confirm.is_some();
        if modal_open {
            return;
        }
        let pressed = ctx.input_mut(|i| {
            // The Shift variants first: Ctrl+Enter also matches Ctrl+Shift+Enter.
            [
                (RUN_ALL, Shortcut::RunAll),
                (RUN, Shortcut::Run),
                (SAVE_AS, Shortcut::SaveAs),
                (SAVE, Shortcut::Save),
                (OPEN, Shortcut::Open),
                (NEW, Shortcut::New),
                (CLOSE, Shortcut::Close),
                (FIND, Shortcut::Find),
                (REPLACE, Shortcut::Replace),
                (FIND_PREV, Shortcut::FindPrev),
                (FIND_NEXT, Shortcut::FindNext),
            ]
            .into_iter()
            .find(|(k, _)| i.consume_shortcut(k))
            .map(|(_, s)| s)
        });
        match pressed {
            Some(Shortcut::RunAll) => self.run(ctx, true),
            Some(Shortcut::Run) => self.run(ctx, false),
            Some(Shortcut::SaveAs) => {
                self.save(self.active, true);
            }
            Some(Shortcut::Save) => {
                self.save(self.active, false);
            }
            Some(Shortcut::Open) => self.open_dialog(),
            Some(Shortcut::New) => {
                self.new_tab(None);
            }
            Some(Shortcut::Close) => self.request_close_tab(self.active),
            Some(Shortcut::Find) => self.open_find(ctx, false),
            Some(Shortcut::Replace) => self.open_find(ctx, true),
            Some(Shortcut::FindPrev) => self.find_step(false),
            Some(Shortcut::FindNext) => self.find_step(true),
            None => {}
        }
        if self.find.open && ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.find.open = false;
            self.docs[self.active].request_focus = true;
        }
    }

    fn open_find(&mut self, ctx: &egui::Context, replace: bool) {
        let doc = &self.docs[self.active];
        // Search for the selection, if it's a short single line.
        if let Some(sel) = doc.selection(ctx) {
            let s = &doc.text[sel];
            if !s.contains('\n') && s.len() < 200 {
                self.find.query = s.to_owned();
            }
        }
        self.find.open = true;
        self.find.replace = replace;
        self.find.focus = true;
        self.find.current = None;
    }

    /// Move to the next (or previous) match from the cursor.
    pub fn find_step(&mut self, forward: bool) {
        if !self.find.open || self.find.query.is_empty() {
            return;
        }
        let doc = &mut self.docs[self.active];
        let matches = editor::find_all(&doc.text, &self.find.query, self.find.match_case);
        if matches.is_empty() {
            self.find.current = None;
            return;
        }
        let n = matches.len();
        let next = match self.find.current.filter(|&c| c < n) {
            Some(c) if forward => (c + 1) % n,
            Some(c) => (c + n - 1) % n,
            None => {
                let at = doc.select.as_ref().map_or(0, |r| r.start);
                if forward {
                    matches.iter().position(|m| m.start >= at).unwrap_or(0)
                } else {
                    matches.iter().rposition(|m| m.start < at).unwrap_or(n - 1)
                }
            }
        };
        self.find.current = Some(next);
        doc.select = Some(matches[next].clone());
        doc.scroll_to = Some(matches[next].start);
    }

    pub fn replace_one(&mut self) {
        let doc = &mut self.docs[self.active];
        let matches = editor::find_all(&doc.text, &self.find.query, self.find.match_case);
        if let Some(r) = self.find.current.and_then(|c| matches.get(c)).cloned() {
            doc.text.replace_range(r.clone(), &self.find.replacement);
            // Stay at this position: the next match starts after the replacement.
            let after = r.start + self.find.replacement.len();
            let remaining = editor::find_all(&doc.text, &self.find.query, self.find.match_case);
            match remaining.iter().position(|m| m.start >= after) {
                Some(i) => {
                    self.find.current = Some(i);
                    doc.select = Some(remaining[i].clone());
                    doc.scroll_to = Some(remaining[i].start);
                }
                None => self.find.current = None,
            }
        } else {
            self.find_step(true);
        }
    }

    pub fn replace_all(&mut self) {
        let doc = &mut self.docs[self.active];
        let matches = editor::find_all(&doc.text, &self.find.query, self.find.match_case);
        for r in matches.iter().rev() {
            doc.text.replace_range(r.clone(), &self.find.replacement);
        }
        self.find.current = None;
        if !matches.is_empty() {
            self.toasts.info(format!(
                "Replaced {} occurrence{}",
                matches.len(),
                if matches.len() == 1 { "" } else { "s" }
            ));
        }
    }

    fn handle_close(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) || self.quitting {
            return;
        }
        let unsaved: Vec<String> = self
            .docs
            .iter()
            .filter(|d| d.dirty())
            .map(|d| d.title())
            .collect();
        if unsaved.is_empty() {
            self.quitting = true;
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Confirm {
                kind: ConfirmKind::Quit,
                title: "Quit without saving?".into(),
                text: format!("These tabs have unsaved changes: {}.", unsaved.join(", ")),
            });
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New tab").clicked() {
                    self.new_tab(None);
                }
                if ui.button("Open…").clicked() {
                    self.open_dialog();
                }
                ui.separator();
                if ui.button("Save").clicked() {
                    self.save(self.active, false);
                }
                if ui.button("Save as…").clicked() {
                    self.save(self.active, true);
                }
                ui.separator();
                if ui.button("Close tab").clicked() {
                    self.request_close_tab(self.active);
                }
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.button("Find").clicked() {
                    self.open_find(ui.ctx(), false);
                }
                if ui.button("Replace").clicked() {
                    self.open_find(ui.ctx(), true);
                }
            });
            ui.menu_button("Query", |ui| {
                if ui.button("Run statement").clicked() {
                    self.run(ui.ctx(), false);
                }
                if ui.button("Run all").clicked() {
                    self.run(ui.ctx(), true);
                }
                if ui.button("Stop").clicked()
                    && let Some(t) = &self.docs[self.active].running
                {
                    t.cancel();
                }
            });
            ui.menu_button("Connections", |ui| {
                if ui.button("Add a connection…").clicked() {
                    self.form = Some(ProfileForm::new());
                }
                if ui.button("Refresh").clicked() {
                    self.refresh_profiles();
                    let names: Vec<_> = self.profiles.iter().map(|p| p.name.clone()).collect();
                    for n in names {
                        self.browser.forget(&n);
                    }
                }
            });
            ui.menu_button("View", |ui| {
                for (t, label) in [
                    (Theme::System, "Theme: system"),
                    (Theme::Light, "Theme: light"),
                    (Theme::Dark, "Theme: dark"),
                ] {
                    if ui.radio(self.settings.theme == t, label).clicked() {
                        self.settings.theme = t;
                        theme::apply_theme(ui.ctx(), t);
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Editor text size");
                    ui.add(
                        egui::DragValue::new(&mut self.settings.editor_font_size)
                            .range(9.0..=28.0)
                            .speed(0.2),
                    );
                });
            });
        });
    }

    fn tab_strip(&mut self, ui: &mut egui::Ui) {
        let mut close = None;
        let mut activate = None;
        egui::ScrollArea::horizontal()
            .id_salt("tabs")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (i, d) in self.docs.iter().enumerate() {
                        let title = if d.dirty() {
                            format!("{} {}", d.title(), icon::DOT)
                        } else {
                            d.title()
                        };
                        let tip = d
                            .path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "Not saved yet".into());
                        let resp = ui
                            .add(egui::Button::selectable(i == self.active, title))
                            .on_hover_text(tip);
                        if resp.clicked() {
                            activate = Some(i);
                        }
                        if resp.middle_clicked() {
                            close = Some(i);
                        }
                        if ui
                            .small_button(icon::X)
                            .on_hover_text("Close tab")
                            .clicked()
                        {
                            close = Some(i);
                        }
                        ui.add_space(4.0);
                    }
                    if ui
                        .small_button(icon::PLUS)
                        .on_hover_text("New tab")
                        .clicked()
                    {
                        self.new_tab(None);
                    }
                });
            });
        if let Some(i) = activate {
            self.active = i;
        }
        if let Some(i) = close {
            self.request_close_tab(i);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let i = self.active;
        ui.horizontal(|ui| {
            let current = self.docs[i].profile.clone();
            let label = current
                .clone()
                .unwrap_or_else(|| "Choose a connection".into());
            let resp = egui::ComboBox::from_id_salt(("tab-profile", self.docs[i].id))
                .selected_text(format!("{}  {label}", icon::DATABASE))
                .width(200.0)
                .show_ui(ui, |ui| {
                    for p in &self.profiles {
                        ui.selectable_value(
                            &mut self.docs[i].profile,
                            Some(p.name.clone()),
                            &p.name,
                        )
                        .on_hover_text(&p.target);
                    }
                    if self.profiles.is_empty() {
                        ui.weak("No connections yet");
                    }
                });
            resp.response
                .on_hover_text("The connection this tab runs on");
            if self.docs[i].profile != current {
                // A different connection: the old session isn't needed.
                self.backend.fire(Command::SessionClose {
                    session: self.docs[i].session(),
                });
            }

            let running = self.docs[i].running.is_some();
            ui.add_enabled_ui(!running, |ui| {
                if ui
                    .button(format!("{} Run", icon::PLAY))
                    .on_hover_text(format!(
                        "Run the selection, or the statement at the cursor ({})",
                        ui.ctx().format_shortcut(&RUN)
                    ))
                    .clicked()
                {
                    self.run(ui.ctx(), false);
                }
                if ui
                    .button(format!("{} Run all", icon::FAST_FORWARD))
                    .on_hover_text(format!(
                        "Run every statement in the tab ({})",
                        ui.ctx().format_shortcut(&RUN_ALL)
                    ))
                    .clicked()
                {
                    self.run(ui.ctx(), true);
                }
            });
            if running
                && ui
                    .button(format!("{} Stop", icon::STOP))
                    .on_hover_text("Cancel the query on the server")
                    .clicked()
                && let Some(t) = &self.docs[i].running
            {
                t.cancel();
            }
            ui.separator();
            ui.label("Row limit");
            ui.add(
                egui::DragValue::new(&mut self.settings.max_rows)
                    .range(1..=1_000_000)
                    .speed(10.0),
            )
            .on_hover_text("Rows kept per result; the rest are left out");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(icon::MAGNIFYING_GLASS)
                    .on_hover_text(format!(
                        "Find and replace ({})",
                        ui.ctx().format_shortcut(&FIND)
                    ))
                    .clicked()
                {
                    let ctx = ui.ctx().clone();
                    self.open_find(&ctx, true);
                }
            });
        });
    }

    fn find_bar(&mut self, ui: &mut egui::Ui) {
        if !self.find.open {
            return;
        }
        let count = editor::find_all(
            &self.docs[self.active].text,
            &self.find.query,
            self.find.match_case,
        )
        .len();
        let mut step = None;
        ui.horizontal(|ui| {
            let l = ui.label(icon::MAGNIFYING_GLASS);
            let field = ui
                .add(
                    egui::TextEdit::singleline(&mut self.find.query)
                        .hint_text("Find")
                        .desired_width(220.0),
                )
                .labelled_by(l.id);
            if field.changed() {
                self.find.current = None;
            }
            if self.find.focus {
                field.request_focus();
                self.find.focus = false;
            }
            if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                step = Some(!ui.input(|i| i.modifiers.shift));
                field.request_focus();
            }
            if ui
                .selectable_label(self.find.match_case, "Aa")
                .on_hover_text("Match case")
                .clicked()
            {
                self.find.match_case = !self.find.match_case;
                self.find.current = None;
            }
            if ui
                .small_button(icon::ARROW_UP)
                .on_hover_text("Previous (Shift+Enter)")
                .clicked()
            {
                step = Some(false);
            }
            if ui
                .small_button(icon::ARROW_DOWN)
                .on_hover_text("Next (Enter)")
                .clicked()
            {
                step = Some(true);
            }
            let status = match (count, self.find.current) {
                (0, _) if !self.find.query.is_empty() => "No matches".to_owned(),
                (0, _) => String::new(),
                (n, Some(c)) if c < n => format!("{} of {n}", c + 1),
                (n, _) => format!("{n} found"),
            };
            ui.weak(status);
            if !self.find.replace
                && ui
                    .small_button(icon::CARET_DOWN)
                    .on_hover_text("Replace")
                    .clicked()
            {
                self.find.replace = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button(icon::X)
                    .on_hover_text("Close (Esc)")
                    .clicked()
                {
                    self.find.open = false;
                }
            });
        });
        if self.find.replace {
            ui.horizontal(|ui| {
                let l = ui.label(icon::SWAP);
                ui.add(
                    egui::TextEdit::singleline(&mut self.find.replacement)
                        .hint_text("Replace with")
                        .desired_width(220.0),
                )
                .labelled_by(l.id);
                if ui.button("Replace").clicked() {
                    self.replace_one();
                }
                if ui.button("Replace all").clicked() {
                    self.replace_all();
                }
            });
        }
        if let Some(forward) = step {
            self.find_step(forward);
        }
    }

    fn browser_action(&mut self, action: BrowserAction) {
        match action {
            BrowserAction::Add => self.form = Some(ProfileForm::new()),
            BrowserAction::NewTab(p) => {
                self.new_tab(Some(p));
            }
            BrowserAction::Edit(name) => match self.backend.client.store().get(&name) {
                Ok(p) => self.form = Some(ProfileForm::edit(&p)),
                Err(e) => {
                    self.toasts.error(e.to_string());
                }
            },
            BrowserAction::Delete(name) => {
                self.confirm = Some(Confirm {
                    title: format!("Delete the connection {name}?"),
                    text: "Its saved settings and password are removed. Your databases and \
                           .sql files aren't touched."
                        .into(),
                    kind: ConfirmKind::DeleteProfile(name),
                });
            }
            BrowserAction::OpenTable {
                profile,
                schema,
                table,
            } => {
                let dialect = self.dialect_of(Some(&profile));
                let sql = format!(
                    "SELECT * FROM {}.{} LIMIT 100;",
                    sql::quote_ident(&schema, dialect),
                    sql::quote_ident(&table, dialect)
                );
                let i = self.new_tab(Some(profile));
                self.docs[i].text = sql.clone();
                self.docs[i].saved_text = sql;
                self.run_whole(i);
            }
        }
    }

    /// Save a result to a file. If it was cut short by the row limit (and is a query),
    /// the statement runs again with every row streamed into the file.
    fn export_result(&mut self, doc: usize, result: usize) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .add_filter("Tab-separated", &["tsv"])
            .add_filter("JSON", &["json"])
            .set_file_name("result.csv")
            .save_file()
        else {
            return;
        };
        self.export_result_to(doc, result, path);
    }

    pub fn export_result_to(&mut self, doc: usize, result: usize, path: std::path::PathBuf) {
        let Some(out) = &self.docs[doc].output else {
            return;
        };
        let Some(r) = out.results.get(result).cloned() else {
            return;
        };
        let format = ExportFormat::from_path(&path).unwrap_or(ExportFormat::Csv);
        let dialect = self.dialect_of(self.docs[doc].profile.as_deref());
        let is_query = matches!(
            sql::first_keyword(&r.sql, dialect).as_deref(),
            Some("SELECT" | "WITH" | "VALUES" | "TABLE" | "SHOW")
        );
        if r.truncated
            && is_query
            && let Some(profile) = self.docs[doc].profile.clone()
        {
            let d = &mut self.docs[doc];
            d.exporting = Some(self.backend.run(Command::Export {
                profile,
                sql: r.sql,
                path,
                format: Some(format),
                session: Some(d.session()),
            }));
            return;
        }
        let names: Vec<&str> = r.columns.iter().map(|c| c.name.as_str()).collect();
        let written = std::fs::File::create(&path)
            .and_then(|f| export::write_all(std::io::BufWriter::new(f), format, &names, &r.rows));
        match written {
            Ok(n) => {
                self.toasts
                    .success(format!("Saved {n} rows to {}", file_name(&path)));
            }
            Err(e) => {
                self.toasts
                    .error(format!("Couldn't save {}: {e}", path.display()));
            }
        }
    }

    fn history_action(&mut self, ctx: &egui::Context, action: HistoryAction) {
        match action {
            HistoryAction::OpenInTab(e) => {
                let known = self.profiles.iter().any(|p| p.name == e.profile);
                let i = self.new_tab(known.then_some(e.profile));
                self.docs[i].text = e.sql;
            }
            HistoryAction::Insert(sql) => {
                let doc = &mut self.docs[self.active];
                let at = doc.cursor_range(ctx).map_or(doc.text.len(), |r| r.end);
                // On its own line(s).
                let mut text = sql;
                if at > 0 && !doc.text[..at].ends_with('\n') {
                    text.insert(0, '\n');
                }
                if !text.ends_with('\n') && at < doc.text.len() {
                    text.push('\n');
                }
                doc.text.insert_str(at, &text);
                let end = at + text.len();
                doc.select = Some(end..end);
                doc.request_focus = true;
            }
            HistoryAction::Clear => {
                self.confirm = Some(Confirm {
                    kind: ConfirmKind::ClearHistory,
                    title: "Clear the query history?".into(),
                    text: "Every query you've run, from the app and the command line, is \
                           forgotten."
                        .into(),
                });
            }
        }
    }

    fn run_whole(&mut self, i: usize) {
        let doc = &mut self.docs[i];
        let Some(profile) = doc.profile.clone() else {
            return;
        };
        doc.running = Some(self.backend.run(Command::Query {
            profile,
            sql: doc.text.clone(),
            session: Some(doc.session()),
            max_rows: Some(self.settings.max_rows),
        }));
        doc.run_started = Some(Instant::now());
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(form) = &mut self.form
            && let Some(done) = form.ui(ctx, &self.backend)
        {
            if let FormDone::Saved(name) = done {
                if let Some(old) = &form.previous {
                    self.browser.forget(old);
                    // Tabs on the old name follow the rename.
                    for d in &mut self.docs {
                        if d.profile.as_deref() == Some(old) {
                            d.profile = Some(name.clone());
                        }
                    }
                }
                self.browser.forget(&name);
                self.toasts.success(format!("Saved {name}"));
                if self.settings.last_profile.is_none() {
                    self.settings.last_profile = Some(name);
                }
                self.refresh_profiles();
            }
            self.form = None;
        }

        let Some(c) = &self.confirm else { return };
        let mut choice: Option<&str> = None;
        egui::Modal::new(egui::Id::new("confirm")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.heading(&c.title);
            ui.add_space(6.0);
            ui.label(&c.text);
            ui.add_space(12.0);
            ui.horizontal(|ui| match &c.kind {
                ConfirmKind::CloseTab(_) => {
                    if ui.button("Save").clicked() {
                        choice = Some("save");
                    }
                    if ui.button("Don't save").clicked() {
                        choice = Some("discard");
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some("cancel");
                    }
                }
                ConfirmKind::Quit => {
                    if ui.button("Quit without saving").clicked() {
                        choice = Some("discard");
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some("cancel");
                    }
                }
                ConfirmKind::ClearHistory => {
                    if ui
                        .button(RichText::new("Clear").color(ui.visuals().error_fg_color))
                        .clicked()
                    {
                        choice = Some("delete");
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some("cancel");
                    }
                }
                ConfirmKind::DeleteProfile(_) => {
                    if ui
                        .button(RichText::new("Delete").color(ui.visuals().error_fg_color))
                        .clicked()
                    {
                        choice = Some("delete");
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some("cancel");
                    }
                }
            });
        });
        let Some(choice) = choice else { return };
        let c = self.confirm.take().expect("checked above");
        match (c.kind, choice) {
            (ConfirmKind::CloseTab(id), "save") => {
                if let Some(i) = self.docs.iter().position(|d| d.id == id)
                    && self.save(i, false)
                {
                    self.close_tab(i);
                }
            }
            (ConfirmKind::CloseTab(id), "discard") => {
                if let Some(i) = self.docs.iter().position(|d| d.id == id) {
                    self.close_tab(i);
                }
            }
            (ConfirmKind::Quit, "discard") => {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            (ConfirmKind::ClearHistory, "delete") => {
                self.backend.fire(Command::HistoryClear { profile: None });
                self.history.entries.clear();
                self.history.stale = true;
            }
            (ConfirmKind::DeleteProfile(name), "delete") => {
                self.delete_task = Some(self.backend.run(Command::ProfileRemove { name }));
            }
            _ => {}
        }
    }

    fn save_settings_if_changed(&mut self) {
        if self.settings != self.saved_settings {
            match self.settings.save(self.backend.config.root()) {
                Ok(()) => self.saved_settings = self.settings.clone(),
                Err(e) => {
                    self.toasts.error(format!("Couldn't save settings: {e}"));
                    // Don't retry every frame.
                    self.saved_settings = self.settings.clone();
                }
            }
        }
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_tasks();
        self.shortcuts(&ctx);
        self.handle_close(&ctx);

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::left("browser")
            .resizable(true)
            .default_size(250.0)
            .min_size(160.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.left, LeftView::Connections, "Connections");
                    ui.selectable_value(&mut self.left, LeftView::History, "History");
                });
                match self.left {
                    LeftView::Connections => {
                        if let Some(a) = self.browser.ui(ui, &self.profiles, &self.backend) {
                            self.browser_action(a);
                        }
                    }
                    LeftView::History => {
                        if let Some(a) = self.history.ui(ui, &self.backend) {
                            self.history_action(&ctx, a);
                        }
                    }
                }
            });
        egui::CentralPanel::default_margins().show(ui, |ui| {
            self.tab_strip(ui);
            self.toolbar(ui);
            self.find_bar(ui);
            let i = self.active;
            egui::Panel::bottom("results")
                .resizable(true)
                .default_size(280.0)
                .min_size(80.0)
                .show(ui, |ui| {
                    // Keep the pane's size whatever it shows.
                    ui.set_min_size(ui.available_size());
                    match results::results_ui(ui, &mut self.docs[i]) {
                        ResultsAction::Stop => {
                            if let Some(t) = &self.docs[i].running {
                                t.cancel();
                            }
                        }
                        ResultsAction::Export(r) => self.export_result(i, r),
                        ResultsAction::None => {}
                    }
                });
            let dialect = self.dialect_of(self.docs[i].profile.as_deref());
            let matches = self.find.matches(&self.docs[i].text);
            let current = self.find.current;
            let size = self.settings.editor_font_size;
            editor::editor_ui(ui, &mut self.docs[i], dialect, &matches, current, size);
        });

        self.dialogs(&ctx);
        self.toasts.show(&ctx);
        let title = format!(
            "{}{} — DataDrop",
            self.docs[self.active].title(),
            if self.docs[self.active].dirty() {
                " ●"
            } else {
                ""
            }
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        self.save_settings_if_changed();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.remember_open_files();
        self.save_settings_if_changed();
        self.backend.shutdown();
    }
}
