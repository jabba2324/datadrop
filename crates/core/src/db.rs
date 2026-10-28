//! Talking to the databases: connecting, running SQL, reading the schema.
//!
//! Queries run over each driver's *text* protocol, so every value arrives as the
//! server's own text for it (what `psql` or `mysql` would print), whatever its type.

use std::str::FromStr;
use std::time::{Duration, Instant};

use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use sqlx::mysql::{
    MySqlConnectOptions, MySqlConnection, MySqlPool, MySqlPoolOptions, MySqlSslMode,
};
use sqlx::postgres::{PgConnectOptions, PgConnection, PgPool, PgPoolOptions, PgSslMode};
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection, SqlitePool, SqlitePoolOptions};
use sqlx::{
    AssertSqlSafe, Column as _, Connection, Either, Executor, Row, SqlSafeStr, Statement as _,
    TypeInfo, ValueRef,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{CoreError, Result};
use crate::export::ExportWriter;
use crate::plan::{self, Plan};
use crate::profile::{Driver, Profile, SslMode};
use crate::sql::{self, Dialect};

/// The first value of the first row, as text.
macro_rules! scalar {
    ($conn:expr, $sql:expr) => {
        async {
            let row = sqlx::raw_sql(AssertSqlSafe(String::from($sql)))
                .fetch_one($conn)
                .await?;
            Ok::<_, CoreError>(text_value(&row, 0)?.unwrap_or_default())
        }
    };
}

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    /// The server's name for the type (`int4`, `VARCHAR`, ...).
    pub type_name: String,
}

/// What one statement produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatementResult {
    pub sql: String,
    /// Whether it returned a result set (possibly empty) rather than just a count.
    pub returns_rows: bool,
    pub columns: Vec<Column>,
    /// Values as text; `None` is SQL NULL.
    pub rows: Vec<Vec<Option<String>>>,
    /// More rows came back than the row limit; only the first ones are kept.
    pub truncated: bool,
    pub rows_affected: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableKind {
    Table,
    View,
    MaterializedView,
    ForeignTable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableInfo {
    pub name: String,
    pub kind: TableKind,
}

/// A table or view and its column names, for autocompletion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogTable {
    pub schema: String,
    pub name: String,
    pub view: bool,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub primary_key: bool,
}

/// Where to connect instead of the profile's own host and port (an SSH tunnel's
/// local end).
pub type Endpoint = Option<(String, u16)>;

fn pg_options(p: &Profile, via: &Endpoint) -> Result<PgConnectOptions> {
    let mut o = match &p.url {
        Some(url) => PgConnectOptions::from_str(url)?,
        None => {
            let mut o = PgConnectOptions::new_without_pgpass()
                .host(if p.host.is_empty() {
                    "localhost"
                } else {
                    &p.host
                })
                .port(p.port.unwrap_or(Driver::Postgres.default_port()));
            if !p.database.is_empty() {
                o = o.database(&p.database);
            }
            o
        }
    };
    let (url_user, url_pass) = url_credentials(p);
    if !url_user && !p.user.is_empty() {
        o = o.username(&p.user);
    }
    if !url_pass && let Some(pw) = &p.password {
        o = o.password(pw);
    }
    if let Some((host, port)) = via {
        o = o.host(host).port(*port);
    }
    o = o.application_name("DataDrop").ssl_mode(match p.ssl_mode {
        SslMode::Disable => PgSslMode::Disable,
        SslMode::Prefer => PgSslMode::Prefer,
        SslMode::Require => PgSslMode::Require,
        SslMode::VerifyCa => PgSslMode::VerifyCa,
        SslMode::VerifyFull => PgSslMode::VerifyFull,
    });
    if let Some(ca) = &p.ssl_root_cert {
        o = o.ssl_root_cert(ca);
    }
    Ok(o)
}

fn my_options(p: &Profile, via: &Endpoint) -> Result<MySqlConnectOptions> {
    let mut o = match &p.url {
        Some(url) => MySqlConnectOptions::from_str(url)?,
        None => {
            let mut o = MySqlConnectOptions::new()
                .host(if p.host.is_empty() {
                    "localhost"
                } else {
                    &p.host
                })
                .port(p.port.unwrap_or(Driver::Mysql.default_port()));
            if !p.database.is_empty() {
                o = o.database(&p.database);
            }
            o
        }
    };
    let (url_user, url_pass) = url_credentials(p);
    if !url_user && !p.user.is_empty() {
        o = o.username(&p.user);
    }
    if !url_pass && let Some(pw) = &p.password {
        o = o.password(pw);
    }
    if let Some((host, port)) = via {
        o = o.host(host).port(*port);
    }
    o = o.charset("utf8mb4").ssl_mode(match p.ssl_mode {
        SslMode::Disable => MySqlSslMode::Disabled,
        SslMode::Prefer => MySqlSslMode::Preferred,
        SslMode::Require => MySqlSslMode::Required,
        SslMode::VerifyCa => MySqlSslMode::VerifyCa,
        SslMode::VerifyFull => MySqlSslMode::VerifyIdentity,
    });
    if let Some(ca) = &p.ssl_root_cert {
        o = o.ssl_ca(ca);
    }
    Ok(o)
}

