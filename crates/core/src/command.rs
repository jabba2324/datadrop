//! Everything DataDrop can do, as data. The CLI and the desktop app both build these
//! and run them with [`crate::Client::execute`]; `datadrop exec '<json>'` takes one
//! as JSON directly.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::complete::Catalog;
use crate::db::{ColumnInfo, StatementResult, TableInfo};
use crate::export::ExportFormat;
use crate::history::HistoryEntry;
use crate::plan::Plan;
use crate::profile::{Profile, ProfileSummary};
use crate::schema::{ObjectKind, SchemaModel};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// All saved profiles (no passwords).
    ProfileList,
    /// One profile; the password is masked unless `show_password`.
    ProfileGet {
        name: String,
        #[serde(default)]
        show_password: bool,
    },
    /// Save a profile. `previous_name` makes it an edit of that profile (and renames it
    /// if the name changed); without it an existing profile is only replaced with
    /// `overwrite`.
    ProfileSave {
        profile: Profile,
        #[serde(default)]
        previous_name: Option<String>,
        #[serde(default)]
        overwrite: bool,
    },
    ProfileRemove {
        name: String,
    },
    /// Connect with a saved profile and report the server version.
    ProfileTest {
        name: String,
    },
    /// Connect with unsaved settings (the profile form's Test button).
    ConnectionTest {
        profile: Profile,
    },
    /// Schemas (PostgreSQL), databases (MySQL) or attached databases (SQLite), without
    /// the system ones.
    Schemas {
        profile: String,
    },
    /// Tables and views in a schema. Without `schema`: `public` on PostgreSQL, the
    /// profile's database on MySQL, `main` on SQLite.
    Tables {
        profile: String,
        #[serde(default)]
        schema: Option<String>,
    },
    Columns {
        profile: String,
        #[serde(default)]
        schema: Option<String>,
        table: String,
    },
    /// Every table and column, for autocompletion.
    Catalog {
        profile: String,
    },
    /// A schema's structure: tables with their columns, keys, indexes and triggers;
    /// views, routines and sequences.
    Schema {
        profile: String,
        #[serde(default)]
        schema: Option<String>,
    },
    /// The SQL that creates an object (or everything in the schema).
    Ddl {
        profile: String,
        #[serde(default)]
        schema: Option<String>,
        kind: ObjectKind,
        #[serde(default)]
        name: String,
    },
    /// Run SQL: one statement or a script, statement by statement, stopping at the
    /// first error. With a `session` the connection stays open for the next query
    /// with the same session (each editor tab has one); without, it's closed after.
    Query {
        profile: String,
        sql: String,
        #[serde(default)]
        session: Option<String>,
        /// Keep at most this many rows per result (default 10,000).
        #[serde(default)]
        max_rows: Option<usize>,
        /// Set the session's auto-commit first (see `SessionAutocommit`).
        #[serde(default)]
        autocommit: Option<bool>,
    },
    /// Run SQL and write the last statement's rows to a file, however many there are
    /// (no row limit). The format comes from the file's extension unless given.
    Export {
        profile: String,
        sql: String,
        path: PathBuf,
        #[serde(default)]
        format: Option<ExportFormat>,
        #[serde(default)]
        session: Option<String>,
    },
    /// The query plan for one statement. With `analyze` it really runs, to measure
    /// each step; statements that change data are then rolled back (on a separate
    /// connection, so the session's own transaction isn't touched).
    Explain {
        profile: String,
        sql: String,
        #[serde(default)]
        analyze: bool,
        #[serde(default)]
        session: Option<String>,
    },
    /// Save edits made to a result: the statements from `edit::statements`, run in
    /// one transaction on their own connection. Each UPDATE and DELETE must change
    /// exactly one row, or nothing is saved.
    ApplyEdits {
        profile: String,
        statements: Vec<String>,
    },
    /// Trust an SSH server's host key (after checking its fingerprint), so tunnels
    /// through it can connect. The key is in OpenSSH format, as the unknown-host
    /// error gives it.
    SshTrustHost {
        host: String,
        port: u16,
        key: String,
    },
    /// Close a session's connection.
    SessionClose {
        session: String,
    },
    /// Queries run before, newest first.
    HistoryList {
        #[serde(default)]
        profile: Option<String>,
        /// Only queries containing this text (ignoring case).
        #[serde(default)]
        search: Option<String>,
        /// At most this many (default 200).
        #[serde(default)]
        limit: Option<usize>,
    },
    /// Forget the history: all of it, or one profile's.
    HistoryClear {
        #[serde(default)]
        profile: Option<String>,
    },
    /// Whether the OS keychain can be used for passwords here.
    KeychainStatus,
    /// Turn auto-commit off (or back on) for a session: with it off, a transaction is
    /// begun before the session's next query and stays open until committed or
    /// rolled back.
    SessionAutocommit {
        session: String,
        autocommit: bool,
    },
    /// Commit or roll back a session's open transaction.
    Transaction {
        profile: String,
        session: String,
        action: TransactionAction,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionAction {
    Commit,
    Rollback,
}

pub const DEFAULT_MAX_ROWS: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Output {
    Profiles {
        profiles: Vec<ProfileSummary>,
    },
    Profile {
        profile: Profile,
    },
    ProfileSaved {
        name: String,
    },
    ProfileRemoved {
        name: String,
    },
    Connected {
        server_version: String,
    },
    Schemas {
        schemas: Vec<String>,
    },
    Tables {
        schema: String,
        tables: Vec<TableInfo>,
    },
    Columns {
        schema: String,
        table: String,
        columns: Vec<ColumnInfo>,
    },
    Query(QueryOutput),
    Catalog(Catalog),
    Schema(SchemaModel),
    Ddl {
        sql: String,
    },
    SessionClosed,
    HostTrusted {
        host: String,
    },
    EditsApplied {
        statements: usize,
    },
    Plan(Plan),
    Exported {
        path: String,
        rows: u64,
    },
    History {
        entries: Vec<HistoryEntry>,
    },
    HistoryCleared {
        removed: usize,
    },
    /// A session's transaction state, after a commit, rollback or change of mode.
    Transaction {
        in_transaction: bool,
    },
    Keychain {
        /// What it's called here ("the macOS Keychain", ...).
        name: String,
        available: bool,
        /// Why it isn't.
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryOutput {
    /// The statements that ran, in order.
    pub results: Vec<StatementResult>,
    /// The statement that failed (the ones after it didn't run).
    pub error: Option<StatementError>,
    pub cancelled: bool,
    /// The session has a transaction open (begun by a statement, or because
    /// auto-commit is off), to commit or roll back.
    #[serde(default)]
    pub in_transaction: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatementError {
    /// Position in the script, from 0.
    pub index: usize,
    pub sql: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_shape() {
        let cmd: Command =
            serde_json::from_str(r#"{"command": "query", "profile": "shop", "sql": "select 1"}"#)
                .unwrap();
        assert_eq!(
            cmd,
            Command::Query {
                profile: "shop".into(),
                sql: "select 1".into(),
                session: None,
                max_rows: None,
                autocommit: None,
            }
        );
        let out = serde_json::to_value(Output::ProfileRemoved { name: "x".into() }).unwrap();
        assert_eq!(out["type"], "profile_removed");
    }
}
