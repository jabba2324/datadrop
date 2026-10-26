//! Against real databases. SQLite always runs (a file in a temporary folder). For
//! servers, set `DATADROP_TEST_POSTGRES_URL` and/or `DATADROP_TEST_MYSQL_URL` (e.g.
//! `postgres://postgres:pw@127.0.0.1:5432/postgres`); without them those tests pass
//! without doing anything. CI runs both on Linux.

use std::time::{Duration, Instant};

use datadrop_core::{
    CancellationToken, Client, Command, ConfigDir, Driver, Output, Profile, QueryOutput, TableKind,
};

struct Db {
    client: Client,
    profile: String,
    driver: Driver,
    _dir: tempfile::TempDir,
}

/// A client with a profile for the test server, and a fresh schema (PostgreSQL) or
/// database (MySQL) named `schema` that the profile uses, so tests don't collide.
async fn setup(driver: Driver, schema: &str) -> Option<Db> {
    let dir = tempfile::tempdir().unwrap();
    let client = Client::new(&ConfigDir::new(dir.path()));
    if driver == Driver::Sqlite {
        let profile = Profile {
            name: "test".into(),
            driver,
            database: dir
                .path()
                .join(format!("{schema}.db"))
                .display()
                .to_string(),
            create_if_missing: true,
            ..Profile::default()
        };
        save(&client, profile).await;
        return Some(Db {
            client,
            profile: "test".into(),
            driver,
            _dir: dir,
        });
    }
    let var = match driver {
        Driver::Postgres => "DATADROP_TEST_POSTGRES_URL",
        Driver::Mysql => "DATADROP_TEST_MYSQL_URL",
        Driver::Sqlite => unreachable!(),
    };
    let Ok(url) = std::env::var(var) else {
        eprintln!("{var} isn't set; skipping");
        return None;
    };
    let admin = Profile {
        name: "admin".into(),
        driver,
        url: Some(url.clone()),
        ..Profile::default()
    };
    save(&client, admin).await;
    let reset = match driver {
        Driver::Postgres => {
            format!("DROP SCHEMA IF EXISTS {schema} CASCADE; CREATE SCHEMA {schema}")
        }
        Driver::Mysql => format!("DROP DATABASE IF EXISTS {schema}; CREATE DATABASE {schema}"),
        Driver::Sqlite => unreachable!(),
    };
    ok(run(&client, "admin", &reset, None).await);

    // The test profile: the same server, using the new schema / database.
    let mut profile = Profile {
        name: "test".into(),
        driver,
        url: Some(url),
        ..Profile::default()
    };
    if driver == Driver::Mysql {
        let mut u = url::Url::parse(profile.url.as_deref().unwrap()).unwrap();
        u.set_path(schema);
        profile.url = Some(u.to_string());
    }
    save(&client, profile).await;
    let db = Db {
        client,
        profile: "test".into(),
        driver,
        _dir: dir,
    };
    Some(db)
}

async fn save(client: &Client, profile: Profile) {
    client
        .execute(Command::ProfileSave {
            profile,
            previous_name: None,
            overwrite: true,
        })
        .await
        .unwrap();
}

async fn run(client: &Client, profile: &str, sql: &str, session: Option<&str>) -> QueryOutput {
    match client
        .execute(Command::Query {
            profile: profile.into(),
            sql: sql.into(),
            session: session.map(Into::into),
            max_rows: None,
        })
        .await
        .unwrap()
    {
        Output::Query(q) => q,
        o => panic!("{o:?}"),
    }
}

fn ok(q: QueryOutput) -> QueryOutput {
    assert!(q.error.is_none(), "query failed: {:?}", q.error);
    q
}

/// `schema.` for PostgreSQL (MySQL tables live in the profile's database).
fn prefix(db: &Db, schema: &str) -> String {
    match db.driver {
        Driver::Postgres => format!("{schema}."),
        Driver::Mysql | Driver::Sqlite => String::new(),
    }
}

/// The schema the test's tables are in.
fn home<'a>(db: &Db, schema: &'a str) -> &'a str {
    if db.driver == Driver::Sqlite {
        "main"
    } else {
        schema
    }
}

