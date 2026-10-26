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

## Desktop app

egui 0.36 with eframe (glow). The editor is an egui `TextEdit` with a custom layouter
that colours lexer tokens and find matches; layouts are cached per text so typing
stays cheap. Results use `egui_extras::TableBuilder`, which only lays out visible rows.

egui-phosphor (icons) and egui-notify (toasts) each target one egui version, so the
egui crates move together, when those do (Dependabot is set up to hold them back).

## Next

- Passwords in the OS keychain (macOS Keychain, Windows Credential Manager, Secret
  Service), with plain-text files as a fallback.
- SSH tunnels.
- Editing table data in the grid (with generated `UPDATE`s shown before running).
- Export results to CSV / JSON files from the app.
- Autocompletion of table and column names.
- Query history.
- `EXPLAIN` view.
- More databases: SQLite next (sqlx supports it).
- macOS `.app` bundle with notarisation; Windows installer; Linux packages.
