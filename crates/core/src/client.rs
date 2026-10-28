use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::command::{Command, DEFAULT_MAX_ROWS, Output, QueryOutput, StatementError};
use crate::complete::Catalog;
use crate::db::Endpoint;
use crate::db::{MetaPool, Session};
use crate::error::{CoreError, Result};
use crate::export::{ExportFormat, ExportWriter};
use crate::history::{self, History, HistoryEntry};
use crate::keychain::Keychain;
use crate::plan::Plan;
use crate::profile::SshTunnel;
use crate::profile::{Driver, Profile, ProfileStore};
use crate::sql::{self, Dialect};
use crate::tunnel::{self, Tunnel};

/// Where DataDrop keeps its files: `$DATADROP_HOME`, or `~/.datadrop`.
#[derive(Debug, Clone)]
pub struct ConfigDir {
    root: PathBuf,
    /// Instead of the system's keychain (for tests).
    keychain: Option<Keychain>,
}

impl ConfigDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            keychain: None,
        }
    }

    /// Keep passwords in this keychain rather than the system's.
    pub fn with_keychain(mut self, keychain: Keychain) -> Self {
        self.keychain = Some(keychain);
        self
    }

    pub fn keychain(&self) -> Keychain {
        self.keychain.clone().unwrap_or_else(Keychain::system)
    }

    pub fn default_location() -> Result<Self> {
        if let Some(dir) = std::env::var_os("DATADROP_HOME").filter(|d| !d.is_empty()) {
            return Ok(Self::new(dir));
        }
        let home = std::env::home_dir().ok_or_else(|| {
            CoreError::Invalid("couldn't find your home folder (set DATADROP_HOME)".into())
        })?;
        Ok(Self::new(home.join(".datadrop")))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn profiles(&self) -> PathBuf {
        self.root.join("profiles")
    }

    pub fn history(&self) -> PathBuf {
        self.root.join("history.jsonl")
    }

    /// SSH host keys trusted in DataDrop.
    pub fn known_hosts(&self) -> PathBuf {
        self.root.join("known_hosts")
    }
}

type SessionSlot = Arc<tokio::sync::Mutex<Option<Session>>>;

/// A tunnel, with the SSH settings and target it was opened for.
type OpenTunnel = (SshTunnel, (String, u16), Arc<Tunnel>);

/// Runs [`Command`]s. Cheap to share: clone the `Arc` it lives in.
pub struct Client {
    store: ProfileStore,
    history: History,
    known_hosts: PathBuf,
    /// Open SSH tunnels, per profile, with the settings they were opened with.
    tunnels: tokio::sync::Mutex<HashMap<String, OpenTunnel>>,
    /// Schema-browsing pools, per profile (rebuilt when the profile changes).
    pools: Mutex<HashMap<String, (Profile, Endpoint, MetaPool)>>,
    sessions: Mutex<HashMap<String, SessionSlot>>,
}

impl Client {
    pub fn new(config: &ConfigDir) -> Self {
        Self {
            store: ProfileStore::with_keychain(config.profiles(), config.keychain()),
            history: History::new(config.history()),
            known_hosts: config.known_hosts(),
            tunnels: Default::default(),
            pools: Mutex::default(),
            sessions: Mutex::default(),
        }
    }

    pub fn store(&self) -> &ProfileStore {
        &self.store
    }

    pub async fn execute(&self, cmd: Command) -> Result<Output> {
        self.execute_cancellable(cmd, &CancellationToken::new())
            .await
    }

