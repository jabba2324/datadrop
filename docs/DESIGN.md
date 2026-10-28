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

A profile's `passwords` setting says where its secrets are: `keychain` (the default
for new profiles when there's a keychain) or `file`. With the keychain
(`crates/core/src/keychain.rs`, on keyring-core and the native stores: the macOS
Keychain, Windows Credential Manager, the Secret Service over zbus), the database
password is under the account `<profile>` and the SSH password or key passphrase under
`<profile> (SSH)`, service `DataDrop`, and the file has none (a password in the URL
moves to the keychain too). `ProfileStore::get` fills them back in; reads are cached
per process, so the keychain is asked once. Renaming a profile moves its entries, and
removing it (or moving it back to the file) deletes them.

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

A session knows whether it has a transaction open from the statements it runs
(`sql::transaction_after`: BEGIN / START TRANSACTION open one; COMMIT, END,
ROLLBACK, and on MySQL the statements that commit implicitly, close it). With
auto-commit off (`autocommit: false` on a `Query`, or `SessionAutocommit`) the
session begins a transaction before running anything when none is open; `Transaction`
commits or rolls it back. Each query's output says whether one is open.

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

## Schema structure

`crates/core/src/schema.rs`. The `Schema` command reads a schema into a
`SchemaModel`: tables and views with columns (type, default, identity or generated
clause, comment), constraints (primary key, unique, check, foreign keys with their
target and rules), indexes, triggers and view definitions; routines and sequences.
It's a few catalog queries per schema (PostgreSQL's `pg_catalog`, MySQL's
`information_schema` plus `SHOW CREATE` for routines, SQLite's `sqlite_master` and
pragmas). Constraints, indexes and triggers keep their SQL as the server renders it
(`pg_get_constraintdef`, `pg_get_indexdef`...), so DDL needs no per-type knowledge.

`Ddl` turns the model back into SQL (MySQL tables use `SHOW CREATE TABLE`, for the
engine and options). For a whole schema the order is the one pg_dump uses:
sequences, routines (with `check_function_bodies` off), tables with referenced ones
first (a foreign key in a cycle is added afterwards), views, triggers. The live tests
recreate a schema from its DDL and check it reads back identical.

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

## Changing tables, and schema diffs

`crates/core/src/diff.rs` compares two `TableModel`s and writes the statements between
them: renames (table and columns, which the editor tracks), dropped and added
columns, changed columns (PostgreSQL `ALTER COLUMN` for each change; MySQL `MODIFY
COLUMN` with the whole column), and constraints, indexes and triggers dropped before
and added after. SQLite can only rename, add and drop columns in place, so for anything
else it rebuilds the table: create the new one, copy the rows, drop the old, rename,
recreate indexes and triggers. The table editor builds the new model from its form;
`AlterTable` runs the result.

`SchemaDiff` loads two schemas and does the same for every table (created in
dependency order when missing, dropped when gone), plus views, routines and sequences.
Definitions naming the source schema are rewritten to the target's. Migrations run in
one transaction on PostgreSQL and SQLite; MySQL commits each DDL statement itself.

## Import

`crates/core/src/import.rs` reads CSV/TSV (the `csv` crate: quoted fields, line
breaks inside them, a BOM) or a JSON array of objects (as exported). `Import` turns the
rows into multi-row INSERTs (200 rows each) with every value a quoted literal, which
the database converts to the column's type as it does for typed SQL, and runs them in
one transaction on a connection of its own: any failure (named by its rows) or a
cancel rolls back everything.

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

Through Microsoft's `mssql-tds` (behind the `sqlserver` feature, on by default; on
Linux it uses OpenSSL, built from source with `openssl/vendored` so binaries don't need
the system's). `crates/core/src/mssql.rs` connects (`sqlserver://user@host:1433/db`
URLs, or the fields; SSL modes map onto its encryption settings, and a CA certificate
is the server's own certificate, matched exactly), runs batches and writes its typed
values as SQL Server's tools show them (dates, `datetimeoffset` at its offset, money,
`uniqueidentifier`, binary as `0x…`).

Scripts run batch by batch, split at `GO` lines (`sql::batches`), so variables live
through a batch and every result set it returns is shown; the statement under the
cursor is still found by `;`. Stop sends the attention signal on the session's own
connection. Plans come from `SHOWPLAN_ALL` (estimated) or `STATISTICS PROFILE` (after
running), which are rows with `NodeId` and `Parent`. The schema model is read from the
`sys` catalog views; DDL puts views, routines and triggers in batches of their own;
table changes use `sp_rename`, `ALTER COLUMN` (type and nullability together) and named
default constraints. The browsing "pool" is one shared connection.

## ClickHouse

Over its HTTP interface (`crates/core/src/clickhouse.rs`, with `ureq`), port 8123 (8443
with TLS). Every query asks for `TabSeparatedWithNamesAndTypes`, so values arrive as
ClickHouse's own text, like the text protocols the other drivers use. A tab's session
is a ClickHouse `session_id`; each query has its own `query_id`, which Stop kills
(`KILL QUERY`). Rows past the row limit aren't read: the connection is dropped.
Metadata comes from `system.tables` / `system.columns` with `{p1:String}` query
parameters, and a table's DDL is its `create_table_query`. ClickHouse has no
transactions to speak of, so imports and migrations run statement by statement there,
and results aren't edited in the grid (rows change with `ALTER TABLE … UPDATE`).

## DuckDB

Embedded (`crates/core/src/duck.rs`, the `duckdb` feature, off by default because it
compiles DuckDB's C++: the release builds turn it on). DuckDB's API is synchronous, so
its work runs on blocking threads. A database file is opened once per process and
sessions use clones of that connection (two opens of one file would conflict). Query
results are cast to text by DuckDB: `SELECT COLUMNS(*)::VARCHAR FROM (query)`. Stop
interrupts the connection. The dialect is PostgreSQL's (close enough for highlighting,
quoting and `ALTER`); plans come from `EXPLAIN (FORMAT JSON)` and, analyzed, DuckDB's
profile; the schema model from `duckdb_tables()`, `duckdb_constraints()` and friends.

## Packaging

`packaging/`: the icon (SVG, and PNG/ICO made from it), `macos/package.sh` (the `.app`
and `.dmg`, signed with a Developer ID and notarised when the `APPLE_*` secrets are
set, ad hoc otherwise), `windows/datadrop.iss` (Inno Setup: per-user or all-users,
Start menu, optional PATH entry, "Open with" for `.sql`) and `linux/package.sh` (`.deb`
and `.rpm` from the same files, with a desktop entry). The release workflow runs them.
