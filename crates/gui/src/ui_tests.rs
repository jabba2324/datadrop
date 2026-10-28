//! UI tests: the real app in egui's test harness (no window or GPU needed).

use datadrop_core::{Client, Command, ConfigDir, Driver, Keychain, Profile};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::app::{App, ConfirmKind};

/// The keychain the tests use: in memory, shared by every client in a test process.
fn keychain() -> Keychain {
    static K: std::sync::OnceLock<Keychain> = std::sync::OnceLock::new();
    K.get_or_init(Keychain::memory).clone()
}

fn config(dir: &std::path::Path) -> ConfigDir {
    ConfigDir::new(dir).with_keychain(keychain())
}

fn app(dir: &std::path::Path) -> Harness<'static, App> {
    let config = config(dir);
    Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .build_eframe(move |cc| App::new(&cc.egui_ctx, config))
}

/// Run frames until `done` holds (background commands need real time).
fn until(h: &mut Harness<'static, App>, what: &str, done: impl Fn(&App) -> bool) {
    for _ in 0..400 {
        h.run_steps(1);
        if done(h.state()) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("timed out waiting for {what}");
}

fn save_profile(dir: &std::path::Path, profile: Profile) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let client = Client::new(&config(dir));
    rt.block_on(client.execute(Command::ProfileSave {
        profile,
        previous_name: None,
        overwrite: true,
    }))
    .unwrap();
}

#[test]
fn first_run_offers_to_add_a_connection() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = app(dir.path());
    h.run_steps(5);
    h.get_by_label("No connections yet.");
    h.get_by_label_contains("Add a connection").click();
    h.run_steps(3);
    assert!(h.state().form.is_some());
    h.get_by_label("New connection");
}

#[test]
fn the_form_saves_a_profile() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = app(dir.path());
    h.run_steps(3);
    h.state_mut().form = Some(crate::profile_form::ProfileForm::new(Ok(())));
    h.run_steps(3);
    // The name field has focus when the form opens.
    h.get_by_label("Name").type_text("Shop");
    h.run_steps(2);
    h.get_by_label("Host").click();
    h.run_steps(1);
    h.get_by_label("Host").type_text("db.internal");
    h.run_steps(1);
    h.get_by_label("Port").click();
    h.run_steps(1);
    h.get_by_label("Port").type_text("6543");
    h.run_steps(1);
    h.get_by_label("Password").click();
    h.run_steps(1);
    h.get_by_label("Password").type_text("pw");
    h.run_steps(1);
    h.get_by_label("Save").click();
    until(&mut h, "the profile to save", |a| {
        a.form.is_none() && a.profiles.iter().any(|p| p.name == "Shop")
    });
    let saved = Client::new(&config(dir.path()))
        .store()
        .get("Shop")
        .unwrap();
    assert_eq!(saved.host, "db.internal");
    assert_eq!(saved.port, Some(6543));
    assert_eq!(saved.password.as_deref(), Some("pw"));
    // The password went in the keychain, not the file.
    assert_eq!(saved.passwords, datadrop_core::PasswordStorage::Keychain);
    let file = std::fs::read_to_string(dir.path().join("profiles/Shop.json")).unwrap();
    assert!(!file.contains("\"pw\""), "{file}");
    // New tabs use it.
    assert_eq!(h.state().docs[0].profile.as_deref(), Some("Shop"));
}

#[test]
fn find_and_replace() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = app(dir.path());
    h.run_steps(3);
    h.state_mut().docs[0].text = "select a from t;\nSELECT b from t;".into();
    {
        let a = h.state_mut();
        a.find.open = true;
        a.find.replace = true;
        a.find.query = "select".into();
        a.find.replacement = "SELECT DISTINCT".into();
    }
    h.run_steps(2);
    h.get_by_label("2 found");
    h.get_by_label("Replace all").click();
    h.run_steps(2);
    assert_eq!(
        h.state().docs[0].text,
        "SELECT DISTINCT a from t;\nSELECT DISTINCT b from t;"
    );
    // Match case finds nothing lower-case now.
    h.state_mut().find.match_case = true;
    h.state_mut().find.query = "distinct".into();
    h.run_steps(2);
    h.get_by_label("No matches");
}