    /// Run a command. Cancelling stops a running query on the server too.
    pub async fn execute_cancellable(
        &self,
        cmd: Command,
        cancel: &CancellationToken,
    ) -> Result<Output> {
        match cmd {
            Command::ProfileList => Ok(Output::Profiles {
                profiles: self.store.list()?.iter().map(Profile::summary).collect(),
            }),
            Command::ProfileGet {
                name,
                show_password,
            } => {
                // Without the passwords, there's no need to ask the keychain.
                let profile = if show_password {
                    self.store.get(&name)?
                } else {
                    self.store.get_without_secrets(&name)?.masked()
                };
                Ok(Output::Profile { profile })
            }
            Command::ProfileSave {
                profile,
                previous_name,
                overwrite,
            } => {
                self.store
                    .save(&profile, previous_name.as_deref(), overwrite)?;
                self.forget_pool(&profile.name).await;
                if let Some(old) = &previous_name {
                    self.forget_pool(old).await;
                }
                Ok(Output::ProfileSaved { name: profile.name })
            }
            Command::ProfileRemove { name } => {
                self.store.remove(&name)?;
                self.forget_pool(&name).await;
                Ok(Output::ProfileRemoved { name })
            }
            Command::ProfileTest { name } => {
                let p = self.store.get(&name)?;
                self.test(&p, cancel).await
            }
            Command::ConnectionTest { profile } => self.test(&profile, cancel).await,
            Command::KeychainStatus => {
                let keychain = self.store.keychain().clone();
                let status = tokio::task::spawn_blocking(move || keychain.check())
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                Ok(Output::Keychain {
                    name: crate::Keychain::name().into(),
                    available: status.is_ok(),
                    error: status.err(),
                })
            }
            Command::Schemas { profile } => {
                let pool = self.pool(&profile).await?;
                Ok(Output::Schemas {
                    schemas: cancellable(cancel, pool.schemas()).await?,
                })
            }
            Command::Tables { profile, schema } => {
                let p = self.store.get(&profile)?;
                let schema = default_schema(&p, schema)?;
                let pool = self.pool(&profile).await?;
                let tables = cancellable(cancel, pool.tables(&schema)).await?;
                Ok(Output::Tables { schema, tables })
            }
            Command::Columns {
                profile,
                schema,
                table,
            } => {
                let p = self.store.get(&profile)?;
                let schema = default_schema(&p, schema)?;
                let pool = self.pool(&profile).await?;
                let columns = cancellable(cancel, pool.columns(&schema, &table)).await?;
                Ok(Output::Columns {
                    schema,
                    table,
                    columns,
                })
            }
            Command::Catalog { profile } => {
                let p = self.store.get(&profile)?;
                let pool = self.pool(&profile).await?;
                let schemas = cancellable(cancel, pool.schemas()).await?;
                let tables = cancellable(cancel, pool.catalog()).await?;
                Ok(Output::Catalog(Catalog {
                    default_schema: default_schema(&p, None).unwrap_or_default(),
                    schemas,
                    tables,
                }))
            }
            Command::Query {
                profile,
                sql,
                session,
                max_rows,
            } => {
                let p = self.store.get(&profile)?;
                let out = self
                    .query(
                        &p,
                        &sql,
                        session,
                        max_rows.unwrap_or(DEFAULT_MAX_ROWS),
                        cancel,
                    )
                    .await?;
                self.remember(&p.name, &sql, &out);
                Ok(Output::Query(out))
            }
            Command::ApplyEdits {
                profile,
                statements,
            } => {
                let p = self.store.get(&profile)?;
                self.apply_edits(&p, &statements, cancel).await?;
                Ok(Output::EditsApplied {
                    statements: statements.len(),
                })
            }
            Command::Explain {
                profile,
                sql,
                analyze,
                session,
            } => {
                let p = self.store.get(&profile)?;
                let plan = self.explain(&p, &sql, analyze, session, cancel).await?;
                Ok(Output::Plan(plan))
            }
            Command::Export {
                profile,
                sql,
                path,
                format,
                session,
            } => {
                let p = self.store.get(&profile)?;
                let format = format
                    .or_else(|| ExportFormat::from_path(&path))
                    .unwrap_or(ExportFormat::Csv);
                let rows = self
                    .export(&p, &sql, &path, format, session, cancel)
                    .await?;
                Ok(Output::Exported {
                    path: path.display().to_string(),
                    rows,
                })
            }
            Command::HistoryList {
                profile,
                search,
                limit,
            } => Ok(Output::History {
                entries: self.history.list(
                    profile.as_deref(),
                    search.as_deref(),
                    limit.unwrap_or(200),
                )?,
            }),
            Command::HistoryClear { profile } => Ok(Output::HistoryCleared {
                removed: self.history.clear(profile.as_deref())?,
            }),
            Command::SshTrustHost { host, port, key } => {
                tunnel::trust_host(&self.known_hosts, &host, port, &key)?;
                Ok(Output::HostTrusted { host })
            }
            Command::SessionClose { session } => {
                let slot = self.sessions.lock().expect("sessions").remove(&session);
                if let Some(slot) = slot
                    && let Some(s) = slot.lock().await.take()
                {
                    s.close().await;
                }
                Ok(Output::SessionClosed)
            }
        }
    }