fn lite_options(p: &Profile) -> Result<SqliteConnectOptions> {
    let o = match &p.url {
        Some(url) => SqliteConnectOptions::from_str(url)?,
        None => {
            if p.database.is_empty() {
                return Err(CoreError::InvalidProfile(
                    "choose the SQLite database file".into(),
                ));
            }
            SqliteConnectOptions::new().filename(&p.database)
        }
    };
    Ok(o.create_if_missing(p.create_if_missing)
        .busy_timeout(Duration::from_secs(5)))
}

/// Whether the profile's URL already carries a user name and a password.
fn url_credentials(p: &Profile) -> (bool, bool) {
    match p.url.as_deref().and_then(|u| url::Url::parse(u).ok()) {
        Some(u) => (!u.username().is_empty(), u.password().is_some()),
        None => (false, false),
    }
}

async fn timed<T>(fut: impl Future<Output = Result<T, sqlx::Error>>) -> Result<T> {
    match tokio::time::timeout(CONNECT_TIMEOUT, fut).await {
        Ok(r) => Ok(r?),
        Err(_) => Err(CoreError::Database(format!(
            "timed out connecting after {} seconds",
            CONNECT_TIMEOUT.as_secs()
        ))),
    }
}

/// One open connection, kept for an editor tab so `SET`, temporary tables and
/// transactions carry over between runs.
pub struct Session {
    conn: Conn,
    stop: Stopper,
    pub profile: Profile,
    /// A transaction is open (as far as the statements run on it say).
    pub in_transaction: bool,
    /// It's CockroachDB, speaking PostgreSQL's protocol.
    cockroach: bool,
}

enum Conn {
    Pg(PgConnection),
    My(MySqlConnection),
    Lite(SqliteConnection),
    #[cfg(feature = "sqlserver")]
    Ms(Box<crate::mssql::MsConn>),
}

/// How to stop a running statement.
enum Stopper {
    /// Ask the server, from another connection, to cancel the statement running on
    /// the connection with this id (backend pid / connection id).
    Server { via: Endpoint, id: u64 },
    /// SQLite checks this flag every few thousand steps and stops when it's set.
    Flag(Arc<AtomicBool>),
    /// SQL Server's attention signal, sent on the session's own connection.
    #[cfg(feature = "sqlserver")]
    Tds(crate::mssql::CancelSlot),
}

macro_rules! run_text {
    ($conn:expr, $sql:expr, $max_rows:expr, $dialect:expr) => {{
        let conn = $conn;
        let mut columns: Option<Vec<Column>> = None;
        let mut rows = Vec::new();
        let mut truncated = false;
        let mut affected = 0u64;
        {
            let mut stream = sqlx::raw_sql(AssertSqlSafe($sql.to_owned())).fetch_many(&mut *conn);
            while let Some(item) = stream.try_next().await? {
                match item {
                    Either::Left(done) => affected += done.rows_affected(),
                    Either::Right(row) => {
                        if columns.is_none() {
                            columns = Some(
                                row.columns()
                                    .iter()
                                    .map(|c| Column {
                                        name: c.name().to_owned(),
                                        type_name: c.type_info().name().to_owned(),
                                    })
                                    .collect(),
                            );
                        }
                        if rows.len() < $max_rows {
                            let mut values = Vec::with_capacity(row.len());
                            for i in 0..row.len() {
                                values.push(text_value(&row, i)?);
                            }
                            rows.push(values);
                        } else {
                            truncated = true;
                        }
                    }
                }
            }
        }
        // No rows means no column list from the server; for a query, prepare it to
        // learn the columns (best effort).
        if columns.is_none() && looks_like_query($sql, $dialect) {
            if let Ok(st) = conn
                .prepare(AssertSqlSafe($sql.to_owned()).into_sql_str())
                .await
            {
                let cols: Vec<Column> = st
                    .columns()
                    .iter()
                    .map(|c| Column {
                        name: c.name().to_owned(),
                        type_name: c.type_info().name().to_owned(),
                    })
                    .collect();
                if !cols.is_empty() {
                    columns = Some(cols);
                }
            }
        }
        (columns, rows, truncated, affected)
    }};
}

/// Stream a statement's rows into an export. Returns whether it was a result set.
macro_rules! export_text {
    ($conn:expr, $sql:expr, $w:expr, $dialect:expr) => {{
        let conn = $conn;
        let w = $w;
        let io = |e: std::io::Error| CoreError::io("writing the export", e);
        let mut header = false;
        {
            let mut stream = sqlx::raw_sql(AssertSqlSafe($sql.to_owned())).fetch_many(&mut *conn);
            while let Some(item) = stream.try_next().await? {
                if let Either::Right(row) = item {
                    if !header {
                        let names: Vec<&str> = row.columns().iter().map(|c| c.name()).collect();
                        w.columns(&names).map_err(io)?;
                        header = true;
                    }
                    let mut values = Vec::with_capacity(row.len());
                    for i in 0..row.len() {
                        values.push(text_value(&row, i)?);
                    }
                    w.row(&values).map_err(io)?;
                }
            }
        }
        if !header && looks_like_query($sql, $dialect) {
            if let Ok(st) = conn
                .prepare(AssertSqlSafe($sql.to_owned()).into_sql_str())
                .await
                && !st.columns().is_empty()
            {
                let names: Vec<&str> = st.columns().iter().map(|c| c.name()).collect();
                w.columns(&names).map_err(io)?;
                header = true;
            }
        }
        header
    }};
}

