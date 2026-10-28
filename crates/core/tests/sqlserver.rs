//! Against a real SQL Server: set `DATADROP_TEST_MSSQL_URL` (e.g.
//! `sqlserver://sa:pw@127.0.0.1:1433/master`) to a login that can create databases.
//! Without it these tests pass without doing anything.

#![cfg(feature = "sqlserver")]

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
    let Ok(url) = std::env::var("DATADROP_TEST_MSSQL_URL") else {
        eprintln!("DATADROP_TEST_MSSQL_URL isn't set; skipping");
        return None;
    };
    let dir = tempfile::tempdir().unwrap();
    let client = Client::new(&ConfigDir::new(dir.path()));
    let profile = |name: &str, url: String| Profile {
        name: name.into(),
        driver: Driver::SqlServer,
        url: Some(url),
        ssl_mode: SslMode::Prefer,
        ..Profile::default()
    };
    save(&client, profile("admin", url.clone())).await;
    ok(run(
        &client,
        "admin",
        &format!(
            "IF DB_ID('{name}') IS NOT NULL BEGIN \
               ALTER DATABASE [{name}] SET SINGLE_USER WITH ROLLBACK IMMEDIATE; \
               DROP DATABASE [{name}]; END\nGO\nCREATE DATABASE [{name}]"
        ),
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
            max_rows: None,
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

fn cell(q: &QueryOutput, result: usize, row: usize, col: usize) -> Option<&str> {
    q.results[result].rows[row][col].as_deref()
}

#[tokio::test]
async fn values_batches_and_sessions() {
    let Some(db) = setup("dd_ms_values").await else {
        return;
    };
    let c = &db.client;
    let out = c
        .execute(Command::ProfileTest {
            name: "test".into(),
        })
        .await
        .unwrap();
    let Output::Connected { server_version } = out else {
        panic!("{out:?}")
    };
    assert!(
        server_version.starts_with("Microsoft SQL Server"),
        "{server_version}"
    );

    // Values as SQL Server's tools show them.
    let q = ok(run(
        c,
        "test",
        "SELECT CAST(42 AS int), CAST(1.50 AS decimal(5,2)), CAST(1 AS bit), N'héllo', \
         CAST('2026-03-04' AS date), CAST('2026-03-04 05:06:07.1234567' AS datetime2), \
         CAST('2026-03-04 05:06:07.123' AS datetime), \
         CAST('2026-03-04 05:06:07 +02:00' AS datetimeoffset(0)), CAST(12.3456 AS money), \
         CAST('6F9619FF-8B86-D011-B42D-00C04FC964FF' AS uniqueidentifier), \
         CAST(0x0102FF AS varbinary(10)), CAST(NULL AS int), CAST('12:34:56' AS time(0))",
        None,
    )
    .await);
    let row: Vec<Option<&str>> = q.results[0].rows[0].iter().map(|v| v.as_deref()).collect();
    assert_eq!(
        row,
        [
            Some("42"),
            Some("1.50"),
            Some("1"),
            Some("héllo"),
            Some("2026-03-04"),
            Some("2026-03-04 05:06:07.1234567"),
            Some("2026-03-04 05:06:07.123"),
            Some("2026-03-04 05:06:07 +02:00"),
            Some("12.3456"),
            Some("6F9619FF-8B86-D011-B42D-00C04FC964FF"),
            Some("0x0102FF"),
            None,
            Some("12:34:56"),
        ]
    );

    // A batch keeps its variables, and returns each of its result sets.
    let q = ok(run(
        c,
        "test",
        "DECLARE @n int = 20;\nSELECT @n AS n;\nSELECT @n * 2 AS twice\nGO\nSELECT 'next batch'",
        None,
    )
    .await);
    assert_eq!(q.results.len(), 3, "{q:?}");
    assert_eq!(cell(&q, 1, 0, 0), Some("40"));
    assert_eq!(cell(&q, 2, 0, 0), Some("next batch"));

    // Errors stop the script; a tab's session keeps its temp tables.
    let q = run(
        c,
        "test",
        "SELECT 1\nGO\nSELECT * FROM nope\nGO\nSELECT 3",
        None,
    )
    .await;
    assert_eq!(q.results.len(), 1);
    let e = q.error.unwrap();
    assert!(e.message.contains("nope"), "{}", e.message);
    ok(run(
        c,
        "test",
        "CREATE TABLE #t (n int); INSERT INTO #t VALUES (7)",
        Some("tab"),
    )
    .await);
    let q = ok(run(c, "test", "SELECT n FROM #t", Some("tab")).await);
    assert_eq!(cell(&q, 0, 0, 0), Some("7"));

    // Row limits.
    let out = c
        .execute(Command::Query {
            profile: "test".into(),
            sql: "SELECT TOP 50 object_id FROM sys.all_objects".into(),
            session: None,
            max_rows: Some(10),
            autocommit: None,
        })
        .await
        .unwrap();
    let Output::Query(q) = out else { panic!() };
    assert_eq!(q.results[0].rows.len(), 10);
    assert!(q.results[0].truncated);
}

#[tokio::test]
async fn cancels_on_the_server() {
    let Some(db) = setup("dd_ms_cancel").await else {
        return;
    };
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        stop.cancel();
    });
    let started = std::time::Instant::now();
    let Output::Query(q) = db
        .client
        .execute_cancellable(
            Command::Query {
                profile: "test".into(),
                sql: "WAITFOR DELAY '00:00:20'".into(),
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
    assert!(started.elapsed().as_secs() < 10);
    // The session still works.
    let q = ok(run(&db.client, "test", "SELECT 'still here'", Some("tab")).await);
    assert_eq!(cell(&q, 0, 0, 0), Some("still here"));
}

#[tokio::test]
async fn browses_explains_and_recreates() {
    let Some(db) = setup("dd_ms_schema").await else {
        return;
    };
    let c = &db.client;
    ok(run(
        c,
        "test",
        "CREATE TABLE customers (id int IDENTITY(1,1) PRIMARY KEY, \
           email nvarchar(100) NOT NULL UNIQUE, name nvarchar(50) DEFAULT N'anon');\n\
         CREATE TABLE orders (id int PRIMARY KEY, \
           customer_id int NOT NULL REFERENCES customers (id) ON DELETE CASCADE, \
           total decimal(10,2) CHECK (total >= 0), placed datetime2 DEFAULT SYSDATETIME(), \
           doubled AS (total * 2));\n\
         CREATE INDEX orders_placed ON orders (placed DESC) INCLUDE (total);\n\
         EXEC sp_addextendedproperty N'MS_Description', N'People', N'SCHEMA', N'dbo', \
           N'TABLE', N'customers'\n\
         GO\nCREATE VIEW big AS SELECT id, total FROM orders WHERE total > 100\n\
         GO\nCREATE FUNCTION twice (@x int) RETURNS int AS BEGIN RETURN @x * 2; END\n\
         GO\nCREATE TRIGGER orders_touch ON orders AFTER UPDATE AS \
           UPDATE orders SET placed = SYSDATETIME() WHERE id IN (SELECT id FROM inserted)\n\
         GO\nCREATE SEQUENCE tickets START WITH 100\n\
         GO\nINSERT INTO customers (email) VALUES (N'a@x'), (N'b@x');\
         INSERT INTO orders (id, customer_id, total) VALUES (1, 1, 150), (2, 2, 20)",
        None,
    )
    .await);

    let Output::Schemas { schemas } = c
        .execute(Command::Schemas {
            profile: "test".into(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(schemas.contains(&"dbo".to_owned()), "{schemas:?}");
    let Output::Columns { columns, .. } = c
        .execute(Command::Columns {
            profile: "test".into(),
            schema: None,
            table: "customers".into(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(columns[1].data_type, "nvarchar(100)");
    assert!(columns[0].primary_key && !columns[1].nullable);

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
    let orders = m.table("orders").unwrap();
    let fk = orders.foreign_keys().next().unwrap();
    let r = fk.references.as_ref().unwrap();
    assert_eq!(
        (r.table.as_str(), r.on_delete.as_str()),
        ("customers", "CASCADE")
    );
    assert!(orders.column("doubled").unwrap().generated.is_some());
    assert_eq!(orders.indexes[0].name, "orders_placed");
    assert_eq!(orders.triggers[0].name, "orders_touch");
    let customers = m.table("customers").unwrap();
    assert_eq!(customers.comment.as_deref(), Some("People"));
    assert_eq!(
        customers.column("id").unwrap().identity.as_deref(),
        Some("IDENTITY(1,1)")
    );
    assert_eq!(m.routines[0].name, "twice");
    assert_eq!(m.routines[0].returns.as_deref(), Some("int"));
    assert_eq!(m.sequences[0].name, "tickets");
    assert!(m.table("big").unwrap().is_view());

    // Plans.
    let Output::Plan(p) = c
        .execute(Command::Explain {
            profile: "test".into(),
            sql: "SELECT o.id, c.email FROM orders o JOIN customers c ON c.id = o.customer_id"
                .into(),
            analyze: false,
            session: None,
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(
        !p.roots.is_empty() && p.roots[0].estimated_rows.is_some(),
        "{p:?}"
    );
    let Output::Plan(p) = c
        .execute(Command::Explain {
            profile: "test".into(),
            sql: "SELECT * FROM orders".into(),
            analyze: true,
            session: None,
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(p.analyzed && p.roots[0].actual_rows.is_some(), "{p:?}");

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
    let Some(other) = setup("dd_ms_schema_copy").await else {
        return;
    };
    let q = run(&other.client, "test", &sql, None).await;
    assert!(q.error.is_none(), "{:?}\n{sql}", q.error);
    let Output::Schema(again) = other
        .client
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

#[tokio::test]
async fn edits_imports_and_transactions() {
    use datadrop_core::command::TransactionAction;
    use datadrop_core::edit::{self, Edits};
    let Some(db) = setup("dd_ms_edits").await else {
        return;
    };
    let c = &db.client;
    ok(run(
        c,
        "test",
        "CREATE TABLE pets (id int PRIMARY KEY, name nvarchar(20), age int); \
         INSERT INTO pets VALUES (1, N'Rex', 3), (2, N'Tom', 5)",
        None,
    )
    .await);
    // Grid edits.
    let select = "SELECT id, name, age FROM pets ORDER BY id";
    let q = ok(run(c, "test", select, None).await);
    let dialect = Driver::SqlServer.into();
    let source = edit::source_table(select, dialect).expect("editable");
    let columns: Vec<String> = q.results[0]
        .columns
        .iter()
        .map(|c| c.name.clone())
        .collect();
    let mut e = Edits::default();
    e.cells.insert((0, 1), Some("Rex's".into()));
    e.deleted.insert(1);
    let statements = edit::statements(
        dialect,
        &source,
        &["id".into()],
        &columns,
        &q.results[0].rows,
        &e,
    )
    .unwrap();
    c.execute(Command::ApplyEdits {
        profile: "test".into(),
        statements,
    })
    .await
    .unwrap();
    let q = ok(run(c, "test", select, None).await);
    assert_eq!(q.results[0].rows.len(), 1);
    assert_eq!(cell(&q, 0, 0, 1), Some("Rex's"));

    // Import.
    let dir = tempfile::tempdir().unwrap();
    let csv = dir.path().join("pets.csv");
    let mut text = "id,name,age\n".to_owned();
    for i in 10..260 {
        text.push_str(&format!("{i},\"Pet {i}\",{}\n", i % 15));
    }
    std::fs::write(&csv, text).unwrap();
    let out = c
        .execute(Command::Import {
            profile: "test".into(),
            schema: None,
            table: "pets".into(),
            path: csv,
            format: None,
            header: true,
            empty_as_null: true,
            columns: None,
        })
        .await
        .unwrap();
    assert_eq!(out, Output::Imported { rows: 250 });

    // Manual commit.
    let count = || async {
        let q = ok(run(c, "test", "SELECT COUNT(*) FROM pets", None).await);
        q.results[0].rows[0][0].clone().unwrap()
    };
    c.execute(Command::SessionAutocommit {
        session: "tab".into(),
        autocommit: false,
    })
    .await
    .unwrap();
    let q = ok(run(c, "test", "DELETE FROM pets WHERE id >= 100", Some("tab")).await);
    assert!(q.in_transaction);
    c.execute(Command::Transaction {
        profile: "test".into(),
        session: "tab".into(),
        action: TransactionAction::Rollback,
    })
    .await
    .unwrap();
    assert_eq!(count().await, "251");

    // Changing the table: a column renamed, one with a new default, one added.
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
    let old = m.table("pets").unwrap().clone();
    let mut new = old.clone();
    new.columns[1].name = "label".into();
    new.columns[2].default = Some("((1))".into());
    new.columns[2].data_type = "bigint".into();
    let mut born = new.columns[2].clone();
    born.name = "born".into();
    born.data_type = "date".into();
    born.default = None;
    new.columns.push(born);
    let out = c
        .execute(Command::AlterTable {
            profile: "test".into(),
            edit: datadrop_core::diff::TableEdit {
                schema: "dbo".into(),
                old: Some(old),
                new,
                renamed_columns: vec![("name".into(), "label".into())],
            },
            apply: true,
        })
        .await
        .unwrap();
    let Output::Migration { migration, .. } = out else {
        panic!()
    };
    let q = ok(run(
        c,
        "test",
        "INSERT INTO pets (id) VALUES (999); SELECT label, age, born FROM pets WHERE id = 999",
        None,
    )
    .await);
    let last = q.results.len() - 1;
    assert_eq!(
        q.results[last].rows[0],
        [None, Some("1".into()), None],
        "{}",
        migration.script()
    );
}
