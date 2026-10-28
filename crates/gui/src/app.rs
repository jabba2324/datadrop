use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use datadrop_core::sql::{self, Dialect};
use datadrop_core::{
    Catalog, Command, ConfigDir, CoreError, Driver, ExportFormat, ObjectKind, Output,
    ProfileSummary, QueryOutput, StatementError,
};
use datadrop_core::{edit, export};
use egui::{Key, KeyboardShortcut, Modifiers, RichText};
use egui_phosphor::regular as icon;

use crate::backend::{Backend, Task, poll};
use crate::browser::{Browser, BrowserAction};
use crate::editor::{self, Document, Find, ResultView};
use crate::grid_edit::{Editability, Review};
use crate::history_view::{HistoryAction, HistoryView};
use crate::params_dialog::{ParamPrompt, ParamsDone, Then};
use crate::profile_form::{FormDone, ProfileForm, TrustPrompt};
use crate::results::{self, ResultsAction};
use crate::settings::{Settings, Theme};
use crate::theme;

pub enum ConfirmKind {
    CloseTab(u64),
    Quit,
    DeleteProfile(String),
    ClearHistory,
    /// The tab has grid edits that weren't saved; then do this.
    DiscardEdits(u64, AfterDiscard),
}

pub enum AfterDiscard {
    Run(Box<Command>, ResultView),
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LeftView {
    Connections,
    History,
}

enum CatalogState {
    Loading(Option<Task>),
    Ready(Arc<Catalog>),
    Failed,
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
    /// Tables and columns per connection, for autocompletion.
    catalogs: HashMap<String, CatalogState>,
    pub form: Option<ProfileForm>,
    pub confirm: Option<Confirm>,
    delete_task: Option<Task>,
    /// Asking for the values of the query's placeholders.
    pub params: Option<ParamPrompt>,
    /// DDL being read, to open in a new tab: (task, profile, tab name).
    ddl_tasks: Vec<(Option<Task>, String, String)>,
    /// An SSH server to trust (its key isn't known yet), and the task trusting it.
    pub trust: Option<TrustPrompt>,
    trusting: Option<Task>,
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
const ANALYZE: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::E);
const EXPLAIN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::E);
const REPLACE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::R);
const FIND_PREV: KeyboardShortcut = KeyboardShortcut::new(Modifiers::SHIFT, Key::F3);
const FIND_NEXT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F3);
const COMPLETE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Space);

enum Shortcut {
    RunAll,
    Run,
    Analyze,
    Explain,
    SaveAs,
    Save,
    Open,
    New,
    Close,
    Find,
    Replace,
    FindPrev,
    FindNext,
    Complete,
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
            catalogs: HashMap::new(),
            form: None,
            confirm: None,
            delete_task: None,
            ddl_tasks: vec![],
            params: None,
            trust: None,
            trusting: None,
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

