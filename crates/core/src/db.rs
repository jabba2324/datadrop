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
use sqlx::{
    AssertSqlSafe, Column as _, Connection, Either, Executor, Row, SqlSafeStr, Statement as _,
    TypeInfo, ValueRef,
};

use crate::error::{CoreError, Result};
use crate::profile::{Driver, Profile, SslMode};
use crate::sql::{self, Dialect};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub primary_key: bool,
}

fn pg_options(p: &Profile) -> Result<PgConnectOptions> {
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

fn my_options(p: &Profile) -> Result<MySqlConnectOptions> {
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
    /// The server's id for this connection (backend pid / connection id), for
    /// cancelling a running query from another connection.
    server_id: u64,
    pub profile: Profile,
}

enum Conn {
    Pg(PgConnection),
    My(MySqlConnection),
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

fn looks_like_query(sql: &str, dialect: Dialect) -> bool {
    matches!(
        sql::first_keyword(sql, dialect).as_deref(),
        Some("SELECT" | "WITH" | "VALUES" | "TABLE" | "SHOW" | "EXPLAIN" | "DESCRIBE" | "DESC")
    )
}

/// A value as display text: the server's text form, or `0x…` hex for bytes that
/// aren't UTF-8 (MySQL binary columns).
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
    if let Ok(s) = row.try_get_unchecked::<&str, _>(i) {
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
    pub async fn connect(profile: &Profile) -> Result<Session> {
        let (conn, server_id) = match profile.driver {
            Driver::Postgres => {
                let opts = pg_options(profile)?;
                let mut c = timed(PgConnection::connect_with(&opts)).await?;
                let id = scalar_pg(&mut c, "SELECT pg_backend_pid()::text").await?;
                (Conn::Pg(c), id.parse().unwrap_or(0))
            }
            Driver::Mysql => {
                let opts = my_options(profile)?;
                let mut c = timed(MySqlConnection::connect_with(&opts)).await?;
                let id = scalar_my(&mut c, "SELECT CAST(CONNECTION_ID() AS CHAR)").await?;
                (Conn::My(c), id.parse().unwrap_or(0))
            }
        };
        Ok(Session {
            conn,
            server_id,
            profile: profile.clone(),
        })
    }

    pub fn dialect(&self) -> Dialect {
        self.profile.driver.into()
    }

    /// Run one statement, keeping at most `max_rows` rows of its result.
    pub async fn run(&mut self, sql: &str, max_rows: usize) -> Result<StatementResult> {
        let started = Instant::now();
        let (columns, rows, truncated, affected) = match &mut self.conn {
            Conn::Pg(c) => run_text!(c, sql, max_rows, Dialect::Postgres),
            Conn::My(c) => run_text!(c, sql, max_rows, Dialect::Mysql),
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

    pub async fn server_version(&mut self) -> Result<String> {
        match &mut self.conn {
            Conn::Pg(c) => scalar_pg(c, "SELECT version()").await,
            Conn::My(c) => scalar_my(c, "SELECT CONCAT('MySQL ', VERSION())").await,
        }
    }

    /// Ask the server to stop whatever this session is running (from a separate
    /// connection, as the session's own one is busy).
    pub async fn cancel_running(&self) -> Result<()> {
        cancel_on_server(&self.profile, self.server_id).await
    }

    pub async fn close(self) {
        let _ = match self.conn {
            Conn::Pg(c) => c.close().await,
            Conn::My(c) => c.close().await,
        };
    }
}

pub async fn cancel_on_server(profile: &Profile, server_id: u64) -> Result<()> {
    if server_id == 0 {
        return Ok(());
    }
    match profile.driver {
        Driver::Postgres => {
            let mut c = timed(PgConnection::connect_with(&pg_options(profile)?)).await?;
            let sql = format!("SELECT pg_cancel_backend({server_id})::text");
            scalar_pg(&mut c, &sql).await?;
            let _ = c.close().await;
        }
        Driver::Mysql => {
            let mut c = timed(MySqlConnection::connect_with(&my_options(profile)?)).await?;
            sqlx::raw_sql(AssertSqlSafe(format!("KILL QUERY {server_id}")))
                .execute(&mut c)
                .await?;
            let _ = c.close().await;
        }
    }
    Ok(())
}

async fn scalar_pg(c: &mut PgConnection, sql: &str) -> Result<String> {
    let row = sqlx::raw_sql(AssertSqlSafe(sql.to_owned()))
        .fetch_one(&mut *c)
        .await?;
    Ok(text_value(&row, 0)?.unwrap_or_default())
}

async fn scalar_my(c: &mut MySqlConnection, sql: &str) -> Result<String> {
    let row = sqlx::raw_sql(AssertSqlSafe(sql.to_owned()))
        .fetch_one(&mut *c)
        .await?;
    Ok(text_value(&row, 0)?.unwrap_or_default())
}

/// A small connection pool for browsing the schema, so that never waits behind a
/// long query in an editor tab.
#[derive(Clone)]
pub enum MetaPool {
    Pg(PgPool),
    My(MySqlPool),
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
    pub fn new(profile: &Profile) -> Result<MetaPool> {
        Ok(match profile.driver {
            Driver::Postgres => MetaPool::Pg(
                PgPoolOptions::new()
                    .max_connections(2)
                    .acquire_timeout(CONNECT_TIMEOUT)
                    .idle_timeout(Duration::from_secs(300))
                    .connect_lazy_with(pg_options(profile)?),
            ),
            Driver::Mysql => MetaPool::My(
                MySqlPoolOptions::new()
                    .max_connections(2)
                    .acquire_timeout(CONNECT_TIMEOUT)
                    .idle_timeout(Duration::from_secs(300))
                    .connect_lazy_with(my_options(profile)?),
            ),
        })
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
        })
    }

    pub async fn tables(&self, schema: &str) -> Result<Vec<TableInfo>> {
        let kind = |k: Option<String>| match k.as_deref() {
            Some("v" | "VIEW" | "SYSTEM VIEW") => TableKind::View,
            Some("m") => TableKind::MaterializedView,
            Some("f") => TableKind::ForeignTable,
            _ => TableKind::Table,
        };
        let rows: Vec<(Option<String>, Option<String>)> = match self {
            MetaPool::Pg(p) => sqlx::query(
                "SELECT c.relname::text, c.relkind::text FROM pg_class c \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'f') \
                 AND NOT c.relispartition ORDER BY c.relname",
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

    pub async fn close(&self) {
        match self {
            MetaPool::Pg(p) => p.close().await,
            MetaPool::My(p) => p.close().await,
        }
    }
}
