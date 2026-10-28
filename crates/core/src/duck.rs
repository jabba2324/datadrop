//! DuckDB, embedded. Its API is synchronous, so work runs on blocking threads; a
//! database file is opened once per process and each session gets a clone of that
//! connection (opening the same file twice would conflict). Query results are cast to
//! text by DuckDB itself: `SELECT COLUMNS(*)::VARCHAR FROM (query)`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use duckdb::Connection;

use crate::db::{Column, StatementResult};
use crate::error::{CoreError, Result};
use crate::export::ExportWriter;
use crate::profile::Profile;
use crate::sql::{self, Dialect};

pub struct DuckConn {
    conn: Arc<Mutex<Connection>>,
    pub interrupt: Arc<duckdb::InterruptHandle>,
}

fn err(e: duckdb::Error) -> CoreError {
    let m = e.to_string();
    if m.contains("INTERRUPT") || m.contains("Interrupted") {
        CoreError::Cancelled
    } else {
        CoreError::Database(m)
    }
}

/// The database file (or `:memory:`) a profile names.
fn path(p: &Profile) -> String {
    match p.url.as_deref() {
        Some(u) => u
            .trim_start_matches("duckdb://")
            .trim_start_matches("duckdb:")
            .to_owned(),
        None => p.database.clone(),
    }
}

/// A connection to the profile's database, from the one opened for this process.
pub fn open(p: &Profile) -> Result<Connection> {
    static OPEN: OnceLock<Mutex<HashMap<String, Connection>>> = OnceLock::new();
    let file = path(p);
    if file.is_empty() {
        return Err(CoreError::InvalidProfile(
            "choose the DuckDB database file".into(),
        ));
    }
    if file != ":memory:" && !p.create_if_missing && !PathBuf::from(&file).exists() {
        return Err(CoreError::Database(format!(
            "there's no database file at {file}"
        )));
    }
    let mut open = OPEN
        .get_or_init(Mutex::default)
        .lock()
        .expect("open databases");
    // Each :memory: profile is its own database.
    let key = if file == ":memory:" {
        format!(":memory:{}", p.name)
    } else {
        std::path::absolute(&file)
            .map(|a| a.display().to_string())
            .unwrap_or(file.clone())
    };
    if !open.contains_key(&key) {
        let c = if file == ":memory:" {
            Connection::open_in_memory()
        } else {
            Connection::open(&file)
        }
        .map_err(|e| CoreError::Database(format!("couldn't open {file}: {e}")))?;
        open.insert(key.clone(), c);
    }
    open[&key].try_clone().map_err(err)
}

/// Run `f` with the connection, on a blocking thread.
async fn with<T: Send + 'static>(
    conn: &Arc<Mutex<Connection>>,
    f: impl FnOnce(&Connection) -> Result<T> + Send + 'static,
) -> Result<T> {
    let conn = conn.clone();
    tokio::task::spawn_blocking(move || f(&conn.lock().expect("duckdb connection")))
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?
}

/// Columns, rows as text, and whether there were more than `max_rows`.
type Rows = (Vec<Column>, Vec<Vec<Option<String>>>, bool);

/// Rows as text: the wrapped query when it can be, else the values' own rendering.
fn query_text(c: &Connection, sql: &str, max_rows: usize, params: &[&str]) -> Result<Rows> {
    let wrapped = format!(
        "SELECT COLUMNS(*)::VARCHAR FROM (\n{}\n)",
        sql.trim().trim_end_matches(';')
    );
    let (mut stmt, cast) = match c.prepare(&wrapped) {
        Ok(s) => (s, true),
        Err(_) => (c.prepare(sql).map_err(err)?, false),
    };
    // Types from the query as written (the wrapped one's are all VARCHAR), without
    // running it: DESCRIBE plans it.
    let types: Vec<String> = if cast {
        c.prepare(&format!("DESCRIBE {}", sql.trim().trim_end_matches(';')))
            .ok()
            .and_then(|mut s| {
                let mut rows = s.query([]).ok()?;
                let mut out = vec![];
                while let Ok(Some(r)) = rows.next() {
                    out.push(r.get::<_, String>(1).unwrap_or_default());
                }
                Some(out)
            })
            .unwrap_or_default()
    } else {
        vec![]
    };
    let mut rows_out = vec![];
    let mut truncated = false;
    let mut rows = stmt
        .query(duckdb::params_from_iter(params.iter()))
        .map_err(err)?;
    let columns: Vec<Column> = {
        let s = rows.as_ref().expect("a statement");
        (0..s.column_count())
            .map(|i| Column {
                name: s.column_name(i).map(|n| n.to_string()).unwrap_or_default(),
                type_name: types
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("{:?}", s.column_type(i))),
            })
            .collect()
    };
    let n = columns.len();
    while let Some(row) = rows.next().map_err(err)? {
        if rows_out.len() >= max_rows {
            truncated = true;
            break;
        }
        let mut values = Vec::with_capacity(n);
        for i in 0..n {
            values.push(if cast {
                row.get::<_, Option<String>>(i).map_err(err)?
            } else {
                value_text(row.get_ref(i).map_err(err)?)
            });
        }
        rows_out.push(values);
    }
    Ok((columns, rows_out, truncated))
}

