//! DataDrop core, shared by every front-end (the `datadrop` CLI and the desktop app).
//!
//! Front-ends build [`Command`]s, run them with [`Client::execute_cancellable`] and
//! render the [`Output`]. Everything else lives here: connection profiles, talking to
//! PostgreSQL and MySQL, and the SQL lexer used for highlighting and for splitting
//! scripts into statements.

pub mod client;
pub mod command;
pub mod complete;
pub mod db;
pub mod error;
pub mod export;
pub mod history;
pub mod plan;
pub mod profile;
pub mod sql;

pub use client::{Client, ConfigDir};
pub use command::{Command, DEFAULT_MAX_ROWS, Output, QueryOutput, StatementError};
pub use complete::{Catalog, Completion, ItemKind};
pub use db::{CatalogTable, Column, ColumnInfo, StatementResult, TableInfo, TableKind};
pub use error::{CoreError, Result};
pub use export::ExportFormat;
pub use history::HistoryEntry;
pub use plan::{Plan, PlanNode};
pub use profile::{Driver, Profile, ProfileStore, ProfileSummary, SslMode};
pub use tokio_util::sync::CancellationToken;
