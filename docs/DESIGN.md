# DataDrop design

## Commands

Everything DataDrop does is a `Command` in `datadrop-core` (see
`crates/core/src/command.rs`), run by `Client::execute_cancellable`, which returns an
`Output`. Both front-ends only build commands and render outputs:

- The **CLI** maps its subcommands onto commands, and `datadrop exec '<json>'` takes
  one directly.
- The **desktop app** runs them on a background tokio runtime (`crates/gui/src/backend.rs`)
  and polls the result each frame, so the UI never waits on the network.

Adding a feature means adding a command first, so it's scriptable from the start.

## Profiles

One JSON file per profile in `~/.datadrop/profiles` (or `$DATADROP_HOME/profiles`). The
file name is the profile name. Names are limited to letters, digits, spaces and
`- _ .` so they're always safe file names.

A profile has either fields (host, port, database) or a connection URL. With a URL, its
host, port and database win; `user` and `password` fill in what it leaves out. Writes
go to a temporary file and are renamed into place, with mode 0600.

Passwords are plain text for now. Moving them to the OS keychain is on the list below.

## Running SQL

- Scripts are split into statements by the lexer (`crates/core/src/sql.rs`), which
  understands strings, quoted identifiers, comments, PostgreSQL dollar quoting and
  MySQL's backticks and `#` comments. Each statement runs on its own, so results,
  timings and errors are reported per statement; a script stops at the first error.
- Statements run over each driver's **text protocol** (`sqlx::raw_sql`). Every value
  arrives as the server's own text for it, so any type (enums, arrays, JSON, geometry,
  intervals...) displays correctly without per-type code. Non-UTF-8 bytes show as
  `0x…` hex.
- A result set with no rows still has its columns: the statement is prepared to read
  them.
- Each editor tab has a **session**: its own connection, kept open, so session state
  carries over between runs. Without a session (the CLI) the connection is closed
  after the command.
- **Cancelling** sends `pg_cancel_backend(pid)` or `KILL QUERY id` from a second
  connection, then waits for the statement to fail, so the session stays usable.
- Rows beyond the row limit (default 10,000 on the CLI, 1,000 in the app) are read
  and dropped, not kept, and the result is marked as truncated.

The schema browser uses a small per-profile connection pool, separate from the tab
sessions, so it never waits behind a long query.

SQLite has no server to cancel from: each session installs a progress handler that
SQLite calls every 10,000 steps, which stops the statement once Stop sets a flag.

## SSH tunnels

`crates/core/src/tunnel.rs`, on russh (with ring). A tunnel connects to the SSH
server, signs in (agent, key file or password), and forwards a random local port on
127.0.0.1 to the database as the SSH server sees it (`direct-tcpip`, like `ssh -L`).
Sessions and the browsing pool connect to that port. Tunnels are kept per profile and
reopened when they drop or the profile's SSH settings change.

Host keys are checked against `~/.ssh/known_hosts` and DataDrop's own
`known_hosts`. An unknown key is an error carrying its fingerprint and key, so the
front-end can ask and then `SshTrustHost`; a changed key is refused outright.

## Autocompletion

`crates/core/src/complete.rs`. The `Catalog` command reads every table and column in
one query (cached per profile in the app, reloaded after DDL). Completion looks at the
text around the cursor: after `FROM`/`JOIN`/`UPDATE`/`INTO` it offers tables; after
`name.` the columns of that table or alias (or the tables of that schema); otherwise
the columns of the tables the statement names, then keywords. Names that need quotes
get them.

## Editing results

`crates/core/src/edit.rs`. A result is editable when it comes from a plain `SELECT`
of one table (no joins, grouping, `DISTINCT`, set operations or subqueries) and the
table's primary key columns are in it. Edits become `UPDATE`/`DELETE` statements keyed
on the primary key and `INSERT`s, with values as quoted literals the database converts
to the column type. `ApplyEdits` runs them in one transaction on its own connection,
and rolls back unless each `UPDATE` and `DELETE` changes exactly one row (MySQL reports
an `UPDATE` to the same values as changing none, which is allowed).

## Query plans

`crates/core/src/plan.rs` turns PostgreSQL's `EXPLAIN (FORMAT JSON)`, MySQL's
`EXPLAIN FORMAT=TREE` / `EXPLAIN ANALYZE` and SQLite's `EXPLAIN QUERY PLAN` into one
tree. With analyze, a statement that changes data runs inside a transaction that's
rolled back, on its own connection (so the tab's own transaction isn't touched).

## History and export

Every `Query` is appended to `history.jsonl` (one JSON object per line; the file is
trimmed to the newest 5,000 when it reaches 10,000). `Export` streams the last
statement's rows straight into a file through `ExportWriter`, so exports aren't held
in memory or limited by the row limit.

## Desktop app

egui 0.36 with eframe (glow). The editor is an egui `TextEdit` with a custom layouter
that colours lexer tokens and find matches; layouts are cached per text so typing
stays cheap. Results use `egui_extras::TableBuilder`, which only lays out visible rows.

egui-phosphor (icons) and egui-notify (toasts) each target one egui version, so the
egui crates move together, when those do (Dependabot is set up to hold them back).

## SQL Server

Not supported yet. The candidates for a driver, as of late 2026:

- **`mssql-tds`** (crate; repo `microsoft/mssql-rs`, MIT): Microsoft's own TDS
  implementation in Rust, the core of their new Python, Node and ODBC drivers. It
  streams rows, exposes column metadata, can cancel a running statement (TDS
  attention), and supports Azure AD and integrated (Kerberos/SSPI) sign-in. It's
  pre-1.0 (0.3) and moving fast, uses `native-tls` (OpenSSL on Linux, so release
  builds would depend on the system's libssl), and enables integrated auth by
  default (turn off default features). The best long-term bet.
- **`tiberius`**: the established TDS crate (0.12/0.13, widely used, rustls or
  native-tls). Maintenance has slowed and it can't cancel a running query.
- **`mssql`**: a new (2026) fork of tiberius focused on maintenance; too young to judge.

Fitting it in means a fourth driver beside sqlx: values come typed rather than as
text, so they need formatting for display (dates, decimals, GUIDs, binary), and the
schema queries and plan parsing (`SET SHOWPLAN_XML`) need T-SQL versions. A `Driver`
behind a Cargo feature would keep the OpenSSL dependency optional.

## Next

- Passwords in the OS keychain (macOS Keychain, Windows Credential Manager, Secret
  Service), with plain-text files as a fallback.
- macOS `.app` bundle with notarisation; Windows installer; Linux packages.
- SQL Server (see above).