fn value_text(v: duckdb::types::ValueRef<'_>) -> Option<String> {
    use duckdb::types::ValueRef as V;
    Some(match v {
        V::Null => return None,
        V::Boolean(b) => b.to_string(),
        V::TinyInt(n) => n.to_string(),
        V::SmallInt(n) => n.to_string(),
        V::Int(n) => n.to_string(),
        V::BigInt(n) => n.to_string(),
        V::HugeInt(n) => n.to_string(),
        V::UTinyInt(n) => n.to_string(),
        V::USmallInt(n) => n.to_string(),
        V::UInt(n) => n.to_string(),
        V::UBigInt(n) => n.to_string(),
        V::Float(n) => n.to_string(),
        V::Double(n) => n.to_string(),
        V::Text(t) => String::from_utf8_lossy(t).into_owned(),
        other => format!("{:?}", other.to_owned()),
    })
}

/// Statements that change rows report how many; queries return them.
fn counts_rows(sql: &str) -> bool {
    matches!(
        sql::first_keyword(sql, Dialect::Postgres).as_deref(),
        Some("INSERT" | "UPDATE" | "DELETE" | "MERGE")
    ) && !sql.to_ascii_uppercase().contains("RETURNING")
}

impl DuckConn {
    pub async fn connect(p: &Profile) -> Result<DuckConn> {
        let p = p.clone();
        let conn = tokio::task::spawn_blocking(move || open(&p))
            .await
            .map_err(|e| CoreError::Database(e.to_string()))??;
        let interrupt = conn.interrupt_handle();
        Ok(DuckConn {
            conn: Arc::new(Mutex::new(conn)),
            interrupt,
        })
    }

