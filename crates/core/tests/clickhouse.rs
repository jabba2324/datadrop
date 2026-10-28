//! Against a real ClickHouse: set `DATADROP_TEST_CLICKHOUSE_URL` (e.g.
//! `clickhouse://default:pw@127.0.0.1:8123/default`). Without it these tests pass
//! without doing anything.

use datadrop_core::{
    CancellationToken, Client, Command, ConfigDir, Driver, ObjectKind, Output, Profile,
    QueryOutput, SslMode,
};

struct Db {
    client: Client,
    _dir: tempfile::TempDir,
}

/// A client with a profile "test" on a fresh database called `name`.
async fn setup(name: &str) -> Option<Db> {
    let Ok(url) = std::env::var("DATADROP_TEST_CLICKHOUSE_URL") else {
        eprintln!("DATADROP_TEST_CLICKHOUSE_URL isn't set; skipping");
        return None;
    };
    let dir = tempfile::tempdir().unwrap();
    let client = Client::new(&ConfigDir::new(dir.path()));
    let profile = |name: &str, url: String| Profile {
        name: name.into(),
        driver: Driver::ClickHouse,
        url: Some(url),
        ssl_mode: SslMode::Disable,
        ..Profile::default()
    };
    save(&client, profile("admin", url.clone())).await;
    ok(run(
        &client,
        "admin",
        &format!("DROP DATABASE IF EXISTS {name} SYNC; CREATE DATABASE {name}"),
        None,
    )
    .await);
    let mut u = url::Url::parse(&url).unwrap();
    u.set_path(name);
    save(&client, profile("test", u.to_string())).await;
    Some(Db { client, _dir: dir })
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
            max_rows: Some(100),
            autocommit: None,
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

#[tokio::test]
async fn queries_sessions_and_limits() {
    let Some(db) = setup("dd_ch_queries").await else {
        return;
    };
    let c = &db.client;
    let Output::Connected { server_version } = c
        .execute(Command::ProfileTest {
            name: "test".into(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(
        server_version.starts_with("ClickHouse "),
        "{server_version}"
    );

    let q = ok(run(
        c,
        "test",
        "SELECT 42 AS n, 'tab\\there' AS s, NULL AS nothing, toDate('2026-03-04') AS d, \
         [1, 2] AS a, toDecimal64(1.5, 2) AS x",
        None,
    )
    .await);
    let r = &q.results[0];
    assert_eq!(r.columns[0].type_name, "UInt8");
    let row: Vec<Option<&str>> = r.rows[0].iter().map(|v| v.as_deref()).collect();
    assert_eq!(
        row,
        [
            Some("42"),
            Some("tab\there"),
            None,
            Some("2026-03-04"),
            Some("[1,2]"),
            Some("1.5")
        ]
    );

    // Statements, errors and the row limit.
    let q = run(c, "test", "SELECT 1; SELECT * FROM nope; SELECT 3", None).await;
    assert_eq!(q.results.len(), 1);
    assert!(q.error.unwrap().message.contains("nope"));
    let q = ok(run(c, "test", "SELECT number FROM numbers(1000000)", None).await);
    assert_eq!(q.results[0].rows.len(), 100);
    assert!(q.results[0].truncated);

    // A tab's session keeps settings and temporary tables.
    ok(run(
        c,
        "test",
        "CREATE TEMPORARY TABLE scratch (n UInt32); INSERT INTO scratch VALUES (7)",
        Some("tab"),
    )
    .await);
    let q = ok(run(c, "test", "SELECT n FROM scratch", Some("tab")).await);
    assert_eq!(q.results[0].rows[0][0].as_deref(), Some("7"));
}

#[tokio::test]
async fn cancels_on_the_server() {
    let Some(db) = setup("dd_ch_cancel").await else {
        return;
    };
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        stop.cancel();
    });
    let started = std::time::Instant::now();
    let Output::Query(q) = db
        .client
        .execute_cancellable(
            Command::Query {
                profile: "test".into(),
                sql: "SELECT count() FROM numbers(100000000000) WHERE sipHash64(number) = 0".into(),
                session: Some("tab".into()),
                max_rows: None,
                autocommit: None,
            },
            &cancel,
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(q.cancelled, "{q:?}");
    assert!(started.elapsed().as_secs() < 15, "{:?}", started.elapsed());
    let q = ok(run(&db.client, "test", "SELECT 'still here'", Some("tab")).await);
    assert_eq!(q.results[0].rows[0][0].as_deref(), Some("still here"));
}

#[tokio::test]
async fn browses_explains_imports_and_exports() {
    let Some(db) = setup("dd_ch_schema").await else {
        return;
    };
    let c = &db.client;
    ok(run(
        c,
        "test",
        "CREATE TABLE events (id UInt64, kind LowCardinality(String), at DateTime, \
           note Nullable(String) COMMENT 'free text', \
           day Date MATERIALIZED toDate(at), INDEX kind_idx kind TYPE set(10) GRANULARITY 4) \
         ENGINE = MergeTree ORDER BY (kind, id) COMMENT 'What happened'; \
         CREATE VIEW recent AS SELECT * FROM events WHERE at > now() - INTERVAL 1 DAY",
        None,
    )
    .await);
    let Output::Tables { tables, .. } = c
        .execute(Command::Tables {
            profile: "test".into(),
            schema: None,
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["events", "recent"]);
    let Output::Schema(m) = c
        .execute(Command::Schema {
            profile: "test".into(),
            schema: None,
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    let events = m.table("events").unwrap();
    assert_eq!(events.primary_key().unwrap().columns, ["kind", "id"]);
    assert!(events.column("note").unwrap().nullable);
    assert_eq!(
        events.column("note").unwrap().comment.as_deref(),
        Some("free text")
    );
    assert_eq!(
        events.column("day").unwrap().generated.as_deref(),
        Some("MATERIALIZED toDate(at)")
    );
    assert_eq!(events.indexes[0].name, "kind_idx");
    assert_eq!(events.comment.as_deref(), Some("What happened"));
    assert!(m.table("recent").unwrap().is_view());

    // Import, export and a plan.
    let dir = tempfile::tempdir().unwrap();
    let csv = dir.path().join("events.csv");
    let mut text = "id,kind,at,note\n".to_owned();
    for i in 0..450 {
        text.push_str(&format!("{i},k{},2026-01-01 00:00:00,\"n, {i}\"\n", i % 3));
    }
    std::fs::write(&csv, text).unwrap();
    let out = c
        .execute(Command::Import {
            profile: "test".into(),
            schema: None,
            table: "events".into(),
            path: csv,
            format: None,
            header: true,
            empty_as_null: true,
            columns: None,
        })
        .await
        .unwrap();
    assert_eq!(out, Output::Imported { rows: 450 });
    let json = dir.path().join("events.json");
    let out = c
        .execute(Command::Export {
            profile: "test".into(),
            sql: "SELECT id, note FROM events ORDER BY id".into(),
            path: json.clone(),
            format: None,
            session: None,
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        Output::Exported {
            path: json.display().to_string(),
            rows: 450
        }
    );
    let exported: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    assert_eq!(exported[449]["note"], "n, 449");
    let Output::Plan(p) = c
        .execute(Command::Explain {
            profile: "test".into(),
            sql: "SELECT kind, count() FROM events GROUP BY kind".into(),
            analyze: false,
            session: None,
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(
        !p.roots.is_empty() && !p.roots[0].children.is_empty(),
        "{p:?}"
    );

    // The schema's DDL recreates it.
    let Output::Ddl { sql } = c
        .execute(Command::Ddl {
            profile: "test".into(),
            schema: None,
            kind: ObjectKind::Schema,
            name: String::new(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(sql.contains("ENGINE = MergeTree"), "{sql}");
    ok(run(
        c,
        "admin",
        "DROP DATABASE dd_ch_schema SYNC; CREATE DATABASE dd_ch_schema",
        None,
    )
    .await);
    let q = run(c, "test", &sql, None).await;
    assert!(q.error.is_none(), "{:?}\n{sql}", q.error);
    let Output::Schema(again) = c
        .execute(Command::Schema {
            profile: "test".into(),
            schema: None,
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(again, m, "{sql}");
}