    /// Add a query that ran to the history (a failure to record it is only logged).
    fn remember(&self, profile: &str, sql: &str, out: &QueryOutput) {
        let last = out.results.last();
        let entry = HistoryEntry {
            at: history::now(),
            profile: profile.to_owned(),
            sql: sql.trim().to_owned(),
            statements: out.results.len() + usize::from(out.error.is_some()),
            elapsed_ms: out.results.iter().map(|r| r.elapsed_ms).sum(),
            rows: last.map(|r| {
                if r.returns_rows {
                    r.rows.len() as u64
                } else {
                    r.rows_affected
                }
            }),
            error: out.error.as_ref().map(|e| e.message.clone()),
        };
        if let Err(e) = self.history.add(&entry) {
            tracing::warn!("couldn't record the query in the history: {e}");
        }
    }

    async fn test(&self, p: &Profile, cancel: &CancellationToken) -> Result<Output> {
        p.validate()?;
        // A tunnel just for the test (the settings may not be saved).
        let tunnel = match &p.ssh {
            Some(ssh) => Some(
                cancellable(
                    cancel,
                    Tunnel::open(
                        ssh,
                        p.server(),
                        tunnel::known_hosts_files(self.known_hosts.clone()),
                    ),
                )
                .await?,
            ),
            None => None,
        };
        let via = tunnel.as_ref().map(|t| t.local.clone());
        let mut s = match cancellable(cancel, Session::connect(p, via)).await {
            Ok(s) => s,
            Err(e) => {
                return Err(match tunnel.as_ref().and_then(|t| t.take_forward_error()) {
                    Some(why) => CoreError::Ssh(why),
                    None => e,
                });
            }
        };
        let server_version = s.server_version().await?;
        s.close().await;
        drop(tunnel);
        Ok(Output::Connected { server_version })
    }

    async fn pool(&self, name: &str) -> Result<MetaPool> {
        let p = self.store.get(name)?;
        let via = self.via(&p).await?;
        let mut pools = self.pools.lock().expect("pools");
        if let Some((saved, saved_via, pool)) = pools.get(name)
            && *saved == p
            && *saved_via == via
        {
            return Ok(pool.clone());
        }
        let pool = MetaPool::new(&p, via.clone())?;
        pools.insert(name.to_owned(), (p, via, pool.clone()));
        Ok(pool)
    }

    /// If connecting failed because the SSH server wouldn't forward the connection,
    /// say so (the database driver only sees the connection drop).
    async fn explain_tunnel_failure<T>(&self, profile: &Profile, r: Result<T>) -> Result<T> {
        let Err(e) = r else { return r };
        if profile.ssh.is_none() {
            return Err(e);
        }
        let tunnels = self.tunnels.lock().await;
        match tunnels
            .get(&profile.name)
            .and_then(|(_, _, t)| t.take_forward_error())
        {
            Some(why) => Err(CoreError::Ssh(why)),
            None => Err(e),
        }
    }