pub(crate) fn looks_like_query(sql: &str, dialect: Dialect) -> bool {
    matches!(
        sql::first_keyword(sql, dialect).as_deref(),
        Some("SELECT" | "WITH" | "VALUES" | "TABLE" | "SHOW" | "EXPLAIN" | "DESCRIBE" | "DESC")
    )
}

/// A value as display text: the server's text form, or `0x…` hex for bytes that
/// aren't UTF-8 (MySQL binary columns) and SQLite BLOBs.
fn text_value<R: Row>(row: &R, i: usize) -> Result<Option<String>>
where
    usize: sqlx::ColumnIndex<R>,
    for<'r> &'r str: sqlx::Decode<'r, R::Database>,
    for<'r> Vec<u8>: sqlx::Decode<'r, R::Database>,
{
    let raw = row.try_get_raw(i)?;
    if raw.is_null() {
        return Ok(None);
    }
    // SQLite would happily read a BLOB as text; show its bytes instead.
    let blob =
        raw.type_info().name() == "BLOB" && <R::Database as sqlx::Database>::NAME == "SQLite";
    if !blob && let Ok(s) = row.try_get_unchecked::<&str, _>(i) {
        return Ok(Some(s.to_owned()));
    }
    let bytes: Vec<u8> = row.try_get_unchecked(i)?;
    let mut hex = String::with_capacity(2 + bytes.len() * 2);
    hex.push_str("0x");
    for b in bytes {
        hex.push_str(&format!("{b:02x}"));
    }
    Ok(Some(hex))
}

impl Session {
    /// Connect with `profile`, through `via` (an SSH tunnel) if given.
    pub async fn connect(profile: &Profile, via: Endpoint) -> Result<Session> {
        let mut cockroach = false;
        let (conn, stop) = match profile.driver {
            Driver::Postgres => {
                let opts = pg_options(profile, &via)?;
                let mut c = timed(PgConnection::connect_with(&opts)).await?;
                let id = scalar!(&mut c, "SELECT pg_backend_pid()::text").await?;
                let id = id.parse().unwrap_or(0);
                let version = scalar!(&mut c, "SELECT version()").await?;
                cockroach = version.starts_with("CockroachDB");
                (Conn::Pg(c), Stopper::Server { via, id })
            }
            Driver::Mysql => {
                let opts = my_options(profile, &via)?;
                let mut c = timed(MySqlConnection::connect_with(&opts)).await?;
                let id = scalar!(&mut c, "SELECT CAST(CONNECTION_ID() AS CHAR)").await?;
                let id = id.parse().unwrap_or(0);
                (Conn::My(c), Stopper::Server { via, id })
            }
            Driver::Sqlite => {
                let opts = lite_options(profile)?;
                let mut c = timed(SqliteConnection::connect_with(&opts)).await?;
                let flag = Arc::new(AtomicBool::new(false));
                let check = flag.clone();
                c.lock_handle()
                    .await?
                    .set_progress_handler(10_000, move || !check.load(Ordering::Relaxed));
                (Conn::Lite(c), Stopper::Flag(flag))
            }
            #[cfg(feature = "sqlserver")]
            Driver::SqlServer => {
                let c = timed_ms(crate::mssql::connect(profile, &via)).await?;
                let slot = c.cancel.clone();
                (Conn::Ms(Box::new(c)), Stopper::Tds(slot))
            }
            #[cfg(not(feature = "sqlserver"))]
            Driver::SqlServer => return Err(no_sql_server()),
        };
        Ok(Session {
            conn,
            stop,
            profile: profile.clone(),
            in_transaction: false,
            cockroach,
        })
    }

    pub fn dialect(&self) -> Dialect {
        self.profile.driver.into()
    }

    /// Run what `sql::batches` makes one request: every result it returns. (One,
    /// except for a SQL Server batch.)
    pub async fn run_batch(&mut self, sql: &str, max_rows: usize) -> Result<Vec<StatementResult>> {
        #[cfg(feature = "sqlserver")]
        if let Conn::Ms(c) = &mut self.conn {
            return c.run_batch(sql, max_rows).await;
        }
        Ok(vec![self.run(sql, max_rows).await?])
    }