#[test]
fn closing_an_unsaved_tab_asks_first() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = app(dir.path());
    h.run_steps(3);
    h.state_mut().new_tab(None);
    h.state_mut().docs[1].text = "select 1".into();
    h.run_steps(2);
    h.state_mut().request_close_tab(1);
    h.run_steps(2);
    assert!(matches!(
        h.state().confirm.as_ref().map(|c| &c.kind),
        Some(ConfirmKind::CloseTab(_))
    ));
    h.get_by_label("Don't save").click();
    h.run_steps(2);
    assert_eq!(h.state().docs.len(), 1);

    // A tab without changes closes straight away.
    h.state_mut().new_tab(None);
    h.state_mut().request_close_tab(1);
    h.run_steps(2);
    assert!(h.state().confirm.is_none());
    assert_eq!(h.state().docs.len(), 1);
}

#[test]
fn opens_and_saves_files() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("report.sql");
    std::fs::write(&file, "select 42;\n").unwrap();
    let mut h = app(dir.path());
    h.run_steps(3);
    h.state_mut().open_path(&file);
    h.run_steps(2);
    // It replaced the empty tab.
    assert_eq!(h.state().docs.len(), 1);
    h.get_by_label("report.sql");
    h.state_mut().docs[0].text.push_str("select 43;\n");
    h.run_steps(2);
    assert!(h.state().docs[0].dirty());
    assert!(h.state_mut().save(0, false));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "select 42;\nselect 43;\n"
    );
    assert!(!h.state().docs[0].dirty());
    // Open files come back next time.
    h.run_steps(2);
    drop(h);
    let mut again = app(dir.path());
    again.run_steps(3);
    assert_eq!(again.state().docs[0].path.as_deref(), Some(file.as_path()));
}

