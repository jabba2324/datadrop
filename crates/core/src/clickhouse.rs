//! ClickHouse, over its HTTP interface. Results come back as
//! `TabSeparatedWithNamesAndTypes`: every value already text, as ClickHouse writes it.
//! A tab's session is a ClickHouse session (`session_id`), so `SET` and temporary
//! tables last; Stop runs `KILL QUERY` for the query's id.

use std::io::{BufRead, BufReader};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::db::{Column, Endpoint, StatementResult};
use crate::error::{CoreError, Result};
use crate::export::ExportWriter;
use crate::profile::{Profile, SslMode};

/// Where and how to reach the server.
#[derive(Clone)]
pub struct ChServer {
    base: String,
    user: String,
    password: String,
    database: String,
    agent: ureq::Agent,
}

pub struct ChConn {
    pub server: ChServer,
    session: String,
    /// The id of the query running now, to kill it.
    pub running: Arc<Mutex<Option<String>>>,
}

fn decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={s}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| s.to_owned())
}

fn new_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("datadrop-{t:x}-{n}-{}", std::process::id())
}

impl ChServer {
    /// From a `clickhouse://user:password@host:8123/database` URL (`https` with an SSL
    /// mode other than disable) or the fields.
    pub fn new(p: &Profile, via: &Endpoint) -> Result<Self> {
        let (mut host, mut port) = p.server();
        let mut user = p.user.clone();
        let mut password = p.password.clone().unwrap_or_default();
        let mut database = p.database.clone();
        if let Some(u) = p.url.as_deref() {
            let url = url::Url::parse(u)
                .map_err(|e| CoreError::InvalidProfile(format!("the URL isn't valid: {e}")))?;
            if !url.username().is_empty() {
                user = decode(url.username());
            }
            if let Some(pw) = url.password() {
                password = decode(pw);
            }
            let db = url.path().trim_start_matches('/');
            if !db.is_empty() {
                database = decode(db);
            }
        }
        if let Some((h, pt)) = via {
            host = h.clone();
            port = *pt;
        }
        let tls = p.ssl_mode != SslMode::Disable;
        let verify = matches!(p.ssl_mode, SslMode::VerifyCa | SslMode::VerifyFull);
        let tls_config = ureq::tls::TlsConfig::builder()
            .disable_verification(!verify)
            .build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(crate::db::CONNECT_TIMEOUT))
            .tls_config(tls_config)
            .build()
            .into();
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host
        };
        Ok(Self {
            base: format!("{}://{host}:{port}/", if tls { "https" } else { "http" }),
            user: if user.is_empty() {
                "default".into()
            } else {
                user
            },
            password,
            database: if database.is_empty() {
                "default".into()
            } else {
                database
            },
            agent,
        })
    }

    /// Send `sql`; the response, checked for an error.
    fn post(
        &self,
        sql: &str,
        session: Option<&str>,
        query_id: Option<&str>,
        params: &[(String, String)],
    ) -> Result<ureq::http::Response<ureq::Body>> {
        let mut req = self
            .agent
            .post(&self.base)
            .header("X-ClickHouse-User", &self.user)
            .header("X-ClickHouse-Key", &self.password)
            .query("database", &self.database)
            .query("default_format", "TabSeparatedWithNamesAndTypes")
            // Report errors as they happen, not after a 200 and half the rows.
            .query("wait_end_of_query", "0")
            .query("send_progress_in_http_headers", "0");
        if let Some(s) = session {
            req = req.query("session_id", s).query("session_timeout", "3600");
        }
        if let Some(q) = query_id {
            req = req.query("query_id", q);
        }
        for (k, v) in params {
            req = req.query(format!("param_{k}"), v);
        }
        let mut resp = req
            .send(sql)
            .map_err(|e| CoreError::Database(format!("couldn't reach ClickHouse: {e}")))?;
        if !resp.status().is_success() {
            let text = resp.body_mut().read_to_string().unwrap_or_default();
            return Err(CoreError::Database(clean_error(&text)));
        }
        Ok(resp)
    }
}

/// `Code: 60. DB::Exception: Unknown table ... (UNKNOWN_TABLE) (version 26.10)` without
/// the version and stack trace.
fn clean_error(text: &str) -> String {
    let first = text.lines().next().unwrap_or(text).trim();
    let first = first.split(" (version ").next().unwrap_or(first);
    first.replace("DB::Exception: ", "").trim().to_owned()
}