    /// Whether passwords can go in the keychain (checked once).
    fn keychain(&self) -> Result<(), String> {
        self.backend.client.store().keychain().check()
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
        let edits = doc.grid.edits.count();
        if edits > 0 {
            self.active = i;
            self.confirm = Some(Confirm {
                kind: ConfirmKind::DiscardEdits(doc.id, AfterDiscard::Close),
                title: "Discard your edits?".into(),
                text: format!(
                    "{} to the results in {} {} not saved yet.",
                    plural(edits, "change"),
                    doc.title(),
                    if edits == 1 { "is" } else { "are" },
                ),
            });
        } else if doc.dirty() {
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

    /// The tab's connection and the SQL to run: the whole tab, the selection, or the
    /// statement at the cursor. `None` (after saying why) if there's nothing to run.
    fn sql_to_run(&mut self, ctx: &egui::Context, whole: bool) -> Option<(String, String)> {
        let i = self.active;
        let dialect = self.dialect_of(self.docs[i].profile.as_deref());
        let doc = &self.docs[i];
        if doc.running.is_some() {
            return None;
        }
        let Some(profile) = doc.profile.clone() else {
            self.toasts
                .warning("Choose a connection for this tab first.");
            return None;
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
            return None;
        };
        self.settings.last_profile = Some(profile.clone());
        Some((profile, sql))
    }

    /// Run the selection or the statement under the cursor (or the whole tab).
    pub fn run(&mut self, ctx: &egui::Context, whole: bool) {
        let Some((profile, sql)) = self.sql_to_run(ctx, whole) else {
            return;
        };
        self.then(Then::Run, profile, sql);
    }

    /// Show the plan for the selection or the statement under the cursor.
    pub fn explain(&mut self, ctx: &egui::Context, analyze: bool) {
        let Some((profile, sql)) = self.sql_to_run(ctx, false) else {
            return;
        };
        self.then(Then::Explain { analyze }, profile, sql);
    }

    /// Run or explain `sql` in the active tab, asking for placeholder values first.
    fn then(&mut self, then: Then, profile: String, sql: String) {
        let dialect = self.dialect_of(Some(&profile));
        let doc = &self.docs[self.active];
        if let Some(prompt) = ParamPrompt::new(
            doc.id,
            then,
            profile.clone(),
            sql.clone(),
            dialect,
            &doc.param_values,
        ) {
            self.params = Some(prompt);
            return;
        }
        self.run_sql(self.active, then, profile, sql);
    }

    fn run_sql(&mut self, i: usize, then: Then, profile: String, sql: String) {
        let session = Some(self.docs[i].session());
        let (command, view) = match then {
            Then::Run => (
                Command::Query {
                    profile,
                    sql,
                    session,
                    max_rows: Some(self.settings.max_rows),
                },
                ResultView::Results,
            ),
            Then::Explain { analyze } => (
                Command::Explain {
                    profile,
                    sql,
                    analyze,
                    session,
                },
                ResultView::Plan,
            ),
        };
        self.start(i, command, view);
    }

    /// Run a command in tab `i`, first asking about grid edits it would throw away.
    fn start(&mut self, i: usize, command: Command, view: ResultView) {
        let doc = &mut self.docs[i];
        let edits = doc.grid.edits.count();
        if edits > 0 {
            self.confirm = Some(Confirm {
                kind: ConfirmKind::DiscardEdits(doc.id, AfterDiscard::Run(Box::new(command), view)),
                title: "Discard your edits?".into(),
                text: format!(
                    "{} to the results {} not saved yet. Running another query throws {} away.",
                    plural(edits, "change"),
                    if edits == 1 { "is" } else { "are" },
                    if edits == 1 { "it" } else { "them" },
                ),
            });
            return;
        }
        doc.running = Some(self.backend.run(command));
        doc.run_started = Some(Instant::now());
        if view == ResultView::Plan {
            doc.plan_error = None;
        }
        doc.view = view;
    }

    /// The catalog for autocompletion on `profile`, loading it if needed (empty until
    /// it arrives, so keywords are still offered).
    fn catalog_for(&mut self, profile: Option<String>) -> Arc<Catalog> {
        let Some(profile) = profile else {
            return Arc::default();
        };
        let state = self.catalogs.entry(profile.clone()).or_insert_with(|| {
            CatalogState::Loading(Some(self.backend.run(Command::Catalog { profile })))
        });
        if let CatalogState::Loading(task) = state {
            match poll(task) {
                Some(Ok(Output::Catalog(c))) => *state = CatalogState::Ready(Arc::new(c)),
                Some(_) => *state = CatalogState::Failed,
                None => {}
            }
        }
        match state {
            CatalogState::Ready(c) => c.clone(),
            _ => Arc::default(),
        }
    }

    /// Something changed a profile's tables (or it was edited): read them again.
    fn schema_changed(&mut self, profile: &str) {
        self.browser.forget(profile);
        self.catalogs.remove(profile);
    }

    fn poll_tasks(&mut self) {
        let mut i = 0;
        while i < self.ddl_tasks.len() {
            let Some(r) = poll(&mut self.ddl_tasks[i].0) else {
                i += 1;
                continue;
            };
            let (_, profile, name) = self.ddl_tasks.remove(i);
            match r {
                Ok(Output::Ddl { sql }) => {
                    let t = self.new_tab(Some(profile));
                    let doc = &mut self.docs[t];
                    doc.untitled_name = name;
                    doc.text = sql.clone();
                    doc.saved_text = sql;
                }
                Ok(_) => {}
                Err(e) => {
                    self.toasts.error(format!("Couldn't read the DDL: {e}"));
                }
            }
        }
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
                    self.schema_changed(&name);
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
        for doc in &mut self.docs {
            let Editability::Checking(task, source) = &mut doc.grid.editability else {
                continue;
            };
            let Some(r) = poll(task) else { continue };
            let source = source.clone();
            let result = doc.grid.result.unwrap_or(0);
            let shown: Vec<String> = doc
                .output
                .as_ref()
                .and_then(|o| o.results.get(result))
                .map(|r| r.columns.iter().map(|c| c.name.clone()).collect())
                .unwrap_or_default();
            doc.grid.editability = match r {
                Ok(Output::Columns { columns, .. }) => {
                    let key: Vec<String> = columns
                        .iter()
                        .filter(|c| c.primary_key)
                        .map(|c| c.name.clone())
                        .collect();
                    let missing: Vec<&String> = key.iter().filter(|k| !shown.contains(k)).collect();
                    if columns.is_empty() {
                        Editability::ReadOnly(format!("Couldn't find the table {}.", source.table))
                    } else if key.is_empty() {
                        Editability::ReadOnly(format!(
                            "{} has no primary key, so its rows can't be told apart.",
                            source.table
                        ))
                    } else if !missing.is_empty() {
                        Editability::ReadOnly(format!(
                            "Add the key column{} {} to the SELECT to edit rows.",
                            if missing.len() == 1 { "" } else { "s" },
                            missing
                                .iter()
                                .map(|m| m.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ))
                    } else {
                        Editability::Editable { source, key }
                    }
                }
                Ok(_) => Editability::ReadOnly("Unexpected reply.".into()),
                Err(e) => Editability::ReadOnly(e.to_string()),
            };
        }
        let mut changed_schema = vec![];
        let mut untrusted = None;
        for doc in &mut self.docs {
            let Some(r) = poll(&mut doc.running) else {
                continue;
            };
            if doc.view == ResultView::Plan {
                match r {
                    Ok(Output::Plan(p)) => {
                        doc.plan = Some(p);
                        doc.plan_collapsed.clear();
                    }
                    Ok(_) => {}
                    Err(e) => {
                        doc.plan = None;
                        doc.plan_error = Some(e.to_string());
                    }
                }
                continue;
            }
            doc.shown_result = usize::MAX; // the last result set
            doc.grid = Default::default();
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
                    cancelled: {
                        if let Some(t) = TrustPrompt::from_error(&e) {
                            untrusted = Some(t);
                        }
                        matches!(e, CoreError::Cancelled)
                    },
                    error: Some(StatementError {
                        index: 0,
                        sql: String::new(),
                        message: e.to_string(),
                    }),
                },
            });
        }
        for p in changed_schema {
            self.schema_changed(&p);
        }
        self.browser.poll();
        if let Some(t) = untrusted.or_else(|| self.browser.untrusted.take())
            && self.trust.is_none()
            && self.trusting.is_none()
        {
            self.trust = Some(t);
        }
        if let Some(r) = poll(&mut self.trusting) {
            match r {
                Ok(_) => {
                    self.toasts
                        .success("Trusted. Run the query again, or reopen the connection.");
                    self.browser.forget_all();
                    self.catalogs.clear();
                }
                Err(e) => {
                    self.toasts.error(e.to_string());
                }
            }
        }
        self.history.poll();
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let modal_open = self.form.is_some() || self.confirm.is_some() || self.params.is_some();
        if modal_open {
            return;
        }
        let pressed = ctx.input_mut(|i| {
            // The Shift variants first: Ctrl+Enter also matches Ctrl+Shift+Enter.
            [
                (RUN_ALL, Shortcut::RunAll),
                (RUN, Shortcut::Run),
                (ANALYZE, Shortcut::Analyze),
                (EXPLAIN, Shortcut::Explain),
                (SAVE_AS, Shortcut::SaveAs),
                (SAVE, Shortcut::Save),
                (OPEN, Shortcut::Open),
                (NEW, Shortcut::New),
                (CLOSE, Shortcut::Close),
                (FIND, Shortcut::Find),
                (REPLACE, Shortcut::Replace),
                (FIND_PREV, Shortcut::FindPrev),
                (FIND_NEXT, Shortcut::FindNext),
                (COMPLETE, Shortcut::Complete),
            ]
            .into_iter()
            .find(|(k, _)| i.consume_shortcut(k))
            .map(|(_, s)| s)
        });
        match pressed {
            Some(Shortcut::RunAll) => self.run(ctx, true),
            Some(Shortcut::Run) => self.run(ctx, false),
            Some(Shortcut::Analyze) => self.explain(ctx, true),
            Some(Shortcut::Explain) => self.explain(ctx, false),
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
            Some(Shortcut::Complete) => self.docs[self.active].force_completion = true,
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
            .filter(|d| d.dirty() || d.grid.edits.count() > 0)
            .map(|d| {
                if d.grid.edits.count() > 0 {
                    format!("{} (unsaved edits to its results)", d.title())
                } else {
                    d.title()
                }
            })
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
                if ui.button("Explain").clicked() {
                    self.explain(ui.ctx(), false);
                }
                if ui.button("Explain analyze").clicked() {
                    self.explain(ui.ctx(), true);
                }
                if ui.button("Stop").clicked()
                    && let Some(t) = &self.docs[self.active].running
                {
                    t.cancel();
                }
            });
            ui.menu_button("Connections", |ui| {
                if ui.button("Add a connection…").clicked() {
                    self.form = Some(ProfileForm::new(self.keychain()));
                }
                if ui.button("Refresh").clicked() {
                    self.refresh_profiles();
                    let names: Vec<_> = self.profiles.iter().map(|p| p.name.clone()).collect();
                    for n in names {
                        self.schema_changed(&n);
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
                if ui
                    .button(format!("{} Explain", icon::TREE_STRUCTURE))
                    .on_hover_text(format!(
                        "Show how the database will run the statement, without running it ({})",
                        ui.ctx().format_shortcut(&EXPLAIN)
                    ))
                    .clicked()
                {
                    self.explain(ui.ctx(), false);
                }
                if ui
                    .button(format!("{} Analyze", icon::TIMER))
                    .on_hover_text(format!(
                        "Run the statement and time each step ({}). Changes to data are \
                         rolled back.",
                        ui.ctx().format_shortcut(&ANALYZE)
                    ))
                    .clicked()
                {
                    self.explain(ui.ctx(), true);
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
            BrowserAction::Add => self.form = Some(ProfileForm::new(self.keychain())),
            BrowserAction::NewTab(p) => {
                self.new_tab(Some(p));
            }
            BrowserAction::Edit(name) => match self.backend.client.store().get(&name) {
                Ok(p) => self.form = Some(ProfileForm::edit(&p, self.keychain())),
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
            BrowserAction::ShowDdl {
                profile,
                schema,
                kind,
                name,
            } => {
                let title = match kind {
                    ObjectKind::Schema => format!("{schema} (DDL)"),
                    _ => format!("{name} (DDL)"),
                };
                let task = self.backend.run(Command::Ddl {
                    profile: profile.clone(),
                    schema: Some(schema),
                    kind,
                    name,
                });
                self.ddl_tasks.push((Some(task), profile, title));
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

    /// Whether a result can be edited: it must come from one table, whose primary
    /// key is in the result.
    fn check_editable(&mut self, doc: usize, result: usize) {
        let dialect = self.dialect_of(self.docs[doc].profile.as_deref());
        let d = &mut self.docs[doc];
        let (Some(out), Some(profile)) = (&d.output, d.profile.clone()) else {
            return;
        };
        let Some(r) = out.results.get(result) else {
            return;
        };
        // A filtered result has the rows of the query as written.
        let sql = match &d.filter {
            Some(f) if f.wrapped == r.sql && !f.base.is_empty() => &f.base,
            _ => &r.sql,
        };
        d.grid.editability = match edit::source_table(sql, dialect) {
            None => Editability::ReadOnly(
                "Only results from a single table can be edited (no joins, grouping, DISTINCT \
                 or subqueries)."
                    .into(),
            ),
            Some(source) => Editability::Checking(
                Some(self.backend.run(Command::Columns {
                    profile,
                    schema: source.schema.clone(),
                    table: source.table.clone(),
                })),
                source,
            ),
        };
    }

    /// Run a result's query again, filtered by the condition typed for it, or sorted by
    /// column `sort` (ascending, descending, then not, as clicks go).
    fn filter_result(&mut self, doc: usize, result: usize, sort: Option<usize>) {
        let dialect = self.dialect_of(self.docs[doc].profile.as_deref());
        let d = &mut self.docs[doc];
        let (Some(r), Some(profile)) = (
            d.output.as_ref().and_then(|o| o.results.get(result)),
            d.profile.clone(),
        ) else {
            return;
        };
        let column = sort.and_then(|c| r.columns.get(c)).map(|c| c.name.clone());
        let sql = r.sql.clone();
        let f = d.filter.get_or_insert_with(Default::default);
        if f.base.is_empty() || f.wrapped != sql {
            f.base = sql;
        }
        if let Some(col) = column {
            f.order = match f.order.take() {
                Some((c, false)) if c == col => Some((col, true)),
                Some((c, true)) if c == col => None,
                _ => Some((col, false)),
            };
        } else {
            f.applied = f.text.trim().to_owned();
        }
        let wrapped = if f.applied.is_empty() && f.order.is_none() {
            f.base.clone()
        } else {
            sql::filtered(
                &f.base,
                &f.applied,
                f.order.as_ref().map(|(c, desc)| (c.as_str(), *desc)),
                dialect,
            )
        };
        f.wrapped = wrapped.clone();
        let command = Command::Query {
            profile,
            sql: wrapped,
            session: Some(d.session()),
            max_rows: Some(self.settings.max_rows),
        };
        self.start(doc, command, ResultView::Results);
    }

    fn clear_filter(&mut self, doc: usize) {
        if let Some(f) = &mut self.docs[doc].filter {
            f.text.clear();
            f.order = None;
        }
        let result = self.docs[doc].grid.result.unwrap_or(0);
        self.filter_result(doc, result, None);
    }

    /// Show the SQL that saves a result's edits.
    fn review_edits(&mut self, doc: usize, result: usize) {
        let dialect = self.dialect_of(self.docs[doc].profile.as_deref());
        let d = &mut self.docs[doc];
        let Some(r) = d.output.as_ref().and_then(|o| o.results.get(result)) else {
            return;
        };
        let Editability::Editable { source, key } = &d.grid.editability else {
            return;
        };
        let columns: Vec<String> = r.columns.iter().map(|c| c.name.clone()).collect();
        match edit::statements(dialect, source, key, &columns, &r.rows, &d.grid.edits) {
            Ok(statements) if statements.is_empty() => {
                self.toasts.info("Nothing has changed.");
            }
            Ok(statements) => {
                d.grid.review = Some(Review {
                    statements,
                    saving: None,
                    error: None,
                    refresh: r.sql.clone(),
                });
            }
            Err(e) => {
                self.toasts.error(e.to_string());
            }
        }
    }

    fn review_dialog(&mut self, ctx: &egui::Context) {
        let i = self.active;
        let Some(profile) = self.docs[i].profile.clone() else {
            return;
        };
        let Some(review) = &mut self.docs[i].grid.review else {
            return;
        };
        let mut done = None;
        if let Some(r) = poll(&mut review.saving) {
            match r {
                Ok(_) => done = Some(true),
                Err(e) => review.error = Some(e.to_string()),
            }
        }
        let mut close = false;
        let mut save = false;
        egui::Modal::new(egui::Id::new("review-edits")).show(ctx, |ui| {
            ui.set_width(620.0);
            ui.heading("Save changes");
            ui.add_space(4.0);
            ui.label(format!(
                "{} statement{} will run in one transaction. If any of them doesn't change \
                 exactly one row, nothing is saved.",
                review.statements.len(),
                if review.statements.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ));
            ui.add_space(6.0);
            let mut text = review
                .statements
                .iter()
                .map(|s| format!("{s};"))
                .collect::<Vec<_>>()
                .join("\n");
            egui::ScrollArea::vertical()
                .max_height(280.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .interactive(true),
                    );
                });
            if let Some(e) = &review.error {
                ui.add_space(6.0);
                ui.label(RichText::new(e).color(ui.visuals().error_fg_color));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(format!("{} Copy SQL", icon::COPY)).clicked() {
                    ui.ctx().copy_text(text.clone());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let saving = review.saving.is_some();
                    if ui
                        .add_enabled(!saving, egui::Button::new(RichText::new("Save").strong()))
                        .clicked()
                    {
                        save = true;
                    }
                    if ui
                        .add_enabled(!saving, egui::Button::new("Cancel"))
                        .clicked()
                    {
                        close = true;
                    }
                    if saving {
                        ui.spinner();
                    }
                });
            });
        });
        if save {
            review.error = None;
            review.saving = Some(self.backend.run(Command::ApplyEdits {
                profile,
                statements: review.statements.clone(),
            }));
        }
        if close {
            self.docs[i].grid.review = None;
        }
        if done == Some(true) {
            let refresh = self.docs[i].grid.review.take().map(|r| r.refresh);
            self.docs[i].grid = Default::default();
            self.toasts.success("Saved");
            if let Some(sql) = refresh {
                self.rerun(i, sql);
            }
        }
    }

    /// Run `sql` in tab `i` (to show a table again after saving, say).
    fn rerun(&mut self, i: usize, sql: String) {
        let doc = &mut self.docs[i];
        let Some(profile) = doc.profile.clone() else {
            return;
        };
        doc.running = Some(self.backend.run(Command::Query {
            profile,
            sql,
            session: Some(doc.session()),
            max_rows: Some(self.settings.max_rows),
        }));
        doc.run_started = Some(Instant::now());
        doc.view = ResultView::Results;
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

    fn trust_dialog(&mut self, ctx: &egui::Context) {
        let Some(t) = self.trust.clone() else { return };
        let mut choice = None;
        egui::Modal::new(egui::Id::new("trust-ssh")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading("Trust this SSH server?");
            ui.add_space(6.0);
            ui.label(format!(
                "DataDrop hasn't connected to {}:{} before. Its key fingerprint is:",
                t.host, t.port
            ));
            ui.add_space(4.0);
            ui.monospace(&t.fingerprint);
            ui.add_space(6.0);
            ui.weak(
                "Check it with the server's admin (or `ssh-keygen -lf` on the server) before \
                 trusting it: a different fingerprint can mean someone is intercepting the \
                 connection.",
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Trust").clicked() {
                    choice = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(false);
                }
            });
        });
        match choice {
            Some(true) => {
                self.trusting = Some(self.backend.run(t.command()));
                self.trust = None;
            }
            Some(false) => self.trust = None,
            None => {}
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        self.review_dialog(ctx);
        if let Some(p) = &mut self.params
            && let Some(done) = p.ui(ctx)
        {
            let p = self.params.take().expect("checked above");
            if let ParamsDone::Run(sql, remember) = done
                && let Some(i) = self.docs.iter().position(|d| d.id == p.doc)
            {
                self.docs[i].param_values.extend(remember);
                self.run_sql(i, p.then, p.profile, sql);
            }
        }
        if self.form.is_none() {
            self.trust_dialog(ctx);
        }
        if let Some(form) = &mut self.form
            && let Some(done) = form.ui(ctx, &self.backend)
        {
            if let FormDone::Saved(name) = done {
                if let Some(old) = form.previous.clone() {
                    self.browser.forget(&old);
                    self.catalogs.remove(&old);
                    // Tabs on the old name follow the rename.
                    for d in &mut self.docs {
                        if d.profile.as_deref() == Some(old.as_str()) {
                            d.profile = Some(name.clone());
                        }
                    }
                }
                self.schema_changed(&name);
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
                ConfirmKind::DiscardEdits(..) => {
                    if ui.button("Review and save").clicked() {
                        choice = Some("review");
                    }
                    if ui
                        .button(RichText::new("Discard").color(ui.visuals().error_fg_color))
                        .clicked()
                    {
                        choice = Some("discard");
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
            (ConfirmKind::DiscardEdits(id, _), "review") => {
                if let Some(i) = self.docs.iter().position(|d| d.id == id) {
                    self.active = i;
                    let result = self.docs[i].grid.result.unwrap_or(0);
                    self.review_edits(i, result);
                }
            }
            (ConfirmKind::DiscardEdits(id, then), "discard") => {
                if let Some(i) = self.docs.iter().position(|d| d.id == id) {
                    self.docs[i].grid = Default::default();
                    match then {
                        AfterDiscard::Run(command, view) => self.start(i, *command, view),
                        AfterDiscard::Close => self.request_close_tab(i),
                    }
                }
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
                    let dialect = self.dialect_of(self.docs[i].profile.as_deref());
                    match results::results_ui(ui, &mut self.docs[i], dialect) {
                        ResultsAction::Stop => {
                            if let Some(t) = &self.docs[i].running {
                                t.cancel();
                            }
                        }
                        ResultsAction::Export(r) => self.export_result(i, r),
                        ResultsAction::CheckEditable(r) => self.check_editable(i, r),
                        ResultsAction::Review(r) => self.review_edits(i, r),
                        ResultsAction::Filter(r) => self.filter_result(i, r, None),
                        ResultsAction::Sort(r, col) => self.filter_result(i, r, Some(col)),
                        ResultsAction::ClearFilter => self.clear_filter(i),
                        ResultsAction::None => {}
                    }
                });
            let dialect = self.dialect_of(self.docs[i].profile.as_deref());
            let matches = self.find.matches(&self.docs[i].text);
            let current = self.find.current;
            let size = self.settings.editor_font_size;
            let catalog = self.catalog_for(self.docs[i].profile.clone());
            editor::editor_ui(
                ui,
                &mut self.docs[i],
                dialect,
                &matches,
                current,
                size,
                &catalog,
            );
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

/// "1 change", "3 changes".
fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}