/// With a real PostgreSQL (`DATADROP_TEST_POSTGRES_URL`): browse, run, see results.
#[test]
fn runs_queries_against_postgres() {
    let Ok(url) = std::env::var("DATADROP_TEST_POSTGRES_URL") else {
        eprintln!("DATADROP_TEST_POSTGRES_URL isn't set; skipping");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    save_profile(
        dir.path(),
        Profile {
            name: "pg".into(),
            driver: Driver::Postgres,
            url: Some(url),
            ..Profile::default()
        },
    );
    let mut h = app(dir.path());
    until(&mut h, "the profiles", |a| !a.profiles.is_empty());
    h.state_mut().docs[0].text = "select 1 as a;\nselect 'x' as b, null as c;".into();
    let ctx = h.ctx.clone();
    h.state_mut().run(&ctx, true);
    until(&mut h, "the query", |a| a.docs[0].output.is_some());
    let out = h.state().docs[0].output.clone().unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    assert_eq!(out.results.len(), 2);
    h.run_steps(3);
    // The last result is shown, NULL included.
    h.get_by_label("Result 2 (1 rows)");
    h.get_by_label("NULL");

    // The schema browser lists the server's schemas when opened.
    h.get_by_label_contains("pg").click();
    until(&mut h, "the schemas", |_| true);
    for _ in 0..80 {
        h.run_steps(1);
        if h.query_by_label_contains("public").is_some() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("the public schema never showed");
}

fn sqlite_profile(dir: &std::path::Path) {
    save_profile(
        dir,
        Profile {
            name: "local".into(),
            driver: Driver::Sqlite,
            database: dir.join("local.db").display().to_string(),
            create_if_missing: true,
            ..Profile::default()
        },
    );
}

/// Run the whole first tab and wait for its output.
fn run_tab(h: &mut Harness<'static, App>, sql: &str) -> datadrop_core::QueryOutput {
    h.state_mut().docs[0].text = sql.into();
    let ctx = h.ctx.clone();
    h.state_mut().run(&ctx, true);
    until(h, "the query", |a| {
        a.docs[0].output.is_some() && a.docs[0].running.is_none()
    });
    h.state().docs[0].output.clone().unwrap()
}

#[test]
fn history_records_and_reopens() {
    let dir = tempfile::tempdir().unwrap();
    sqlite_profile(dir.path());
    let mut h = app(dir.path());
    until(&mut h, "the profiles", |a| !a.profiles.is_empty());
    let out = run_tab(
        &mut h,
        "create table fruit (name text); insert into fruit values ('lemon')",
    );
    assert!(out.error.is_none(), "{:?}", out.error);

    h.get_by_label("History").click();
    until(&mut h, "the history", |a| !a.history.entries.is_empty());
    h.run_steps(2);
    let entry = h.state().history.entries[0].clone();
    assert_eq!(entry.profile, "local");
    assert!(entry.sql.starts_with("create table fruit"));
    assert_eq!(entry.statements, 2);

    // Its menu opens it in a new tab, on the same connection.
    h.get_by_label_contains("create table fruit")
        .click_secondary();
    h.run_steps(2);
    h.get_by_label_contains("Open in a new tab").click();
    h.run_steps(3);
    let a = h.state();
    assert_eq!(a.docs.len(), 2);
    assert_eq!(a.docs[1].text, entry.sql);
    assert_eq!(a.docs[1].profile.as_deref(), Some("local"));
}

#[test]
fn exports_results() {
    let dir = tempfile::tempdir().unwrap();
    sqlite_profile(dir.path());
    let mut h = app(dir.path());
    until(&mut h, "the profiles", |a| !a.profiles.is_empty());
    h.state_mut().settings.max_rows = 10;
    let out = run_tab(
        &mut h,
        "with recursive c(n) as (select 1 union all select n + 1 from c where n < 50) \
         select n, n * n as square from c",
    );
    assert!(out.results[0].truncated);
    h.run_steps(2);
    h.get_by_label_contains("Export…");

    // Cut short by the row limit: the query runs again and every row is saved.
    let all = dir.path().join("all.csv");
    h.state_mut().export_result_to(0, 0, all.clone());
    until(&mut h, "the export", |a| a.docs[0].exporting.is_none());
    let csv = std::fs::read_to_string(&all).unwrap();
    assert_eq!(csv.lines().count(), 51);
    assert_eq!(csv.lines().nth(50), Some("50,2500"));

    // A complete result is saved from what's on screen.
    let out = run_tab(&mut h, "select 'a' as x union all select null");
    assert!(!out.results[0].truncated);
    let json = dir.path().join("small.json");
    h.state_mut().export_result_to(0, 0, json.clone());
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(json).unwrap()).unwrap();
    assert_eq!(v, serde_json::json!([{"x": "a"}, {"x": null}]));
}

#[test]
fn autocompletes_tables_and_columns() {
    let dir = tempfile::tempdir().unwrap();
    sqlite_profile(dir.path());
    let mut h = app(dir.path());
    until(&mut h, "the profiles", |a| !a.profiles.is_empty());
    run_tab(&mut h, "create table fruit (name text, ripe boolean)");
    // The table change reloads the catalog; give it a moment.
    for _ in 0..20 {
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    h.state_mut().docs[0].text.clear();
    h.run_steps(2);
    let editor = || egui::accesskit::Role::MultilineTextInput;
    h.get_by_role(editor()).focus();
    h.run_steps(1);
    h.get_by_role(editor()).type_text("select * from fr");
    until(&mut h, "suggestions", |a| a.docs[0].completion.is_some());
    h.run_steps(1);
    h.get_by_label_contains("fruit");
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
    assert_eq!(h.state().docs[0].text, "select * from fruit");
    assert!(h.state().docs[0].completion.is_none());

    h.get_by_role(editor()).type_text(" f where f.r");
    until(&mut h, "column suggestions", |a| {
        a.docs[0].completion.is_some()
    });
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().docs[0].text, "select * from fruit f where f.ripe");

    // Esc closes the list without changing anything.
    h.get_by_role(editor()).type_text(" and f.n");
    until(&mut h, "more suggestions", |a| {
        a.docs[0].completion.is_some()
    });
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().docs[0].completion.is_none());
    assert!(h.state().docs[0].text.ends_with("f.n"));
}

