//! DataDrop core, shared by every front-end (the `datadrop` CLI and the desktop app).
//!
//! Front-ends build [`Command`]s, run them with [`Client::execute_cancellable`] and
//! render the [`Output`]. Everything else lives here: connection profiles, talking to
//! PostgreSQL and MySQL, and the SQL lexer used for highlighting and for splitting
//! scripts into statements.

pub mod clickhouse;
pub mod client;
pub mod command;
pub mod complete;
pub mod db;
pub mod diagram;
pub mod diff;
#[cfg(feature = "duckdb")]
pub mod duck;
pub mod edit;
pub mod error;
pub mod export;
pub mod format;
pub mod history;
pub mod import;
pub mod keychain;
#[cfg(feature = "sqlserver")]
pub mod mssql;
pub mod params;
pub mod plan;
pub mod profile;
pub mod schema;
pub mod sql;
pub mod tunnel;

pub use client::{Client, ConfigDir};
pub use command::{Command, DEFAULT_MAX_ROWS, Output, QueryOutput, StatementError};
pub use complete::{Catalog, Completion, ItemKind};
pub use db::{CatalogTable, Column, ColumnInfo, StatementResult, TableInfo, TableKind};
pub use error::{CoreError, Result};
pub use export::ExportFormat;
pub use history::HistoryEntry;
pub use keychain::Keychain;
pub use plan::{Plan, PlanNode};
pub use profile::{
    Driver, PasswordStorage, Profile, ProfileStore, ProfileSummary, SshAuth, SshTunnel, SslMode,
};
pub use schema::{ObjectKind, SchemaModel, TableModel};
pub use tokio_util::sync::CancellationToken;