    /// Run one statement, keeping at most `max_rows` rows of its result.
    pub async fn run(&mut self, sql: &str, max_rows: usize) -> Result<StatementResult> {
        let started = Instant::now();
        let (columns, rows, truncated, affected) = match &mut self.conn {
            Conn::Pg(c) => run_text!(c, sql, max_rows, Dialect::Postgres),
            Conn::My(c) => run_text!(c, sql, max_rows, Dialect::Mysql),
            Conn::Lite(c) => run_text!(c, sql, max_rows, Dialect::Sqlite),
            #[cfg(feature = "sqlserver")]
            Conn::Ms(c) => {
                // The batch's last result set (or, without one, its row counts).
                let results = c.run_batch(sql, max_rows).await?;
                let affected = results.iter().map(|r| r.rows_affected).sum();
                return Ok(match results.into_iter().rev().find(|r| r.returns_rows) {
                    Some(r) => r,
                    None => StatementResult {
                        sql: sql.to_owned(),
                        returns_rows: false,
                        columns: vec![],
                        rows: vec![],
                        truncated: false,
                        rows_affected: affected,
                        elapsed_ms: started.elapsed().as_millis() as u64,
                    },
                });
            }
        };
        Ok(StatementResult {
            sql: sql.to_owned(),
            returns_rows: columns.is_some(),
            columns: columns.unwrap_or_default(),
            rows,
            truncated,
            rows_affected: affected,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }

    /// The plan for one statement. With `analyze` the statement really runs.
    pub async fn explain(&mut self, sql: &str, analyze: bool) -> Result<Plan> {
        let first_value = |r: StatementResult| -> Result<String> {
            r.rows
                .into_iter()
                .flatten()
                .flatten()
                .next()
                .ok_or_else(|| CoreError::Database("the database returned no plan".into()))
        };
        match self.profile.driver {
            Driver::Postgres if self.cockroach => {
                let how = if analyze {
                    "EXPLAIN ANALYZE"
                } else {
                    "EXPLAIN"
                };
                let r = self.run(&format!("{how} {sql}"), 100_000).await?;
                let lines: Vec<String> = r
                    .rows
                    .into_iter()
                    .map(|row| row.into_iter().next().flatten().unwrap_or_default())
                    .collect();
                plan::parse_cockroach(&lines, analyze)
            }
            Driver::Postgres => {
                let opts = if analyze {
                    "FORMAT JSON, ANALYZE, BUFFERS"
                } else {
                    "FORMAT JSON"
                };
                let r = self.run(&format!("EXPLAIN ({opts}) {sql}"), 1).await?;
                plan::parse_postgres(&first_value(r)?, analyze)
            }
            Driver::Mysql => {
                let how = if analyze { "ANALYZE" } else { "FORMAT=TREE" };
                let r = self.run(&format!("EXPLAIN {how} {sql}"), 1).await?;
                plan::parse_mysql_tree(&first_value(r)?, analyze)
            }
            Driver::Sqlite => {
                if analyze {
                    return Err(CoreError::Invalid(
                        "SQLite can't time each step of a query; use Explain without analyze"
                            .into(),
                    ));
                }
                let r = self
                    .run(&format!("EXPLAIN QUERY PLAN {sql}"), 10_000)
                    .await?;
                let rows: Vec<(i64, i64, String)> = r
                    .rows
                    .into_iter()
                    .filter_map(|row| {
                        let id = row.first()?.as_deref()?.parse().ok()?;
                        let parent = row.get(1)?.as_deref()?.parse().ok()?;
                        let detail = row.get(3)?.clone()?;
                        Some((id, parent, detail))
                    })
                    .collect();
                plan::parse_sqlite(&rows)
            }
            Driver::SqlServer => {
                // SHOWPLAN_ALL describes the plan (without running it); STATISTICS
                // PROFILE runs the statement and adds the plan with actual rows.
                let set = if analyze {
                    "STATISTICS PROFILE"
                } else {
                    "SHOWPLAN_ALL"
                };
                self.run_batch(&format!("SET {set} ON"), 1).await?;
                let r = self.run_batch(sql, 100_000).await;
                let _ = self.run_batch(&format!("SET {set} OFF"), 1).await;
                let results = r?;
                let profile = results
                    .iter()
                    .rev()
                    .find(|r| r.columns.iter().any(|c| c.name == "NodeId"))
                    .ok_or_else(|| CoreError::Database("SQL Server returned no plan".into()))?;
                plan::parse_sqlserver(profile, analyze)
            }
        }
    }

    /// Run one statement, writing all its rows to `w`. Returns whether it returned a
    /// result set.
    pub async fn export<W: std::io::Write + Send>(
        &mut self,
        sql: &str,
        w: &mut ExportWriter<W>,
    ) -> Result<bool> {
        Ok(match &mut self.conn {
            Conn::Pg(c) => export_text!(c, sql, w, Dialect::Postgres),
            Conn::My(c) => export_text!(c, sql, w, Dialect::Mysql),
            Conn::Lite(c) => export_text!(c, sql, w, Dialect::Sqlite),
            #[cfg(feature = "sqlserver")]
            Conn::Ms(c) => c.export(sql, w).await?,
        })
    }

    pub async fn server_version(&mut self) -> Result<String> {
        match &mut self.conn {
            Conn::Pg(c) => scalar!(c, "SELECT version()").await,
            Conn::My(c) => scalar!(c, "SELECT CONCAT('MySQL ', VERSION())").await,
            Conn::Lite(c) => scalar!(c, "SELECT 'SQLite ' || sqlite_version()").await,
            #[cfg(feature = "sqlserver")]
            Conn::Ms(c) => {
                let r = c.run_batch("SELECT @@VERSION", 1).await?;
                let v = r
                    .into_iter()
                    .find_map(|r| r.rows.into_iter().next())
                    .and_then(|row| row.into_iter().next().flatten())
                    .unwrap_or_default();
                Ok(v.lines().next().unwrap_or_default().trim().to_owned())
            }
        }
    }

    /// Ask the database to stop whatever this session is running (for a server,
    /// from a separate connection, as the session's own one is busy).
    pub async fn cancel_running(&self) -> Result<()> {
        match &self.stop {
            Stopper::Server { via, id } => cancel_on_server(&self.profile, via, *id).await,
            Stopper::Flag(flag) => {
                flag.store(true, Ordering::Relaxed);
                Ok(())
            }
            #[cfg(feature = "sqlserver")]
            Stopper::Tds(slot) => {
                crate::mssql::cancel(slot);
                Ok(())
            }
        }
    }

    /// A way to stop what's running from elsewhere while it runs, for drivers that
    /// do that on the session's own connection (SQL Server).
    pub fn interrupter(&self) -> Option<Box<dyn Fn() + Send + Sync>> {
        match &self.stop {
            #[cfg(feature = "sqlserver")]
            Stopper::Tds(slot) => {
                let slot = slot.clone();
                Some(Box::new(move || crate::mssql::cancel(&slot)))
            }
            _ => None,
        }
    }

    /// After a cancel has taken effect: let the next statement run.
    pub fn clear_cancel(&self) {
        if let Stopper::Flag(flag) = &self.stop {
            flag.store(false, Ordering::Relaxed);
        }
    }

    pub async fn close(self) {
        let _ = match self.conn {
            Conn::Pg(c) => c.close().await,
            Conn::My(c) => c.close().await,
            Conn::Lite(c) => c.close().await,
            #[cfg(feature = "sqlserver")]
            Conn::Ms(c) => {
                c.close().await;
                Ok(())
            }
        };
    }
}

async fn cancel_on_server(profile: &Profile, via: &Endpoint, server_id: u64) -> Result<()> {
    if server_id == 0 {
        return Ok(());
    }
    match profile.driver {
        Driver::Postgres => {
            let opts = pg_options(profile, via)?;
            let mut c = timed(PgConnection::connect_with(&opts)).await?;
            scalar!(
                &mut c,
                &format!("SELECT pg_cancel_backend({server_id})::text")
            )
            .await?;
            let _ = c.close().await;
        }
        Driver::Mysql => {
            let opts = my_options(profile, via)?;
            let mut c = timed(MySqlConnection::connect_with(&opts)).await?;
            sqlx::raw_sql(AssertSqlSafe(format!("KILL QUERY {server_id}")))
                .execute(&mut c)
                .await?;
            let _ = c.close().await;
        }
        Driver::Sqlite | Driver::SqlServer => {}
    }
    Ok(())
}

#[cfg(not(feature = "sqlserver"))]
fn no_sql_server() -> CoreError {
    CoreError::Invalid("this DataDrop was built without SQL Server support".into())
}

#[cfg(feature = "sqlserver")]
async fn timed_ms<T>(f: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(CONNECT_TIMEOUT, f)
        .await
        .map_err(|_| CoreError::Database("timed out connecting to SQL Server".into()))?
}

/// A small connection pool for browsing the schema, so that never waits behind a
/// long query in an editor tab.
#[derive(Clone)]
pub enum MetaPool {
    Pg(PgPool),
    My(MySqlPool),
    Lite(SqlitePool),
    /// One shared connection, opened when first needed.
    #[cfg(feature = "sqlserver")]
    Ms(Arc<MsMeta>),
}

#[cfg(feature = "sqlserver")]
pub struct MsMeta {
    profile: Profile,
    via: Endpoint,
    conn: tokio::sync::Mutex<Option<crate::mssql::MsConn>>,
}

/// The type of a SQL Server column as it's written (`nvarchar(50)`, `decimal(10,2)`),
/// from `sys.columns c` joined to `sys.types tp`.
#[cfg(feature = "sqlserver")]
pub(crate) const MS_TYPE: &str = "CASE \
    WHEN tp.name IN ('varchar', 'char', 'varbinary', 'binary') THEN tp.name + '(' + \
      CASE WHEN c.max_length = -1 THEN 'max' ELSE CAST(c.max_length AS varchar(10)) END + ')' \
    WHEN tp.name IN ('nvarchar', 'nchar') THEN tp.name + '(' + \
      CASE WHEN c.max_length = -1 THEN 'max' ELSE CAST(c.max_length / 2 AS varchar(10)) END + ')' \
    WHEN tp.name IN ('decimal', 'numeric') THEN tp.name + '(' + CAST(c.precision AS varchar(10)) \
      + ',' + CAST(c.scale AS varchar(10)) + ')' \
    WHEN tp.name IN ('datetime2', 'time', 'datetimeoffset') AND c.scale <> 7 \
      THEN tp.name + '(' + CAST(c.scale AS varchar(10)) + ')' \
    ELSE tp.name END";

fn arr<const N: usize>(r: Vec<Option<String>>) -> [Option<String>; N] {
    std::array::from_fn(|i| r.get(i).cloned().flatten())
}

/// Text of column `i`, for metadata queries that cast everything to text.
fn text_col<R: Row>(row: &R, i: usize) -> Option<String>
where
    usize: sqlx::ColumnIndex<R>,
    for<'r> String: sqlx::Decode<'r, R::Database>,
{
    if row.try_get_raw(i).ok()?.is_null() {
        return None;
    }
    row.try_get_unchecked::<String, _>(i).ok()
}

impl MetaPool {
    pub fn new(profile: &Profile, via: Endpoint) -> Result<MetaPool> {
        Ok(match profile.driver {
            Driver::Postgres => MetaPool::Pg(
                PgPoolOptions::new()
                    .max_connections(2)
                    .acquire_timeout(CONNECT_TIMEOUT)
                    .idle_timeout(Duration::from_secs(300))
                    .connect_lazy_with(pg_options(profile, &via)?),
            ),
            Driver::Mysql => MetaPool::My(
                MySqlPoolOptions::new()
                    .max_connections(2)
                    .acquire_timeout(CONNECT_TIMEOUT)
                    .idle_timeout(Duration::from_secs(300))
                    .connect_lazy_with(my_options(profile, &via)?),
            ),
            Driver::Sqlite => MetaPool::Lite(
                SqlitePoolOptions::new()
                    .max_connections(2)
                    .acquire_timeout(CONNECT_TIMEOUT)
                    .idle_timeout(Duration::from_secs(300))
                    .connect_lazy_with(lite_options(profile)?),
            ),
            #[cfg(feature = "sqlserver")]
            Driver::SqlServer => MetaPool::Ms(Arc::new(MsMeta {
                profile: profile.clone(),
                via,
                conn: tokio::sync::Mutex::default(),
            })),
            #[cfg(not(feature = "sqlserver"))]
            Driver::SqlServer => return Err(no_sql_server()),
        })
    }

    /// Run a metadata query with string parameters, every column read as text.
    pub async fn text_rows(&self, sql: &str, binds: &[&str]) -> Result<Vec<Vec<Option<String>>>> {
        macro_rules! rows {
            ($pool:expr) => {{
                let mut q = sqlx::query(AssertSqlSafe(sql.to_owned()));
                for b in binds {
                    q = q.bind(*b);
                }
                q.fetch_all($pool)
                    .await?
                    .iter()
                    .map(|r| (0..r.len()).map(|i| text_col(r, i)).collect())
                    .collect()
            }};
        }
        Ok(match self {
            MetaPool::Pg(p) => rows!(p),
            MetaPool::My(p) => rows!(p),
            MetaPool::Lite(p) => rows!(p),
            #[cfg(feature = "sqlserver")]
            MetaPool::Ms(m) => {
                // Parameters are @p1, @p2... written in as literals.
                let mut sql = sql.to_owned();
                for (i, b) in binds.iter().enumerate().rev() {
                    sql = sql.replace(&format!("@p{}", i + 1), &crate::mssql::literal(b));
                }
                let mut guard = m.conn.lock().await;
                if guard.is_none() {
                    *guard = Some(timed_ms(crate::mssql::connect(&m.profile, &m.via)).await?);
                }
                let c = guard.as_mut().expect("connected above");
                match c.run_batch(&sql, usize::MAX).await {
                    Ok(results) => results
                        .into_iter()
                        .find(|r| r.returns_rows)
                        .map(|r| r.rows)
                        .unwrap_or_default(),
                    Err(e) => {
                        // Start again next time.
                        guard.take();
                        return Err(e);
                    }
                }
            }
        })
    }

    pub fn driver(&self) -> Driver {
        match self {
            MetaPool::Pg(_) => Driver::Postgres,
            MetaPool::My(_) => Driver::Mysql,
            MetaPool::Lite(_) => Driver::Sqlite,
            #[cfg(feature = "sqlserver")]
            MetaPool::Ms(_) => Driver::SqlServer,
        }
    }

    pub async fn schemas(&self) -> Result<Vec<String>> {
        Ok(match self {
            MetaPool::Pg(p) => sqlx::query(
                "SELECT nspname::text FROM pg_namespace \
                 WHERE nspname !~ '^pg_' AND nspname <> 'information_schema' ORDER BY 1",
            )
            .fetch_all(p)
            .await?
            .iter()
            .filter_map(|r| text_col(r, 0))
            .collect(),
            MetaPool::My(p) => sqlx::query(
                "SELECT CAST(schema_name AS CHAR) FROM information_schema.schemata \
                 WHERE schema_name NOT IN ('mysql', 'performance_schema', 'sys', 'information_schema') \
                 ORDER BY 1",
            )
            .fetch_all(p)
            .await?
            .iter()
            .filter_map(|r| text_col(r, 0))
            .collect(),
            MetaPool::Lite(p) => sqlx::query(
                "SELECT name FROM pragma_database_list WHERE name <> 'temp' ORDER BY seq",
            )
            .fetch_all(p)
            .await?
            .iter()
            .filter_map(|r| text_col(r, 0))
            .collect(),
            #[cfg(feature = "sqlserver")]
            MetaPool::Ms(_) => self
                .text_rows(
                    "SELECT name FROM sys.schemas WHERE schema_id < 16384 \
                     AND name NOT IN ('sys', 'INFORMATION_SCHEMA', 'guest') ORDER BY name",
                    &[],
                )
                .await?
                .into_iter()
                .filter_map(|r| r.into_iter().next().flatten())
                .collect(),
        })
    }

    pub async fn tables(&self, schema: &str) -> Result<Vec<TableInfo>> {
        let kind = |k: Option<String>| match k.as_deref() {
            Some("v" | "V" | "VIEW" | "SYSTEM VIEW" | "view") => TableKind::View,
            Some("m") => TableKind::MaterializedView,
            Some("f") => TableKind::ForeignTable,
            _ => TableKind::Table,
        };
        let rows: Vec<(Option<String>, Option<String>)> = match self {
            MetaPool::Pg(p) => sqlx::query(
                "SELECT c.relname::text, c.relkind::text FROM pg_class c \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'f') \
                 AND NOT coalesce(c.relispartition, false) ORDER BY c.relname",
            )
            .bind(schema)
            .fetch_all(p)
            .await?
            .iter()
            .map(|r| (text_col(r, 0), text_col(r, 1)))
            .collect(),
            MetaPool::My(p) => sqlx::query(
                "SELECT CAST(table_name AS CHAR), CAST(table_type AS CHAR) \
                 FROM information_schema.tables WHERE table_schema = ? ORDER BY table_name",
            )
            .bind(schema)
            .fetch_all(p)
            .await?
            .iter()
            .map(|r| (text_col(r, 0), text_col(r, 1)))
            .collect(),
            MetaPool::Lite(p) => {
                // The schema can't be a parameter; it's quoted as an identifier.
                let sql = format!(
                    "SELECT name, type FROM {}.sqlite_master \
                     WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
                     ORDER BY name",
                    sql::quote_ident(schema, Dialect::Sqlite)
                );
                sqlx::query(AssertSqlSafe(sql))
                    .fetch_all(p)
                    .await?
                    .iter()
                    .map(|r| (text_col(r, 0), text_col(r, 1)))
                    .collect()
            }
            #[cfg(feature = "sqlserver")]
            MetaPool::Ms(_) => self
                .text_rows(
                    "SELECT o.name, RTRIM(o.type) FROM sys.objects o \
                     JOIN sys.schemas s ON s.schema_id = o.schema_id \
                     WHERE s.name = @p1 AND o.type IN ('U', 'V') AND o.is_ms_shipped = 0 \
                     ORDER BY o.name",
                    &[schema],
                )
                .await?
                .into_iter()
                .map(|r| {
                    let [a, b] = arr(r);
                    (a, b)
                })
                .collect(),
        };
        Ok(rows
            .into_iter()
            .filter_map(|(name, k)| {
                Some(TableInfo {
                    name: name?,
                    kind: kind(k),
                })
            })
            .collect())
    }

    pub async fn columns(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let rows: Vec<[Option<String>; 5]> = match self {
            MetaPool::Pg(p) => sqlx::query(
                "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod)::text, \
                 CASE WHEN a.attnotnull THEN 'NO' ELSE 'YES' END, \
                 pg_get_expr(d.adbin, d.adrelid)::text, \
                 CASE WHEN EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = c.oid \
                   AND i.indisprimary AND a.attnum = ANY(i.indkey)) THEN 'YES' ELSE 'NO' END \
                 FROM pg_attribute a \
                 JOIN pg_class c ON c.oid = a.attrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
                 WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped \
                 ORDER BY a.attnum",
            )
            .bind(schema)
            .bind(table)
            .fetch_all(p)
            .await?
            .iter()
            .map(|r| std::array::from_fn(|i| text_col(r, i)))
            .collect(),
            MetaPool::My(p) => sqlx::query(
                "SELECT CAST(column_name AS CHAR), CAST(column_type AS CHAR), \
                 CAST(is_nullable AS CHAR), CAST(column_default AS CHAR), \
                 CASE WHEN column_key = 'PRI' THEN 'YES' ELSE 'NO' END \
                 FROM information_schema.columns WHERE table_schema = ? AND table_name = ? \
                 ORDER BY ordinal_position",
            )
            .bind(schema)
            .bind(table)
            .fetch_all(p)
            .await?
            .iter()
            .map(|r| std::array::from_fn(|i| text_col(r, i)))
            .collect(),
            MetaPool::Lite(p) => sqlx::query(
                "SELECT name, type, CASE WHEN \"notnull\" THEN 'NO' ELSE 'YES' END, \
                 dflt_value, CASE WHEN pk > 0 THEN 'YES' ELSE 'NO' END \
                 FROM pragma_table_info(?1, ?2) ORDER BY cid",
            )
            .bind(table)
            .bind(schema)
            .fetch_all(p)
            .await?
            .iter()
            .map(|r| std::array::from_fn(|i| text_col(r, i)))
            .collect(),
            #[cfg(feature = "sqlserver")]
            MetaPool::Ms(_) => self
                .text_rows(
                    &format!(
                        "SELECT c.name, {MS_TYPE}, CASE WHEN c.is_nullable = 1 THEN 'YES' ELSE 'NO' END, \
                         OBJECT_DEFINITION(c.default_object_id), \
                         CASE WHEN EXISTS (SELECT 1 FROM sys.indexes i JOIN sys.index_columns ic \
                           ON ic.object_id = i.object_id AND ic.index_id = i.index_id \
                           WHERE i.object_id = c.object_id AND i.is_primary_key = 1 \
                           AND ic.column_id = c.column_id) THEN 'YES' ELSE 'NO' END \
                         FROM sys.columns c JOIN sys.types tp ON tp.user_type_id = c.user_type_id \
                         WHERE c.object_id = OBJECT_ID(QUOTENAME(@p1) + '.' + QUOTENAME(@p2)) \
                         ORDER BY c.column_id"
                    ),
                    &[schema, table],
                )
                .await?
                .into_iter()
                .map(arr)
                .collect(),
        };
        Ok(rows
            .into_iter()
            .filter_map(|[name, ty, nullable, default, pk]| {
                Some(ColumnInfo {
                    name: name?,
                    data_type: ty.unwrap_or_default(),
                    nullable: nullable.as_deref() == Some("YES"),
                    default,
                    primary_key: pk.as_deref() == Some("YES"),
                })
            })
            .collect())
    }

    /// Every table and view with its columns (for autocompletion), outside the
    /// system schemas, up to a limit that keeps huge databases responsive.
    pub async fn catalog(&self) -> Result<Vec<CatalogTable>> {
        let rows: Vec<[Option<String>; 4]> = match self {
            MetaPool::Pg(p) => sqlx::query(
                "SELECT n.nspname::text, c.relname::text, c.relkind::text, a.attname::text \
                 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                 LEFT JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped \
                 WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f') AND NOT coalesce(c.relispartition, false) \
                 AND n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' \
                 ORDER BY 1, 2, a.attnum LIMIT 100000",
            )
            .fetch_all(p)
            .await?
            .iter()
            .map(|r| std::array::from_fn(|i| text_col(r, i)))
            .collect(),
            MetaPool::My(p) => sqlx::query(
                "SELECT CAST(t.table_schema AS CHAR), CAST(t.table_name AS CHAR), \
                 CAST(t.table_type AS CHAR), CAST(c.column_name AS CHAR) \
                 FROM information_schema.tables t LEFT JOIN information_schema.columns c \
                 ON c.table_schema = t.table_schema AND c.table_name = t.table_name \
                 WHERE t.table_schema NOT IN ('mysql', 'performance_schema', 'sys', 'information_schema') \
                 ORDER BY 1, 2, c.ordinal_position LIMIT 100000",
            )
            .fetch_all(p)
            .await?
            .iter()
            .map(|r| std::array::from_fn(|i| text_col(r, i)))
            .collect(),
            MetaPool::Lite(p) => sqlx::query(
                "SELECT 'main', m.name, m.type, c.name FROM sqlite_master m \
                 LEFT JOIN pragma_table_info(m.name) c \
                 WHERE m.type IN ('table', 'view') AND m.name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
                 ORDER BY m.name, c.cid LIMIT 100000",
            )
            .fetch_all(p)
            .await?
            .iter()
            .map(|r| std::array::from_fn(|i| text_col(r, i)))
            .collect(),
            #[cfg(feature = "sqlserver")]
            MetaPool::Ms(_) => self
                .text_rows(
                    "SELECT TOP 100000 s.name, o.name, RTRIM(o.type), c.name FROM sys.objects o \
                     JOIN sys.schemas s ON s.schema_id = o.schema_id \
                     LEFT JOIN sys.columns c ON c.object_id = o.object_id \
                     WHERE o.type IN ('U', 'V') AND o.is_ms_shipped = 0 \
                     ORDER BY s.name, o.name, c.column_id",
                    &[],
                )
                .await?
                .into_iter()
                .map(arr)
                .collect(),
        };
        let mut out: Vec<CatalogTable> = vec![];
        for [schema, table, kind, column] in rows {
            let (Some(schema), Some(table)) = (schema, table) else {
                continue;
            };
            let same = out
                .last()
                .is_some_and(|t| t.schema == schema && t.name == table);
            if !same {
                let view = matches!(
                    kind.as_deref(),
                    Some("v" | "m" | "V" | "VIEW" | "SYSTEM VIEW" | "view")
                );
                out.push(CatalogTable {
                    schema,
                    name: table,
                    view,
                    columns: vec![],
                });
            }
            if let Some(c) = column {
                out.last_mut().expect("pushed above").columns.push(c);
            }
        }
        Ok(out)
    }

    pub async fn close(&self) {
        match self {
            MetaPool::Pg(p) => p.close().await,
            MetaPool::My(p) => p.close().await,
            MetaPool::Lite(p) => p.close().await,
            #[cfg(feature = "sqlserver")]
            MetaPool::Ms(m) => {
                if let Some(c) = m.conn.lock().await.take() {
                    c.close().await;
                }
            }
        }
    }
}