#[test]
fn edits_rows_in_the_grid() {
    use crate::grid_edit::Editability;
    let dir = tempfile::tempdir().unwrap();
    sqlite_profile(dir.path());
    let mut h = app(dir.path());
    until(&mut h, "the profiles", |a| !a.profiles.is_empty());
    run_tab(
        &mut h,
        "create table pets (id integer primary key, name text); \
         insert into pets values (1, 'Rex'), (2, 'Tom'), (3, 'Bo')",
    );
    run_tab(&mut h, "select id, name from pets order by id");
    until(&mut h, "the grid to become editable", |a| {
        a.docs[0].grid.is_editable()
    });

    // Right-click a cell: delete its row.
    h.get_by_label("Tom").click_secondary();
    h.run_steps(2);
    h.get_by_label("Delete the row").click();
    h.run_steps(2);
    // Change another cell (as typing in it would) and add a row.
    {
        let a = h.state_mut();
        let rows = a.docs[0].output.as_ref().unwrap().results[0].rows.clone();
        a.docs[0].grid.set(0, 1, Some("Rex's".into()), &rows);
        a.docs[0].grid.edits.inserted.push(Default::default());
        a.docs[0].grid.set(3, 0, Some("4".into()), &rows);
        a.docs[0].grid.set(3, 1, None, &rows);
    }
    h.run_steps(2);
    h.get_by_label_contains("Review and save (3)").click();
    h.run_steps(3);
    let statements = h.state().docs[0]
        .grid
        .review
        .as_ref()
        .unwrap()
        .statements
        .clone();
    assert_eq!(
        statements,
        [
            "UPDATE \"pets\" SET \"name\" = 'Rex''s' WHERE \"id\" = '1'",
            "DELETE FROM \"pets\" WHERE \"id\" = '2'",
            "INSERT INTO \"pets\" (\"id\", \"name\") VALUES ('4', NULL)",
        ]
    );
    h.get_by_label("Save").click();
    until(&mut h, "the save and the refresh", |a| {
        a.docs[0].grid.review.is_none()
            && a.docs[0].running.is_none()
            && a.docs[0]
                .output
                .as_ref()
                .is_some_and(|o| o.results[0].rows.len() == 3)
    });
    let rows = h.state().docs[0].output.clone().unwrap().results[0]
        .rows
        .clone();
    assert_eq!(
        rows,
        [
            vec![Some("1".to_owned()), Some("Rex's".to_owned())],
            vec![Some("3".to_owned()), Some("Bo".to_owned())],
            vec![Some("4".to_owned()), None],
        ]
    );

    // Running another query with an edit not saved asks first.
    until(&mut h, "the grid to become editable", |a| {
        a.docs[0].grid.is_editable()
    });
    {
        let a = h.state_mut();
        let rows = a.docs[0].output.as_ref().unwrap().results[0].rows.clone();
        a.docs[0].grid.set(0, 1, Some("Max".into()), &rows);
        a.docs[0].text = "select 1".into();
        let ctx = h.ctx.clone();
        h.state_mut().run(&ctx, true);
    }
    h.run_steps(2);
    assert!(h.state().docs[0].running.is_none());
    h.get_by_label("Discard your edits?");
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert_eq!(h.state().docs[0].grid.edits.count(), 1);
    let ctx = h.ctx.clone();
    h.state_mut().run(&ctx, true);
    h.run_steps(2);
    h.get_by_label("Discard").click();
    h.run_steps(1);
    assert!(h.state().docs[0].running.is_some());
    until(&mut h, "the query", |a| a.docs[0].running.is_none());
    assert_eq!(h.state().docs[0].grid.edits.count(), 0);

    // A join isn't editable.
    run_tab(
        &mut h,
        "select a.id, b.name from pets a join pets b on a.id = b.id",
    );
    h.run_steps(3);
    assert!(matches!(
        h.state().docs[0].grid.editability,
        Editability::ReadOnly(_)
    ));
}

/// With an SSH server in front of PostgreSQL (see crates/core/tests/ssh.rs for the
/// environment): an unknown host key brings up the trust prompt, and once trusted
/// queries go through.
#[test]
fn trusts_an_ssh_server_then_runs() {
    let (Ok(host), Ok(db)) = (
        std::env::var("DATADROP_TEST_SSH_HOST"),
        std::env::var("DATADROP_TEST_SSH_DB_URL"),
    ) else {
        eprintln!("DATADROP_TEST_SSH_HOST isn't set; skipping");
        return;
    };
    let var = |k: &str| std::env::var(format!("DATADROP_TEST_SSH_{k}")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    save_profile(
        dir.path(),
        Profile {
            name: "tunnelled".into(),
            driver: Driver::Postgres,
            url: Some(db),
            ssl_mode: datadrop_core::SslMode::Disable,
            ssh: Some(datadrop_core::SshTunnel {
                host,
                port: var("PORT").parse().ok(),
                user: var("USER"),
                auth: datadrop_core::SshAuth::Password {
                    password: var("PASSWORD"),
                },
            }),
            ..Profile::default()
        },
    );
    let mut h = app(dir.path());
    until(&mut h, "the profiles", |a| !a.profiles.is_empty());
    let out = run_tab(&mut h, "select 'through the tunnel' as how");
    assert!(out.error.is_some());
    until(&mut h, "the trust prompt", |a| a.trust.is_some());
    h.run_steps(2);
    h.get_by_label("Trust this SSH server?");
    h.get_by_label("Trust").click();
    until(&mut h, "trusting", |a| a.trust.is_none());
    for _ in 0..20 {
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let out = run_tab(&mut h, "select 'through the tunnel' as how");
    assert!(out.error.is_none(), "{:?}", out.error);
    assert_eq!(
        out.results[0].rows[0][0].as_deref(),
        Some("through the tunnel")
    );
}