    pub async fn run(&mut self, sql: &str, max_rows: usize) -> Result<StatementResult> {
        let started = Instant::now();
        let text = sql.to_owned();
        let (columns, rows, truncated, affected) = with(&self.conn, move |c| {
            if counts_rows(&text) || !crate::db::looks_like_query(&text, Dialect::Postgres) {
                // DDL, SET, PRAGMA...: run it; a result (PRAGMA, SHOW) is still shown.
                if counts_rows(&text) {
                    let n = c.execute(&text, []).map_err(err)?;
                    return Ok((None, vec![], false, n as u64));
                }
                let mut stmt = c.prepare(&text).map_err(err)?;
                let mut rows = stmt.query([]).map_err(err)?;
                let s = rows.as_ref().expect("a statement");
                let n = s.column_count();
                // DDL reports a single "Count" (or nothing): not worth a grid.
                let names: Vec<String> = (0..n)
                    .map(|i| s.column_name(i).map(|x| x.to_string()).unwrap_or_default())
                    .collect();
                if n == 0 || names == ["Count"] {
                    while rows.next().map_err(err)?.is_some() {}
                    return Ok((None, vec![], false, 0));
                }
                let columns = names
                    .into_iter()
                    .map(|name| Column {
                        name,
                        type_name: String::new(),
                    })
                    .collect();
                let mut out = vec![];
                let mut truncated = false;
                while let Some(row) = rows.next().map_err(err)? {
                    if out.len() >= max_rows {
                        truncated = true;
                        break;
                    }
                    let mut values = vec![];
                    for i in 0..n {
                        values.push(value_text(row.get_ref(i).map_err(err)?));
                    }
                    out.push(values);
                }
                return Ok((Some(columns), out, truncated, 0));
            }
            let (columns, rows, truncated) = query_text(c, &text, max_rows, &[])?;
            Ok((Some(columns), rows, truncated, 0))
        })
        .await?;
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

    pub async fn export<W: std::io::Write + Send>(
        &mut self,
        sql: &str,
        w: &mut ExportWriter<W>,
    ) -> Result<bool> {
        // Read in a blocking thread, a chunk of rows at a time.
        let (tx, mut rx) =
            tokio::sync::mpsc::channel::<Result<(Vec<String>, Vec<Vec<Option<String>>>)>>(4);
        let text = sql.to_owned();
        let conn = self.conn.clone();
        let reader = tokio::task::spawn_blocking(move || {
            let c = conn.lock().expect("duckdb connection");
            let run = || -> Result<()> {
                let wrapped = format!(
                    "SELECT COLUMNS(*)::VARCHAR FROM (\n{}\n)",
                    text.trim().trim_end_matches(';')
                );
                let mut stmt = c.prepare(&wrapped).map_err(err)?;
                let mut rows = stmt.query([]).map_err(err)?;
                let s = rows.as_ref().expect("a statement");
                let names: Vec<String> = (0..s.column_count())
                    .map(|i| s.column_name(i).map(|x| x.to_string()).unwrap_or_default())
                    .collect();
                let n = names.len();
                let mut chunk = vec![];
                let mut first = true;
                while let Some(row) = rows.next().map_err(err)? {
                    let mut values = vec![];
                    for i in 0..n {
                        values.push(row.get::<_, Option<String>>(i).map_err(err)?);
                    }
                    chunk.push(values);
                    if chunk.len() == 1000 {
                        let names = if first { names.clone() } else { vec![] };
                        first = false;
                        if tx
                            .blocking_send(Ok((names, std::mem::take(&mut chunk))))
                            .is_err()
                        {
                            return Ok(());
                        }
                    }
                }
                let names = if first { names } else { vec![] };
                let _ = tx.blocking_send(Ok((names, chunk)));
                Ok(())
            };
            if let Err(e) = run() {
                let _ = tx.blocking_send(Err(e));
            }
        });
        let io = |e: std::io::Error| CoreError::io("writing the export", e);
        let mut header = false;
        let mut result = Ok(());
        while let Some(chunk) = rx.recv().await {
            match chunk {
                Ok((names, rows)) => {
                    if !header {
                        let names: Vec<&str> = names.iter().map(String::as_str).collect();
                        w.columns(&names).map_err(io)?;
                        header = true;
                    }
                    for r in rows {
                        w.row(&r).map_err(io)?;
                    }
                }
                Err(e) => result = Err(e),
            }
        }
        let _ = reader.await;
        result.map(|()| header)
    }

    pub async fn plan_json(&mut self, sql: &str, analyze: bool) -> Result<String> {
        let text = format!(
            "EXPLAIN {}(FORMAT JSON) {sql}",
            if analyze { "ANALYZE " } else { "" }
        );
        with(&self.conn, move |c| {
            let mut stmt = c.prepare(&text).map_err(err)?;
            let mut rows = stmt.query([]).map_err(err)?;
            let mut out = String::new();
            while let Some(row) = rows.next().map_err(err)? {
                // (explain_key, explain_value)
                if let Some(v) = value_text(row.get_ref(1).map_err(err)?) {
                    out = v;
                }
            }
            Ok(out)
        })
        .await
    }

    pub async fn server_version(&mut self) -> Result<String> {
        let r = self.run("SELECT 'DuckDB ' || version()", 1).await?;
        Ok(r.rows
            .into_iter()
            .next()
            .and_then(|r| r.into_iter().next().flatten())
            .unwrap_or_default())
    }
}

/// A metadata query (with `?` parameters) on the profile's database.
pub async fn text_rows(p: &Profile, sql: &str, binds: &[&str]) -> Result<Vec<Vec<Option<String>>>> {
    let p = p.clone();
    let sql = sql.to_owned();
    let binds: Vec<String> = binds.iter().map(|b| (*b).to_owned()).collect();
    tokio::task::spawn_blocking(move || {
        let c = open(&p)?;
        let binds: Vec<&str> = binds.iter().map(String::as_str).collect();
        Ok(query_text(&c, &sql, usize::MAX, &binds)?.1)
    })
    .await
    .map_err(|e| CoreError::Database(e.to_string()))?
}
