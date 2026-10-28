//! SQL Server, through Microsoft's `mssql-tds`. Values come typed rather than as
//! text, so they're written out here the way SQL Server's own tools show them.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use mssql_tds::connection::client_context::ClientContext;
use mssql_tds::connection::tds_client::{
    ExecuteOptions, ResultSet, StatementResult as Next, TdsClient,
};
use mssql_tds::connection_provider::tds_connection_provider::TdsConnectionProvider;
use mssql_tds::core::{CancelHandle, EncryptionOptions, EncryptionSetting};
use mssql_tds::datatypes::column_values::ColumnValues;

use crate::db::{Column, Endpoint, StatementResult};
use crate::error::{CoreError, Result};
use crate::export::ExportWriter;
use crate::profile::{Profile, SslMode};

/// The request running on a connection, to cancel it from elsewhere.
pub type CancelSlot = Arc<Mutex<Option<Arc<CancelHandle>>>>;

pub struct MsConn {
    pub client: TdsClient,
    pub cancel: CancelSlot,
}

fn err(e: mssql_tds::error::Error) -> CoreError {
    use mssql_tds::error::Error as E;
    match e {
        E::OperationCancelledError(_) => CoreError::Cancelled,
        e => CoreError::Database(e.to_string()),
    }
}

fn decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={s}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| s.to_owned())
}

/// The login from the profile: a `sqlserver://user:password@host:port/database` URL
/// or the fields.
fn context(p: &Profile, via: &Endpoint) -> Result<(ClientContext, String)> {
    let mut c = ClientContext::default();
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
    c.user_name = user;
    c.password = password;
    c.database = if database.is_empty() {
        "master".into()
    } else {
        database
    };
    c.application_name = "DataDrop".into();
    c.connect_timeout = crate::db::CONNECT_TIMEOUT.as_secs() as u32;
    let mut tls = EncryptionOptions::new();
    let (mode, trust) = match p.ssl_mode {
        SslMode::Disable => (EncryptionSetting::PreferOff, true),
        SslMode::Prefer => (EncryptionSetting::On, true),
        SslMode::Require => (EncryptionSetting::Required, true),
        SslMode::VerifyCa | SslMode::VerifyFull => (EncryptionSetting::Required, false),
    };
    tls.mode = mode;
    tls.trust_server_certificate = trust;
    // A certificate given is the server's own, to match exactly.
    tls.server_certificate = p.ssl_root_cert.clone();
    c.encryption_options = tls;
    // One address at a time (a tunnel's local end has just the one).
    c.multi_subnet_failover = Some(false);
    Ok((c, format!("tcp:{host},{port}")))
}

pub async fn connect(p: &Profile, via: &Endpoint) -> Result<MsConn> {
    let (ctx, source) = context(p, via)?;
    // mssql-tds's futures are large: on the heap, not the stack.
    let client = Box::pin(TdsConnectionProvider {}.create_client(ctx, &source, None))
        .await
        .map_err(|e| CoreError::Database(format!("couldn't connect to SQL Server: {e}")))?;
    Ok(MsConn {
        client,
        cancel: Arc::default(),
    })
}