/// A TSV field: `\N` is NULL; `\t`, `\n`, `\\`... are escapes.
fn field(s: &str) -> Option<String> {
    if s == "\\N" {
        return None;
    }
    if !s.contains('\\') {
        return Some(s.to_owned());
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    Some(out)
}

fn split(line: &str) -> Vec<Option<String>> {
    line.split('\t').map(field).collect()
}

/// Rows read from a response: the header (names, types), then up to `max_rows` rows,
/// and whether there were more.
type Table = (Vec<Column>, Vec<Vec<Option<String>>>, bool);

fn read_table(resp: ureq::http::Response<ureq::Body>, max_rows: usize) -> Result<Option<Table>> {
    let mut body = resp.into_body();
    let reader = BufReader::new(body.as_reader());
    let mut lines = reader.lines();
    let read = |l: Option<std::io::Result<String>>| -> Result<Option<String>> {
        l.transpose()
            .map_err(|e| CoreError::Database(format!("reading ClickHouse's reply: {e}")))
    };
    let Some(names) = read(lines.next())? else {
        // No result set (CREATE, INSERT...).
        return Ok(None);
    };
    let types = read(lines.next())?.unwrap_or_default();
    let columns = names
        .split('\t')
        .zip(types.split('\t').chain(std::iter::repeat("")))
        .map(|(n, t)| Column {
            name: field(n).unwrap_or_default(),
            type_name: t.to_owned(),
        })
        .collect();
    let mut rows = vec![];
    let mut truncated = false;
    while let Some(line) = read(lines.next())? {
        // An error part-way through arrives in the body.
        if line.starts_with("Code: ") && line.contains("Exception") {
            return Err(CoreError::Database(clean_error(&line)));
        }
        if rows.len() >= max_rows {
            // Stop reading: dropping the connection ends the query.
            truncated = true;
            break;
        }
        rows.push(split(&line));
    }
    Ok(Some((columns, rows, truncated)))
}

impl ChConn {
    pub async fn connect(p: &Profile, via: &Endpoint) -> Result<ChConn> {
        let server = ChServer::new(p, via)?;
        let conn = ChConn {
            server,
            session: new_id(),
            running: Arc::default(),
        };
        // Check it answers (and the login works).
        let s = conn.server.clone();
        blocking(move || s.post("SELECT 1", None, None, &[]).map(|_| ())).await?;
        Ok(conn)
    }

    pub async fn run(&mut self, sql: &str, max_rows: usize) -> Result<StatementResult> {
        let started = Instant::now();
        let server = self.server.clone();
        let session = self.session.clone();
        let id = new_id();
        *self.running.lock().expect("running") = Some(id.clone());
        let body = sql.to_owned();
        let r = blocking(move || {
            let resp = server.post(&body, Some(&session), Some(&id), &[])?;
            let written: u64 = resp
                .headers()
                .get("X-ClickHouse-Summary")
                .and_then(|h| h.to_str().ok())
                .and_then(|h| serde_json::from_str::<serde_json::Value>(h).ok())
                .and_then(|v| v["written_rows"].as_str()?.parse().ok())
                .unwrap_or(0);
            Ok((read_table(resp, max_rows)?, written))
        })
        .await;
        self.running.lock().expect("running").take();
        let (table, written) = r?;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(match table {
            Some((columns, rows, truncated)) => StatementResult {
                sql: sql.to_owned(),
                returns_rows: true,
                columns,
                rows,
                truncated,
                rows_affected: 0,
                elapsed_ms,
            },
            None => StatementResult {
                sql: sql.to_owned(),
                returns_rows: false,
                columns: vec![],
                rows: vec![],
                truncated: false,
                rows_affected: written,
                elapsed_ms,
            },
        })
    }

    /// Run `sql`, writing every row to `w`.
    pub async fn export<W: std::io::Write + Send>(
        &mut self,
        sql: &str,
        w: &mut ExportWriter<W>,
    ) -> Result<bool> {
        // ClickHouse streams; the rows are read as they come, on a blocking thread, and
        // handed over in chunks.
        let server = self.server.clone();
        let session = self.session.clone();
        let id = new_id();
        *self.running.lock().expect("running") = Some(id.clone());
        let body = sql.to_owned();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<Vec<Vec<Option<String>>>>>(4);
        let (head_tx, head_rx) = tokio::sync::oneshot::channel::<Option<Vec<String>>>();
        let reader = tokio::task::spawn_blocking(move || {
            let resp = match server.post(&body, Some(&session), Some(&id), &[]) {
                Ok(r) => r,
                Err(e) => {
                    let _ = head_tx.send(None);
                    let _ = tx.blocking_send(Err(e));
                    return;
                }
            };
            let mut body = resp.into_body();
            let mut lines = BufReader::new(body.as_reader()).lines();
            let Some(Ok(names)) = lines.next() else {
                let _ = head_tx.send(None);
                return;
            };
            let _ = lines.next();
            let _ = head_tx.send(Some(
                names
                    .split('\t')
                    .map(|n| field(n).unwrap_or_default())
                    .collect(),
            ));
            let mut chunk = vec![];
            for line in lines {
                let line = match line {
                    Ok(l) => l,
                    Err(e) => {
                        let _ = tx.blocking_send(Err(CoreError::Database(e.to_string())));
                        return;
                    }
                };
                if line.starts_with("Code: ") && line.contains("Exception") {
                    let _ = tx.blocking_send(Err(CoreError::Database(clean_error(&line))));
                    return;
                }
                chunk.push(split(&line));
                if chunk.len() == 1000 && tx.blocking_send(Ok(std::mem::take(&mut chunk))).is_err()
                {
                    return;
                }
            }
            if !chunk.is_empty() {
                let _ = tx.blocking_send(Ok(chunk));
            }
        });
        let io = |e: std::io::Error| CoreError::io("writing the export", e);
        let result = async {
            let Ok(Some(names)) = head_rx.await else {
                // No result set, or an error (which follows on the channel).
                if let Some(Err(e)) = rx.recv().await {
                    return Err(e);
                }
                return Ok(false);
            };
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            w.columns(&names).map_err(io)?;
            while let Some(chunk) = rx.recv().await {
                for row in chunk? {
                    w.row(&row).map_err(io)?;
                }
            }
            Ok(true)
        }
        .await;
        let _ = reader.await;
        self.running.lock().expect("running").take();
        result
    }
}

/// Stop the query with this id.
pub async fn kill(server: &ChServer, query_id: &str) -> Result<()> {
    let server = server.clone();
    let sql = format!(
        "KILL QUERY WHERE query_id = '{}' SYNC",
        query_id.replace('\'', "")
    );
    blocking(move || server.post(&sql, None, None, &[]).map(|_| ())).await
}

/// A metadata query with `{p1:String}`-style parameters: its rows as text.
pub async fn text_rows(
    server: &ChServer,
    sql: &str,
    binds: &[&str],
) -> Result<Vec<Vec<Option<String>>>> {
    let server = server.clone();
    let sql = sql.to_owned();
    let params: Vec<(String, String)> = binds
        .iter()
        .enumerate()
        .map(|(i, b)| (format!("p{}", i + 1), (*b).to_owned()))
        .collect();
    blocking(move || {
        let resp = server.post(&sql, None, None, &params)?;
        Ok(read_table(resp, usize::MAX)?
            .map(|t| t.1)
            .unwrap_or_default())
    })
    .await
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::time::timeout(
        Duration::from_secs(24 * 3600),
        tokio::task::spawn_blocking(f),
    )
    .await
    .map_err(|_| CoreError::Database("ClickHouse didn't answer".into()))?
    .map_err(|e| CoreError::Database(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsv_fields() {
        assert_eq!(field("\\N"), None);
        assert_eq!(field("a\\tb\\\\c\\nd").as_deref(), Some("a\tb\\c\nd"));
        assert_eq!(
            split("1\t\\N\tx"),
            [Some("1".into()), None, Some("x".into())]
        );
        assert_eq!(
            clean_error(
                "Code: 60. DB::Exception: Unknown table expression identifier 'nope'. (UNKNOWN_TABLE) (version 26.10.3.7 (official build))\n"
            ),
            "Code: 60. Unknown table expression identifier 'nope'. (UNKNOWN_TABLE)"
        );
    }
}
