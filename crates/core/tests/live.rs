//! Against real servers. Set `DATADROP_TEST_POSTGRES_URL` and/or
//! `DATADROP_TEST_MYSQL_URL` (e.g. `postgres://postgres:pw@127.0.0.1:5432/postgres`);
//! without them these tests pass without doing anything. CI runs both on Linux.

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
    let var = match driver {
        Driver::Postgres => "DATADROP_TEST_POSTGRES_URL",
        Driver::Mysql => "DATADROP_TEST_MYSQL_URL",
    };
    let Ok(url) = std::env::var(var) else {
        eprintln!("{var} isn't set; skipping");
        return None;
    };
    let dir = tempfile::tempdir().unwrap();
    let client = Client::new(&ConfigDir::new(dir.path()));
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
        Driver::Mysql => String::new(),
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
    };
    assert!(server_version.contains(expect), "{server_version}");

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
    assert!(columns[0].primary_key && !columns[0].nullable);
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
    // The session is still usable.
    let q = ok(run(&db.client, "test", "SELECT 42", Some("tab")).await);
    assert_eq!(q.results[0].rows[0][0].as_deref(), Some("42"));
});