macro_rules! both {
    ($name:ident, $body:expr) => {
        mod $name {
            use super::*;
            #[tokio::test]
            async fn postgres() {
                let f: fn(Driver, &'static str) -> _ = $body;
                f(Driver::Postgres, concat!("dd_", stringify!($name))).await;
            }
            #[tokio::test]
            async fn mysql() {
                let f: fn(Driver, &'static str) -> _ = $body;
                f(Driver::Mysql, concat!("dd_", stringify!($name))).await;
            }
            #[tokio::test]
            async fn sqlite() {
                let f: fn(Driver, &'static str) -> _ = $body;
                f(Driver::Sqlite, concat!("dd_", stringify!($name))).await;
            }
        }
    };
}

both!(connects, |driver, schema| async move {
    let Some(db) = setup(driver, schema).await else {
        return;
    };
    let Output::Connected { server_version } = db
        .client
        .execute(Command::ProfileTest {
            name: db.profile.clone(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    let expect = match driver {
        Driver::Postgres => "PostgreSQL",
        Driver::Mysql => "MySQL",
        Driver::Sqlite => "SQLite",
    };
    assert!(server_version.contains(expect), "{server_version}");
    if driver == Driver::Sqlite {
        // A missing file is an error unless the profile says to create it.
        let missing = Profile {
            name: "x".into(),
            driver,
            database: "/nonexistent/folder/none.db".into(),
            ..Profile::default()
        };
        assert!(
            db.client
                .execute(Command::ConnectionTest { profile: missing })
                .await
                .is_err()
        );
        return;
    }

    // A wrong password fails with the server's message, not a panic.
    let mut bad = db.client.store().get("test").unwrap();
    let mut u = url::Url::parse(bad.url.as_deref().unwrap()).unwrap();
    u.set_password(Some("wrong")).unwrap();
    bad.url = Some(u.to_string());
    let err = db
        .client
        .execute(Command::ConnectionTest { profile: bad })
        .await
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("password") || err.to_string().contains("denied"),
        "{err}"
    );
});

both!(values_as_text, |driver, schema| async move {
    let Some(db) = setup(driver, schema).await else {
        return;
    };
    let p = prefix(&db, schema);
    let script = match driver {
        Driver::Postgres => format!(
            "CREATE TABLE {p}t (id int PRIMARY KEY, name text, price numeric(8,2), ok boolean, \
               at timestamp, tags text[], doc jsonb, raw bytea);
             INSERT INTO {p}t VALUES (1, 'café', 12.50, true, '2026-01-02 03:04:05', '{{a,b}}', '{{\"k\": 1}}', '\\x00ff'),
                                     (2, NULL, NULL, NULL, NULL, NULL, NULL, NULL);
             SELECT * FROM {p}t ORDER BY id"
        ),
        Driver::Mysql => "CREATE TABLE t (id int PRIMARY KEY, name text, price decimal(8,2), ok boolean, \
               at datetime, doc json, raw varbinary(4));
             INSERT INTO t VALUES (1, 'café', 12.50, true, '2026-01-02 03:04:05', '{\"k\": 1}', 0x00ff),
                                  (2, NULL, NULL, NULL, NULL, NULL, NULL);
             SELECT * FROM t ORDER BY id"
            .to_owned(),
        Driver::Sqlite => "CREATE TABLE t (id integer PRIMARY KEY, name text, price numeric, ok boolean, \
               at text, doc text, raw blob);
             INSERT INTO t VALUES (1, 'café', 12.50, true, '2026-01-02 03:04:05', '{\"k\": 1}', x'00ff'),
                                  (2, NULL, NULL, NULL, NULL, NULL, NULL);
             SELECT * FROM t ORDER BY id"
            .to_owned(),
    };
    let q = ok(run(&db.client, "test", &script, None).await);
    assert_eq!(q.results.len(), 3);
    assert!(!q.results[0].returns_rows);
    assert_eq!(q.results[1].rows_affected, 2);
    let sel = &q.results[2];
    assert!(sel.returns_rows);
    let names: Vec<_> = sel.columns.iter().map(|c| c.name.as_str()).collect();
    let row = |i: usize| sel.rows[i].iter().map(|v| v.as_deref()).collect::<Vec<_>>();
    match driver {
        Driver::Postgres => {
            assert_eq!(
                names,
                ["id", "name", "price", "ok", "at", "tags", "doc", "raw"]
            );
            assert_eq!(
                row(0),
                [
                    Some("1"),
                    Some("café"),
                    Some("12.50"),
                    Some("t"),
                    Some("2026-01-02 03:04:05"),
                    Some("{a,b}"),
                    Some("{\"k\": 1}"),
                    Some("\\x00ff")
                ]
            );
            assert_eq!(sel.columns[0].type_name, "INT4");
        }
        Driver::Mysql => {
            assert_eq!(names, ["id", "name", "price", "ok", "at", "doc", "raw"]);
            assert_eq!(
                row(0),
                [
                    Some("1"),
                    Some("café"),
                    Some("12.50"),
                    Some("1"),
                    Some("2026-01-02 03:04:05"),
                    Some("{\"k\": 1}"),
                    Some("0x00ff")
                ]
            );
        }
        Driver::Sqlite => {
            assert_eq!(names, ["id", "name", "price", "ok", "at", "doc", "raw"]);
            assert_eq!(
                row(0),
                [
                    Some("1"),
                    Some("café"),
                    Some("12.5"),
                    Some("1"),
                    Some("2026-01-02 03:04:05"),
                    Some("{\"k\": 1}"),
                    Some("0x00ff")
                ]
            );
        }
    }
    assert!(row(1)[1..].iter().all(Option::is_none), "NULLs stay NULL");
});

both!(scripts_stop_at_errors, |driver, schema| async move {
    let Some(db) = setup(driver, schema).await else {
        return;
    };
    let p = prefix(&db, schema);
    let q = run(
        &db.client,
        "test",
        &format!("CREATE TABLE {p}s (n int); INSERT INTO {p}s VALUES (1); SELECT * FROM {p}nope; INSERT INTO {p}s VALUES (2)"),
        None,
    )
    .await;
    assert_eq!(q.results.len(), 2);
    let err = q.error.unwrap();
    assert_eq!(err.index, 2);
    assert!(err.message.contains("nope"), "{}", err.message);
    let q = ok(run(
        &db.client,
        "test",
        &format!("SELECT count(*) FROM {p}s"),
        None,
    )
    .await);
    assert_eq!(q.results[0].rows[0][0].as_deref(), Some("1"));
});

both!(sessions_keep_state, |driver, schema| async move {
    let Some(db) = setup(driver, schema).await else {
        return;
    };
    let tmp = "CREATE TEMPORARY TABLE scratch (n int)";
    ok(run(&db.client, "test", tmp, Some("tab-1")).await);
    ok(run(
        &db.client,
        "test",
        "INSERT INTO scratch VALUES (7)",
        Some("tab-1"),
    )
    .await);
    let q = ok(run(&db.client, "test", "SELECT n FROM scratch", Some("tab-1")).await);
    assert_eq!(q.results[0].rows, [[Some("7".to_owned())]]);
    // Another session (or none) has its own connection.
    assert!(
        run(&db.client, "test", "SELECT n FROM scratch", Some("tab-2"))
            .await
            .error
            .is_some()
    );
    assert!(
        run(&db.client, "test", "SELECT n FROM scratch", None)
            .await
            .error
            .is_some()
    );
    db.client
        .execute(Command::SessionClose {
            session: "tab-1".into(),
        })
        .await
        .unwrap();
    assert!(
        run(&db.client, "test", "SELECT n FROM scratch", Some("tab-1"))
            .await
            .error
            .is_some()
    );
});

both!(row_limit_and_empty_results, |driver, schema| async move {
    let Some(db) = setup(driver, schema).await else {
        return;
    };
    let p = prefix(&db, schema);
    let many = match driver {
        Driver::Postgres => "SELECT g AS n FROM generate_series(1, 2500) g".to_owned(),
        Driver::Mysql => {
            "WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < 2500) \
            SELECT /*+ SET_VAR(cte_max_recursion_depth = 5000) */ n FROM c"
                .to_owned()
        }
        Driver::Sqlite => {
            "WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < 2500) \
            SELECT n FROM c"
                .to_owned()
        }
    };
    let out = db
        .client
        .execute(Command::Query {
            profile: "test".into(),
            sql: many,
            session: None,
            max_rows: Some(1000),
        })
        .await
        .unwrap();
    let Output::Query(q) = out else { panic!() };
    let r = &q.results[0];
    assert_eq!(r.rows.len(), 1000);
    assert!(r.truncated);
    assert_eq!(r.rows[999][0].as_deref(), Some("1000"));

    // No rows: still a result set, with its columns.
    let q = ok(run(
        &db.client,
        "test",
        &format!("CREATE TABLE {p}e (a int, b text); SELECT a, b FROM {p}e"),
        None,
    )
    .await);
    let r = &q.results[1];
    assert!(r.returns_rows);
    assert!(r.rows.is_empty());
    let names: Vec<_> = r.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
});

both!(browses_the_schema, |driver, schema| async move {
    let Some(db) = setup(driver, schema).await else {
        return;
    };
    let p = prefix(&db, schema);
    ok(run(
        &db.client,
        "test",
        &format!(
            "CREATE TABLE {p}orders (id int PRIMARY KEY, total numeric(10,2) NOT NULL DEFAULT 0, note text);
             CREATE VIEW {p}big_orders AS SELECT * FROM {p}orders WHERE total > 100"
        ),
        None,
    )
    .await);
    let Output::Schemas { schemas } = db
        .client
        .execute(Command::Schemas {
            profile: "test".into(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    let schema = home(&db, schema);
    assert!(schemas.iter().any(|s| s == schema), "{schemas:?}");
    assert!(
        !schemas
            .iter()
            .any(|s| s == "information_schema" || s.starts_with("pg_"))
    );

    let Output::Tables { tables, .. } = db
        .client
        .execute(Command::Tables {
            profile: "test".into(),
            schema: Some(schema.into()),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    let got: Vec<_> = tables.iter().map(|t| (t.name.as_str(), t.kind)).collect();
    assert_eq!(
        got,
        [
            ("big_orders", TableKind::View),
            ("orders", TableKind::Table)
        ]
    );

    let Output::Columns { columns, .. } = db
        .client
        .execute(Command::Columns {
            profile: "test".into(),
            schema: Some(schema.into()),
            table: "orders".into(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    let names: Vec<_> = columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "total", "note"]);
    assert!(columns[0].primary_key);
    // SQLite allows NULL in a primary key unless it's exactly INTEGER PRIMARY KEY.
    assert!(!columns[0].nullable || driver == Driver::Sqlite);
    assert!(!columns[1].nullable && columns[1].default.is_some());
    assert!(columns[2].nullable && !columns[2].primary_key);
    assert!(
        columns[1].data_type.contains("10,2"),
        "{}",
        columns[1].data_type
    );
});

both!(cancels_on_the_server, |driver, schema| async move {
    let Some(db) = setup(driver, schema).await else {
        return;
    };
    let sleep = match driver {
        Driver::Postgres => "SELECT pg_sleep(30)",
        Driver::Mysql => "SELECT SLEEP(30)",
        // No sleep in SQLite: a long count instead.
        Driver::Sqlite => {
            "WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c) \
                           SELECT count(*) FROM c"
        }
    };
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(700)).await;
        stop.cancel();
    });
    let started = Instant::now();
    let out = db
        .client
        .execute_cancellable(
            Command::Query {
                profile: "test".into(),
                sql: sleep.into(),
                session: Some("tab".into()),
                max_rows: None,
            },
            &cancel,
        )
        .await
        .unwrap();
    let Output::Query(q) = out else { panic!() };
    assert!(q.cancelled);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    // The session is still usable, for long statements too.
    let q = ok(run(&db.client, "test", "SELECT 42", Some("tab")).await);
    assert_eq!(q.results[0].rows[0][0].as_deref(), Some("42"));
    let long = match driver {
        Driver::Postgres => "SELECT count(*) FROM generate_series(1, 300000)",
        Driver::Mysql => "SELECT BENCHMARK(300000, MD5('x'))",
        Driver::Sqlite => {
            "WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < 300000) \
                           SELECT count(*) FROM c"
        }
    };
    ok(run(&db.client, "test", long, Some("tab")).await);
});

both!(exports_stream_to_files, |driver, schema| async move {
    let Some(db) = setup(driver, schema).await else {
        return;
    };
    let p = prefix(&db, schema);
    let dir = tempfile::tempdir().unwrap();
    let export = |sql: String, file: &str| {
        let path = dir.path().join(file);
        let client = &db.client;
        async move {
            client
                .execute(Command::Export {
                    profile: "test".into(),
                    sql,
                    path: path.clone(),
                    format: None,
                    session: None,
                })
                .await
                .map(|o| (o, path))
        }
    };
    // A script: the table is made first, then the last statement is exported, all
    // of it (no row limit).
    let script = match driver {
        Driver::Postgres => format!(
            "CREATE TABLE {p}nums AS SELECT g AS n, 'row ' || g AS label FROM generate_series(1, 2500) g; \
             SELECT n, label FROM {p}nums ORDER BY n"
        ),
        Driver::Mysql => "CREATE TABLE nums AS WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < 2500) \
             SELECT /*+ SET_VAR(cte_max_recursion_depth = 5000) */ n, CONCAT('row ', n) AS label FROM c; \
             SELECT n, label FROM nums ORDER BY n"
            .to_owned(),
        Driver::Sqlite => "CREATE TABLE nums AS WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < 2500) \
             SELECT n, 'row ' || n AS label FROM c; \
             SELECT n, label FROM nums ORDER BY n"
            .to_owned(),
    };
    let (out, path) = export(script, "nums.csv").await.unwrap();
    assert!(
        matches!(out, Output::Exported { rows: 2500, .. }),
        "{out:?}"
    );
    let csv = std::fs::read_to_string(&path).unwrap();
    assert_eq!(csv.lines().count(), 2501);
    assert_eq!(
        csv.lines().take(2).collect::<Vec<_>>(),
        ["n,label", "1,row 1"]
    );

    let (_, path) = export(
        format!("SELECT n, label FROM {p}nums WHERE n > 2498"),
        "tail.json",
    )
    .await
    .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        json,
        serde_json::json!([{"n": "2499", "label": "row 2499"}, {"n": "2500", "label": "row 2500"}])
    );

    // No rows: just the header.
    let (out, path) = export(
        format!("SELECT n, label FROM {p}nums WHERE n < 0"),
        "none.csv",
    )
    .await
    .unwrap();
    assert!(matches!(out, Output::Exported { rows: 0, .. }));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "n,label\n");

    // Nothing to export, and no file left behind.
    let err = export(format!("DELETE FROM {p}nums WHERE n < 0"), "nothing.csv")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("doesn't return rows"), "{err}");
    assert!(!dir.path().join("nothing.csv").exists());
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        3,
        "no temporary files left"
    );
});

both!(explains_queries, |driver, schema| async move {
    let Some(db) = setup(driver, schema).await else {
        return;
    };
    let p = prefix(&db, schema);
    ok(run(
        &db.client,
        "test",
        &format!(
            "CREATE TABLE {p}things (id int PRIMARY KEY, n int); \
             INSERT INTO {p}things VALUES (1, 10), (2, 20), (3, 30)"
        ),
        None,
    )
    .await);
    let explain = |sql: String, analyze: bool| {
        let client = &db.client;
        async move {
            client
                .execute(Command::Explain {
                    profile: "test".into(),
                    sql,
                    analyze,
                    session: None,
                })
                .await
                .map(|o| match o {
                    Output::Plan(plan) => plan,
                    o => panic!("{o:?}"),
                })
        }
    };
    let plan = explain(format!("SELECT * FROM {p}things WHERE n > 15"), false)
        .await
        .unwrap();
    let text = format!("{plan:?}").to_lowercase();
    assert!(text.contains("things"), "{plan:#?}");
    assert!(!plan.analyzed);

    if driver == Driver::Sqlite {
        let err = explain(format!("SELECT * FROM {p}things"), true)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("SQLite"), "{err}");
        return;
    }
    let plan = explain(format!("SELECT * FROM {p}things WHERE n > 15"), true)
        .await
        .unwrap();
    assert!(plan.analyzed);
    assert!(plan.execution_ms.is_some(), "{plan:#?}");
    let mut node = &plan.roots[0];
    while node.actual_rows.is_none() && !node.children.is_empty() {
        node = &node.children[0];
    }
    assert!(node.actual_rows.is_some(), "{plan:#?}");

    // Analyzing a DELETE runs it, then rolls it back.
    explain(format!("DELETE FROM {p}things WHERE n > 15"), true)
        .await
        .unwrap();
    let q = ok(run(
        &db.client,
        "test",
        &format!("SELECT count(*) FROM {p}things"),
        None,
    )
    .await);
    assert_eq!(q.results[0].rows[0][0].as_deref(), Some("3"));

    // One statement at a time.
    assert!(explain("SELECT 1; SELECT 2".into(), false).await.is_err());
});

both!(
    saves_edits_in_one_transaction,
    |driver, schema| async move {
        use datadrop_core::edit::{self, Edits};
        use std::collections::BTreeMap;
        let Some(db) = setup(driver, schema).await else {
            return;
        };
        let p = prefix(&db, schema);
        ok(run(
            &db.client,
            "test",
            &format!(
                "CREATE TABLE {p}pets (id int PRIMARY KEY, name varchar(20), age int); \
             INSERT INTO {p}pets VALUES (1, 'Rex', 3), (2, 'Tom', 5), (3, 'Bo', 1)"
            ),
            None,
        )
        .await);
        let select = format!("SELECT id, name, age FROM {p}pets ORDER BY id");
        let q = ok(run(&db.client, "test", &select, None).await);
        let result = &q.results[0];
        let dialect = driver.into();
        let source = edit::source_table(&select, dialect).expect("editable");
        let columns: Vec<String> = result.columns.iter().map(|c| c.name.clone()).collect();

        let mut e = Edits::default();
        e.cells.insert((0, 1), Some("Rex's".into()));
        e.cells.insert((1, 2), None);
        e.deleted.insert(2);
        e.inserted.push(BTreeMap::from([
            (0, Some("4".into())),
            (1, Some("Kit".into())),
        ]));
        let statements =
            edit::statements(dialect, &source, &["id".into()], &columns, &result.rows, &e).unwrap();
        assert_eq!(statements.len(), 4, "{statements:#?}");
        db.client
            .execute(Command::ApplyEdits {
                profile: "test".into(),
                statements,
            })
            .await
            .unwrap();
        let q = ok(run(&db.client, "test", &select, None).await);
        let rows: Vec<Vec<Option<&str>>> = q.results[0]
            .rows
            .iter()
            .map(|r| r.iter().map(|v| v.as_deref()).collect())
            .collect();
        assert_eq!(
            rows,
            [
                vec![Some("1"), Some("Rex's"), Some("3")],
                vec![Some("2"), Some("Tom"), None],
                vec![Some("4"), Some("Kit"), None],
            ]
        );

        // A row that's gone: nothing is saved, not even the statements before it.
        let err = db
            .client
            .execute(Command::ApplyEdits {
                profile: "test".into(),
                statements: vec![
                    format!("UPDATE {p}pets SET name = 'Changed' WHERE id = '1'"),
                    format!("DELETE FROM {p}pets WHERE id = '99'"),
                ],
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("nothing was saved"), "{err}");
        let q = ok(run(&db.client, "test", &select, None).await);
        assert_eq!(q.results[0].rows[0][1].as_deref(), Some("Rex's"));
    }
);