    /// The SSH tunnel's local end for a profile that uses one (opening it, or
    /// opening it again if it dropped); `None` without SSH.
    async fn via(&self, profile: &Profile) -> Result<Endpoint> {
        let Some(ssh) = &profile.ssh else {
            return Ok(None);
        };
        let target = profile.server();
        let mut tunnels = self.tunnels.lock().await;
        if let Some((s, t, tunnel)) = tunnels.get(&profile.name)
            && s == ssh
            && *t == target
            && tunnel.is_alive()
        {
            return Ok(Some(tunnel.local.clone()));
        }
        let tunnel = Tunnel::open(
            ssh,
            target.clone(),
            tunnel::known_hosts_files(self.known_hosts.clone()),
        )
        .await?;
        let local = tunnel.local.clone();
        tunnels.insert(
            profile.name.clone(),
            (ssh.clone(), target, Arc::new(tunnel)),
        );
        Ok(Some(local))
    }

    async fn forget_pool(&self, name: &str) {
        let old = self.pools.lock().expect("pools").remove(name);
        if let Some((_, _, pool)) = old {
            pool.close().await;
        }
        self.tunnels.lock().await.remove(name);
    }

    async fn query(
        &self,
        profile: &Profile,
        sql: &str,
        session: Option<String>,
        max_rows: usize,
        cancel: &CancellationToken,
    ) -> Result<QueryOutput> {
        let dialect = Dialect::from(profile.driver);
        let statements = sql::split_statements(sql, dialect);
        if statements.is_empty() {
            return Err(CoreError::Invalid("there's no SQL to run".into()));
        }
        let slot = self.slot(&session);
        // One query at a time per session.
        let mut guard = slot.lock().await;
        self.connect(&mut guard, profile, cancel).await?;

        let mut out = QueryOutput::default();
        for (index, range) in statements.into_iter().enumerate() {
            let text = &sql[range];
            let live = guard.as_mut().expect("connected above");
            // Boxed so it can be dropped (ending its borrow of the session) on cancel.
            let mut run = Box::pin(live.run(text, max_rows));
            let outcome = tokio::select! {
                r = &mut run => Some(r),
                _ = cancel.cancelled() => None,
            };
            drop(run);
            let result = match outcome {
                Some(r) => r,
                None => {
                    stop_and_settle(&mut guard).await;
                    out.cancelled = true;
                    out.error = Some(StatementError {
                        index,
                        sql: text.to_owned(),
                        message: CoreError::Cancelled.to_string(),
                    });
                    break;
                }
            };
            match result {
                Ok(r) => out.results.push(r),
                Err(e) => {
                    if is_connection_lost(&e) {
                        guard.take();
                    }
                    out.error = Some(StatementError {
                        index,
                        sql: text.to_owned(),
                        message: e.to_string(),
                    });
                    break;
                }
            }
        }
        if session.is_none()
            && let Some(s) = guard.take()
        {
            s.close().await;
        }
        Ok(out)
    }

    /// The session slot for `session`, or a throwaway one.
    fn slot(&self, session: &Option<String>) -> SessionSlot {
        match session {
            Some(id) => self
                .sessions
                .lock()
                .expect("sessions")
                .entry(id.clone())
                .or_default()
                .clone(),
            None => SessionSlot::default(),
        }
    }

    /// Make sure the session is connected, with this version of the profile.
    async fn connect(
        &self,
        guard: &mut Option<Session>,
        profile: &Profile,
        cancel: &CancellationToken,
    ) -> Result<()> {
        if guard.as_ref().is_some_and(|s| s.profile != *profile) {
            // The profile was edited (or the tab switched profile): reconnect.
            if let Some(old) = guard.take() {
                old.close().await;
            }
        }
        if guard.is_none() {
            let via = cancellable(cancel, self.via(profile)).await?;
            let session = cancellable(cancel, Session::connect(profile, via)).await;
            *guard = Some(self.explain_tunnel_failure(profile, session).await?);
        }
        Ok(())
    }

