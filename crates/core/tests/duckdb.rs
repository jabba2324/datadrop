//! DuckDB, embedded: a database file in a temporary folder.

#![cfg(feature = "duckdb")]

use datadrop_core::{
    CancellationToken, Client, Command, ConfigDir, Driver, ObjectKind, Output, Profile, QueryOutput,
};

fn client(dir: &std::path::Path, name: &str) -> Client {
    let c = Client::new(&ConfigDir::new(dir));
    let rt = tokio::runtime::Handle::current();
    let p = Profile {
        name: name.into(),
        driver: Driver::DuckDb,
        database: dir.join(format!("{name}.duckdb")).display().to_string(),
        create_if_missing: true,
        ..Profile::default()
    };
    tokio::task::block_in_place(|| {
        rt.block_on(c.execute(Command::ProfileSave {
            profile: p,
            previous_name: None,
            overwrite: true,
        }))
    })
    .unwrap();
    c
}

async fn run(c: &Client, profile: &str, sql: &str, session: Option<&str>) -> QueryOutput {
    match c
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

#[tokio::test(flavor = "multi_thread")]
async fn queries_edits_and_plans() {
    let dir = tempfile::tempdir().unwrap();
    let c = client(dir.path(), "duck");
    let Output::Connected { server_version } = c
        .execute(Command::ProfileTest {
            name: "duck".into(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(server_version.starts_with("DuckDB "), "{server_version}");

    // Values as DuckDB writes them.
    let q = ok(run(
        &c,
        "duck",
        "SELECT 42 AS n, 1.5::DECIMAL(5,2) AS d, DATE '2026-03-04' AS day, [1, 2] AS l, \
         {'a': 1} AS s, NULL AS nothing, 'x''y' AS t",
        None,
    )
    .await);
    let row: Vec<Option<&str>> = q.results[0].rows[0].iter().map(|v| v.as_deref()).collect();
    assert_eq!(
        row,
        [
            Some("42"),
            Some("1.50"),
            Some("2026-03-04"),
            Some("[1, 2]"),
            Some("{'a': 1}"),
            None,
            Some("x'y")
        ]
    );
    assert_eq!(q.results[0].columns[1].type_name, "DECIMAL(5,2)");

    // Tables, counts, the row limit, a session's state.
    let q = ok(run(
        &c,
        "duck",
        "CREATE TABLE pets (id INTEGER PRIMARY KEY, name VARCHAR NOT NULL, age INTEGER DEFAULT 0); \
         INSERT INTO pets VALUES (1, 'Rex', 3), (2, 'Tom', 5); \
         SELECT * FROM range(1000)",
        None,
    )
    .await);
    assert_eq!(q.results[1].rows_affected, 2);
    assert!(q.results[2].truncated && q.results[2].rows.len() == 100);
    ok(run(&c, "duck", "SET VARIABLE v = 7", Some("tab")).await);
    let q = ok(run(&c, "duck", "SELECT getvariable('v')", Some("tab")).await);
    assert_eq!(q.results[0].rows[0][0].as_deref(), Some("7"));

    // Edits in a transaction, through the same file.
    c.execute(Command::ApplyEdits {
        profile: "duck".into(),
        statements: vec!["UPDATE \"pets\" SET \"name\" = 'Rex''s' WHERE \"id\" = '1'".into()],
    })
    .await
    .unwrap();
    let q = ok(run(&c, "duck", "SELECT name FROM pets WHERE id = 1", None).await);
    assert_eq!(q.results[0].rows[0][0].as_deref(), Some("Rex's"));

    // Plans, estimated and analyzed.
    for analyze in [false, true] {
        let Output::Plan(p) = c
            .execute(Command::Explain {
                profile: "duck".into(),
                sql: "SELECT name, count(*) FROM pets GROUP BY name".into(),
                analyze,
                session: None,
            })
            .await
            .unwrap()
        else {
            panic!()
        };
        assert!(!p.roots.is_empty(), "{p:?}");
        assert_eq!(p.analyzed, analyze);
    }

    // Stopping a long query.
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        stop.cancel();
    });
    let Output::Query(q) = c
        .execute_cancellable(
            Command::Query {
                profile: "duck".into(),
                sql: "SELECT count(*) FROM range(100000000000) a WHERE hash(a.range) = 0".into(),
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
    let q = ok(run(&c, "duck", "SELECT 'still here'", Some("tab")).await);
    assert_eq!(q.results[0].rows[0][0].as_deref(), Some("still here"));
}

#[tokio::test(flavor = "multi_thread")]
async fn browses_and_recreates_the_schema() {
    let dir = tempfile::tempdir().unwrap();
    let c = client(dir.path(), "duck");
    ok(run(
        &c,
        "duck",
        "CREATE TABLE owners (id INTEGER PRIMARY KEY, email VARCHAR UNIQUE); \
         CREATE TABLE pets (id INTEGER PRIMARY KEY, owner INTEGER REFERENCES owners (id), \
           name VARCHAR CHECK (length(name) > 0), born DATE DEFAULT DATE '2026-01-01'); \
         CREATE INDEX pets_name ON pets (name); \
         CREATE VIEW named AS SELECT name FROM pets; \
         CREATE SEQUENCE tickets START 100; \
         COMMENT ON TABLE pets IS 'Animals'",
        None,
    )
    .await);
    let Output::Schema(m) = c
        .execute(Command::Schema {
            profile: "duck".into(),
            schema: None,
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    let pets = m.table("pets").unwrap();
    assert_eq!(pets.primary_key().unwrap().columns, ["id"]);
    let fk = pets.foreign_keys().next().unwrap();
    assert_eq!(fk.references.as_ref().unwrap().table, "owners");
    assert!(
        pets.constraints
            .iter()
            .any(|c| c.definition.starts_with("CHECK"))
    );
    assert_eq!(pets.indexes[0].name, "pets_name");
    assert_eq!(pets.comment.as_deref(), Some("Animals"));
    assert!(m.table("named").unwrap().is_view());
    assert_eq!(m.sequences[0].name, "tickets");

    let Output::Ddl { sql } = c
        .execute(Command::Ddl {
            profile: "duck".into(),
            schema: None,
            kind: ObjectKind::Schema,
            name: String::new(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    let copy = client(dir.path(), "copy");
    let q = run(&copy, "copy", &sql, None).await;
    assert!(q.error.is_none(), "{:?}\n{sql}", q.error);
    let Output::Schema(again) = copy
        .execute(Command::Schema {
            profile: "copy".into(),
            schema: None,
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    let names = |m: &datadrop_core::SchemaModel| -> Vec<(String, usize, usize)> {
        m.tables
            .iter()
            .map(|t| (t.name.clone(), t.columns.len(), t.constraints.len()))
            .collect()
    };
    assert_eq!(names(&again), names(&m), "{sql}");
}