/// A value as SQL Server's tools write it; `None` for NULL. (`time_nanoseconds` is
/// in 100 ns units, SQL Server's tick, whatever the column's scale.)
pub fn text(v: &ColumnValues) -> Option<String> {
    use ColumnValues as V;
    Some(match v {
        V::Null => return None,
        V::TinyInt(n) => n.to_string(),
        V::SmallInt(n) => n.to_string(),
        V::Int(n) => n.to_string(),
        V::BigInt(n) => n.to_string(),
        V::Real(n) => n.to_string(),
        V::Float(n) => n.to_string(),
        V::Decimal(d) | V::Numeric(d) => d.to_string(),
        V::Bit(b) => if *b { "1" } else { "0" }.into(),
        V::String(s) => s.to_utf8_string(),
        V::Uuid(u) => u.to_string().to_uppercase(),
        V::Bytes(b) => {
            let mut s = String::with_capacity(2 + b.len() * 2);
            s.push_str("0x");
            for byte in b {
                s.push_str(&format!("{byte:02X}"));
            }
            s
        }
        V::Xml(x) => x.as_string(),
        V::Json(j) => j.as_string(),
        V::Vector(v) => match v.as_f32() {
            Some(values) => format!(
                "[{}]",
                values
                    .iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            None => format!("{v:?}"),
        },
        V::Money(m) => money(((m.msb_part as i64) << 32) | (m.lsb_part as u32 as i64)),
        V::SmallMoney(m) => money(m.int_val as i64),
        V::Date(d) => date(d.get_days() as i64 - DAYS_0001_TO_1970),
        V::Time(t) => time(t.time_nanoseconds * 100, t.scale),
        V::DateTime2(dt) => format!(
            "{} {}",
            date(dt.days as i64 - DAYS_0001_TO_1970),
            time(dt.time.time_nanoseconds * 100, dt.time.scale)
        ),
        V::DateTimeOffset(o) => {
            // Stored in UTC; shown at its own offset.
            let dt = &o.datetime2;
            let total = dt.days as i128 * DAY_NS as i128
                + dt.time.time_nanoseconds as i128 * 100
                + o.offset as i128 * 60_000_000_000;
            let days = total.div_euclid(DAY_NS as i128) as i64;
            let ns = total.rem_euclid(DAY_NS as i128) as u64;
            let (sign, off) = if o.offset < 0 {
                ('-', -(o.offset as i32))
            } else {
                ('+', o.offset as i32)
            };
            format!(
                "{} {} {sign}{:02}:{:02}",
                date(days - DAYS_0001_TO_1970),
                time(ns, dt.time.scale),
                off / 60,
                off % 60
            )
        }
        V::DateTime(dt) => {
            // 1/300 s ticks, to milliseconds as SQL Server rounds them.
            let ms = (dt.time as u64 * 10 + 1) / 3;
            format!(
                "{} {}",
                date(dt.days as i64 - DAYS_1900_TO_1970),
                time(ms * 1_000_000, 3)
            )
        }
        V::SmallDateTime(dt) => format!(
            "{} {}",
            date(dt.days as i64 - DAYS_1900_TO_1970),
            time(dt.time as u64 * 60_000_000_000, 0)
        ),
    })
}

const DAYS_0001_TO_1970: i64 = 719_162;
const DAYS_1900_TO_1970: i64 = 25_567;
const DAY_NS: u64 = 86_400_000_000_000;

fn money(scaled: i64) -> String {
    let sign = if scaled < 0 { "-" } else { "" };
    let a = scaled.unsigned_abs();
    format!("{sign}{}.{:04}", a / 10_000, a % 10_000)
}

/// `yyyy-mm-dd` for days since 1970-01-01 (Howard Hinnant's civil-from-days).
fn date(z: i64) -> String {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// `hh:mm:ss` with `scale` digits of fraction.
fn time(ns: u64, scale: u8) -> String {
    let secs = ns / 1_000_000_000;
    let mut s = format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60);
    if scale > 0 {
        let frac = format!("{:09}", ns % 1_000_000_000);
        s.push('.');
        s.push_str(&frac[..(scale as usize).min(9)]);
    }
    s
}

impl MsConn {
    /// A new cancel handle for the request about to run.
    fn arm(&self) -> Arc<CancelHandle> {
        let h = Arc::new(CancelHandle::new());
        *self.cancel.lock().expect("cancel slot") = Some(h.clone());
        h
    }

    fn disarm(&self) {
        self.cancel.lock().expect("cancel slot").take();
    }

    fn columns(&self) -> Vec<Column> {
        self.client
            .get_metadata()
            .iter()
            .map(|c| Column {
                name: c.column_name.clone(),
                type_name: format!("{:?}", c.data_type),
            })
            .collect()
    }

    /// Run a batch: every result set and row count it returns, at most `max_rows`
    /// rows of each kept.
    pub async fn run_batch(&mut self, sql: &str, max_rows: usize) -> Result<Vec<StatementResult>> {
        let handle = self.arm();
        let r = self.run_inner(sql, max_rows, &handle).await;
        self.disarm();
        r
    }

    async fn run_inner(
        &mut self,
        sql: &str,
        max_rows: usize,
        handle: &CancelHandle,
    ) -> Result<Vec<StatementResult>> {
        let started = Instant::now();
        let opts = ExecuteOptions {
            cancel: Some(handle),
            ..ExecuteOptions::default()
        };
        let mut out = vec![];
        let mut next = Box::pin(self.client.execute(sql.to_owned(), opts))
            .await
            .map_err(err)?;
        loop {
            match next {
                Next::End => break,
                Next::NoRows { rows_affected } => out.push(StatementResult {
                    sql: sql.to_owned(),
                    returns_rows: false,
                    columns: vec![],
                    rows: vec![],
                    truncated: false,
                    rows_affected: rows_affected.unwrap_or(0),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                }),
                Next::Rows => {
                    let columns = self.columns();
                    let mut rows = vec![];
                    let mut truncated = false;
                    while let Some(row) = Box::pin(self.client.next_row()).await.map_err(err)? {
                        if rows.len() >= max_rows {
                            // The rest is drained by advance().
                            truncated = true;
                            break;
                        }
                        rows.push(row.iter().map(text).collect());
                    }
                    out.push(StatementResult {
                        sql: sql.to_owned(),
                        returns_rows: true,
                        columns,
                        rows,
                        truncated,
                        rows_affected: 0,
                        elapsed_ms: started.elapsed().as_millis() as u64,
                    });
                }
            }
            next = Box::pin(self.client.advance()).await.map_err(err)?;
        }
        Ok(out)
    }

    /// Run a batch, writing the rows of its last result set to `w`.
    pub async fn export<W: std::io::Write + Send>(
        &mut self,
        sql: &str,
        w: &mut ExportWriter<W>,
    ) -> Result<bool> {
        let handle = self.arm();
        let io = |e: std::io::Error| CoreError::io("writing the export", e);
        let opts = ExecuteOptions {
            cancel: Some(&handle),
            ..ExecuteOptions::default()
        };
        let r = async {
            let mut header = false;
            let mut next = Box::pin(self.client.execute(sql.to_owned(), opts))
                .await
                .map_err(err)?;
            loop {
                match next {
                    Next::End => break,
                    Next::NoRows { .. } => {}
                    Next::Rows => {
                        if !header {
                            let names: Vec<String> =
                                self.columns().into_iter().map(|c| c.name).collect();
                            let names: Vec<&str> = names.iter().map(String::as_str).collect();
                            w.columns(&names).map_err(io)?;
                            header = true;
                        }
                        while let Some(row) = Box::pin(self.client.next_row()).await.map_err(err)? {
                            let values: Vec<Option<String>> = row.iter().map(text).collect();
                            w.row(&values).map_err(io)?;
                        }
                    }
                }
                next = Box::pin(self.client.advance()).await.map_err(err)?;
            }
            Ok(header)
        }
        .await;
        self.disarm();
        r
    }

    pub async fn close(mut self) {
        let _ = Box::pin(self.client.close_connection()).await;
    }
}

/// Cancel the request running on a connection (SQL Server's attention signal).
pub fn cancel(slot: &CancelSlot) {
    if let Some(h) = slot.lock().expect("cancel slot").as_ref() {
        h.cancel();
    }
}

/// A string as a T-SQL literal: `N'it''s'`.
pub fn literal(s: &str) -> String {
    format!("N'{}'", s.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_money() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(-DAYS_0001_TO_1970), "0001-01-01");
        assert_eq!(date(-DAYS_1900_TO_1970), "1900-01-01");
        assert_eq!(date(20_454), "2026-01-01");
        assert_eq!(time(3_723_000_000_000 + 4_500_000, 3), "01:02:03.004");
        assert_eq!(time(59 * 60 * 1_000_000_000, 0), "00:59:00");
        assert_eq!(money(-12_345_678), "-1234.5678");
        assert_eq!(literal("it's"), "N'it''s'");
    }
}