    async fn apply_edits(
        &self,
        profile: &Profile,
        statements: &[String],
        cancel: &CancellationToken,
    ) -> Result<()> {
        if statements.is_empty() {
            return Ok(());
        }
        let via = cancellable(cancel, self.via(profile)).await?;
        let mut s = cancellable(cancel, Session::connect(profile, via)).await?;
        let begin = if profile.driver == Driver::Mysql {
            "START TRANSACTION"
        } else {
            "BEGIN"
        };
        let result = async {
            s.run(begin, 1).await?;
            for (i, st) in statements.iter().enumerate() {
                let r = s.run(st, 1).await.map_err(|e| {
                    CoreError::Database(format!(
                        "statement {} failed, nothing was saved: {e}",
                        i + 1
                    ))
                })?;
                let kind = sql::first_keyword(st, profile.driver.into()).unwrap_or_default();
                // MySQL counts an UPDATE that sets the same values as changing nothing.
                let unchanged_ok = kind == "UPDATE" && profile.driver == Driver::Mysql;
                let n = r.rows_affected;
                if n > 1 || (n == 0 && !unchanged_ok) {
                    return Err(CoreError::Database(format!(
                        "statement {} changed {n} rows instead of 1 (was the row changed or \
                         deleted meanwhile?), so nothing was saved",
                        i + 1
                    )));
                }
            }
            s.run("COMMIT", 1).await?;
            Ok(())
        }
        .await;
        if result.is_err() {
            let _ = s.run("ROLLBACK", 1).await;
        }
        s.close().await;
        result
    }

    async fn explain(
        &self,
        profile: &Profile,
        sql: &str,
        analyze: bool,
        session: Option<String>,
        cancel: &CancellationToken,
    ) -> Result<Plan> {
        let dialect = Dialect::from(profile.driver);
        let statements = sql::split_statements(sql, dialect);
        let statement = match statements.as_slice() {
            [one] => &sql[one.clone()],
            [] => return Err(CoreError::Invalid("there's no SQL to explain".into())),
            _ => {
                return Err(CoreError::Invalid(
                    "explain one statement at a time (put the cursor in it, or select it)".into(),
                ));
            }
        };
        if analyze && !crate::db::looks_like_query(statement, dialect) {
            // It changes data: run it in a transaction that's rolled back, on its own
            // connection.
            let mut s = cancellable(cancel, Session::connect(profile, None)).await?;
            let begin = if profile.driver == Driver::Mysql {
                "START TRANSACTION"
            } else {
                "BEGIN"
            };
            let result = async {
                s.run(begin, 1).await?;
                cancellable(cancel, s.explain(statement, true)).await
            }
            .await;
            let _ = s.run("ROLLBACK", 1).await;
            s.close().await;
            return result;
        }
        let slot = self.slot(&session);
        let mut guard = slot.lock().await;
        self.connect(&mut guard, profile, cancel).await?;
        let live = guard.as_mut().expect("connected above");
        let mut run = Box::pin(live.explain(statement, analyze));
        let outcome = tokio::select! {
            r = &mut run => Some(r),
            _ = cancel.cancelled() => None,
        };
        drop(run);
        let result = match outcome {
            Some(r) => r,
            None => {
                stop_and_settle(&mut guard).await;
                Err(CoreError::Cancelled)
            }
        };
        if session.is_none()
            && let Some(s) = guard.take()
        {
            s.close().await;
        }
        result
    }

