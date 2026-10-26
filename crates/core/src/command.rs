//! Everything DataDrop can do, as data. The CLI and the desktop app both build these
//! and run them with [`crate::Client::execute`]; `datadrop exec '<json>'` takes one
//! as JSON directly.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::db::{ColumnInfo, StatementResult, TableInfo};
use crate::export::ExportFormat;
use crate::history::HistoryEntry;
use crate::profile::{Profile, ProfileSummary};

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
}

pub const DEFAULT_MAX_ROWS: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    SessionClosed,
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
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryOutput {
    /// The statements that ran, in order.
    pub results: Vec<StatementResult>,
    /// The statement that failed (the ones after it didn't run).
    pub error: Option<StatementError>,
    pub cancelled: bool,
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
                max_rows: None
            }
        );
        let out = serde_json::to_value(Output::ProfileRemoved { name: "x".into() }).unwrap();
        assert_eq!(out["type"], "profile_removed");
    }
}