    /// Run a script, streaming the last statement's rows into a file. The other
    /// statements run first, as usual; the last one must return rows.
    async fn export(
        &self,
        profile: &Profile,
        sql: &str,
        path: &Path,
        format: ExportFormat,
        session: Option<String>,
        cancel: &CancellationToken,
    ) -> Result<u64> {
        let dialect = Dialect::from(profile.driver);
        let statements = sql::split_statements(sql, dialect);
        let Some((last, before)) = statements.split_last() else {
            return Err(CoreError::Invalid("there's no SQL to run".into()));
        };
        let slot = self.slot(&session);
        let mut guard = slot.lock().await;
        self.connect(&mut guard, profile, cancel).await?;
        for range in before {
            let live = guard.as_mut().expect("connected above");
            cancellable(cancel, live.run(&sql[range.clone()], 1)).await?;
        }

        let tmp = path.with_file_name(format!(
            ".{}.datadrop-tmp",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("export")
        ));
        let ctx = |e| CoreError::io(format!("writing {}", path.display()), e);
        let file = std::fs::File::create(&tmp).map_err(ctx)?;
        let mut writer = ExportWriter::new(std::io::BufWriter::new(file), format);
        let live = guard.as_mut().expect("connected above");
        let mut run = Box::pin(live.export(&sql[last.clone()], &mut writer));
        let outcome = tokio::select! {
            r = &mut run => Some(r),
            _ = cancel.cancelled() => None,
        };
        drop(run);
        let result = match outcome {
            Some(Ok(true)) => writer.finish().map_err(ctx),
            Some(Ok(false)) => Err(CoreError::Invalid(
                "the last statement doesn't return rows, so there's nothing to export".into(),
            )),
            Some(Err(e)) => Err(e),
            None => {
                stop_and_settle(&mut guard).await;
                Err(CoreError::Cancelled)
            }
        };
        if session.is_none()
            && let Some(s) = guard.take()
        {
            s.close().await;
        }
        match result {
            Ok(rows) => {
                std::fs::rename(&tmp, path).map_err(ctx)?;
                Ok(rows)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }

    /// Close every session and pool.
    pub async fn shutdown(&self) {
        let sessions: Vec<_> = self
            .sessions
            .lock()
            .expect("sessions")
            .drain()
            .map(|(_, s)| s)
            .collect();
        for slot in sessions {
            if let Ok(mut g) = slot.try_lock()
                && let Some(s) = g.take()
            {
                s.close().await;
            }
        }
        let pools: Vec<_> = self
            .pools
            .lock()
            .expect("pools")
            .drain()
            .map(|(_, (_, _, p))| p)
            .collect();
        for p in pools {
            p.close().await;
        }
    }
}

/// After cancelling a statement: ask the database to stop it, then let it finish
/// failing so the session is usable again (or drop the session if that fails).
async fn stop_and_settle(guard: &mut Option<Session>) {
    let Some(live) = guard.as_mut() else { return };
    let cancelled = live.cancel_running().await;
    let settle = tokio::time::timeout(Duration::from_secs(5), live.run("SELECT 1", 1)).await;
    if cancelled.is_err() || !matches!(settle, Ok(Ok(_))) {
        guard.take();
    } else {
        live.clear_cancel();
    }
}

/// What `schema` means when it's left out.
fn default_schema(p: &Profile, schema: Option<String>) -> Result<String> {
    if let Some(s) = schema.filter(|s| !s.is_empty()) {
        return Ok(s);
    }
    match p.driver {
        Driver::Postgres => Ok("public".into()),
        Driver::Sqlite => Ok("main".into()),
        Driver::Mysql => {
            let db = match p.url.as_deref().and_then(|u| url::Url::parse(u).ok()) {
                Some(u) => u.path().trim_start_matches('/').to_owned(),
                None => p.database.clone(),
            };
            if db.is_empty() {
                Err(CoreError::Invalid(
                    "say which database (schema): the profile doesn't name one".into(),
                ))
            } else {
                Ok(db)
            }
        }
    }
}

fn is_connection_lost(e: &CoreError) -> bool {
    matches!(e, CoreError::Database(m) if m.starts_with("couldn't reach the server"))
}

async fn cancellable<T>(
    cancel: &CancellationToken,
    fut: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        r = fut => r,
        _ = cancel.cancelled() => Err(CoreError::Cancelled),
    }
}
