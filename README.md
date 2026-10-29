# DataDrop

**A fast, open-source SQL client for the desktop and the terminal.**

DataDrop is one native app for PostgreSQL, MySQL, SQLite, SQL Server, ClickHouse and
DuckDB. You get a schema browser, an editor that knows your tables, a results grid you
can edit, query plans, ER diagrams and schema diffs. A command line does everything the
app does, for scripts and servers.

It's written in Rust and runs on Windows, macOS and Linux. There's no account and no
telemetry. Passwords go in your OS keychain,
and SSH tunnels are built in.

![DataDrop: the explorer, a query and its results](docs/screenshots/workspace-dark.png)

**[Download the latest release](https://github.com/jabba2324/datadrop/releases/latest)**:
a `.dmg` for macOS, a setup `.exe` for Windows, `.deb` and `.rpm` packages for Linux.
See [Install](#install).

## What it does

- **Connect** with a host and port or a URL, over SSL with an optional CA certificate.
  Go through an SSH server if you need to, signing in with your agent, a key file or a
  password. Test a connection before saving it.
- **Browse** schemas, tables and their columns, keys, foreign keys, indexes and
  triggers, plus views, functions, procedures and sequences. Double-click a table to
  see its rows; right-click for its DDL, or the whole schema's.
- **Write SQL** with syntax highlighting, tabs and autocompletion of tables, columns
  (including after `alias.`), schemas and keywords. **Format** it (`Ctrl+Shift+F`).
  **Find and replace** (`Ctrl+F`).
- **Run** the statement at the cursor or the selection (`Ctrl+Enter`, `⌘↩` on a Mac),
  or the whole tab (`Ctrl+Shift+Enter`). Each statement gets its own result. **Stop**
  cancels the query on the server, not just in the app.
- **Parameters**: write `:name` (or `?`, or `$1` on PostgreSQL) and you're asked for
  the values. Each tab remembers the last ones.
- **Work with results**: sort, filter with a `WHERE` condition (both run on the server
  when not every row was fetched), copy, and **export** every row to CSV, TSV or JSON.
- **Edit data in the grid**: change cells, set NULL, add and delete rows. The
  statements are shown before they run, and they run in one transaction.
- **Transactions**: switch a tab to **Manual** and what you run stays in a transaction
  until you **Commit** or **Roll back**.
- **Change tables** from a form: columns, primary key, indexes, foreign keys and checks.
  The `CREATE`/`ALTER` statements are shown before they run.
- **Explain** and **Explain analyze**: the query plan as a tree, with estimated and
  actual rows. Slow steps and far-off estimates are highlighted.
- **Diagrams** of a schema's tables and their foreign keys. Save them as SVG or copy
  them as Mermaid.
- **Compare schemas**, on two servers or one, and get the SQL that makes one like the
  other.
- **Import** a CSV, TSV or JSON file into a table, in one transaction.
- **History** of every query, from the app and the command line, searchable.

| | |
|---|---|
| ![Editing rows in the grid](docs/screenshots/editing.png) | ![The table editor](docs/screenshots/table-editor.png) |
| Edit rows in the grid, then review the SQL and save | Create and change tables from a form |
| ![A new connection](docs/screenshots/connection.png) | ![An ER diagram](docs/screenshots/diagram.png) |
| Six databases, SSH tunnels, passwords in the keychain | ER diagrams, as SVG or Mermaid |

It comes in light and dark, and follows your system's theme:

![The light theme](docs/screenshots/workspace-light.png)

## How it's built

DataDrop is three Rust crates. All the work happens in one library, and the desktop
app and the command line are thin layers over it:

```
 datadrop-gui (desktop app)      datadrop (command line)
              \                       /
               Command  →  Output
                         |
                   datadrop-core
     drivers · SSH tunnels · schema, DDL and diffs · plans
     import/export · history · profiles and the keychain
```

- **`datadrop-core`** does everything: it connects to the databases, runs SQL statement
  by statement, reads schemas and writes DDL, diffs schemas, parses query plans,
  imports and exports, and keeps the history and the connection profiles. Every action
  is a `Command` given to `Client::execute`, which returns an `Output`.
- **`datadrop`** (`crates/cli`) is the command line. It turns arguments into `Command`s
  and prints the `Output` as a table, CSV or JSON. `datadrop exec` takes a `Command` as
  JSON. See [docs/CLI.md](docs/CLI.md).
- **`datadrop-gui`** (`crates/gui`) is the desktop app, built with
  [egui](https://github.com/emilk/egui). It sends the same `Command`s and draws the
  results.

So anything the app can do, the command line can do too. [docs/DESIGN.md](docs/DESIGN.md)
explains how the pieces fit.

## Supported databases

| Database | Tested with | Connection handled by |
|---|---|---|
| PostgreSQL | 17 | [`sqlx`](https://crates.io/crates/sqlx) 0.9 (`postgres`), TLS with rustls |
| MySQL | 8.4 | [`sqlx`](https://crates.io/crates/sqlx) 0.9 (`mysql`) |
| SQLite | the SQLite bundled with `sqlx` | [`sqlx`](https://crates.io/crates/sqlx) 0.9 (`sqlite`) |
| SQL Server | 2022 | [`mssql-tds`](https://crates.io/crates/mssql-tds) 0.3, the native TDS protocol |
| ClickHouse | latest | DataDrop's own client, over the HTTP interface with [`ureq`](https://crates.io/crates/ureq) 3 |
| DuckDB | 1.5 | [`duckdb`](https://crates.io/crates/duckdb) (duckdb-rs), with the engine compiled in |
| CockroachDB | latest | the PostgreSQL driver |

PostgreSQL, MySQL, SQL Server and ClickHouse are tested against real servers on every
change, in CI; SQLite and DuckDB need no server.

All of the above work everywhere, except that ClickHouse results are read-only in the
grid (it has no keys to match rows on) and ClickHouse has no transactions. SSH tunnels
use [`russh`](https://crates.io/crates/russh). Passwords go through
[`keyring-core`](https://crates.io/crates/keyring-core): the macOS Keychain, Windows
Credential Manager or the Secret Service on Linux.

Other databases that speak the PostgreSQL or MySQL protocol (MariaDB, TiDB, Neon,
Supabase…) may well work, but they haven't been tested.

## Install

Download from the [releases page](https://github.com/jabba2324/datadrop/releases):

- **macOS** (Apple Silicon): `DataDrop-….dmg`; drag DataDrop to Applications. Until
  releases are notarised, open it the first time with right-click → Open.
- **Windows**: `DataDrop-…-setup.exe` installs the app (with a Start menu entry, and
  optionally the `datadrop` command on your PATH), for you or for everyone.
- **Linux**: `datadrop_…_amd64.deb` / `_arm64.deb` (Debian, Ubuntu) or
  `datadrop-….x86_64.rpm` / `.aarch64.rpm` (Fedora, RHEL, openSUSE), with a desktop
  entry.

There are plain archives for every platform too: `datadrop-gui-…` is the desktop app
and `datadrop-…` the command line. Each release has a `SHA256SUMS` file signed with
[minisign](https://jedisct1.github.io/minisign/):

```sh
minisign -Vm SHA256SUMS -p minisign.pub
sha256sum -c --ignore-missing SHA256SUMS
```

## The command line

```sh
datadrop profile add shop --url postgres://alice@db.example.com/shop --ask-password
datadrop tables shop --schema public
datadrop query shop "select id, total from orders order by id desc limit 5"
datadrop query shop -f report.sql --format csv
datadrop query shop "select * from orders" -o orders.csv   # every row
datadrop diff dev staging --source-schema public --target-schema public
```

Every command takes `--json`. The full reference is in [docs/CLI.md](docs/CLI.md).

## Where things are kept

Everything lives in `~/.datadrop` (set `DATADROP_HOME` to use another folder):

- `profiles/<name>.json`: one file per connection.
- `history.jsonl`: the query history (the newest 5,000 queries).
- `known_hosts`: SSH servers trusted in DataDrop (your `~/.ssh/known_hosts` is also
  read).
- `gui.json`: the desktop app's settings and open files.

Passwords (database and SSH) and key passphrases are kept in the OS keychain, under
"DataDrop". `datadrop keychain` says whether there's one to use. Without one (a server
with no desktop session, say), or if you choose `--passwords file` / "In the profile
file", **they're stored in the profile file as plain text**. Move an existing profile
with `datadrop profile edit <name> --passwords keychain`.

The files and folder are created readable only by you (mode 0600/0700 on macOS and
Linux; on Windows they're in your user profile). Prefer a read-only database user for
browsing.

## Developing locally

### Prerequisites

- **Rust** 1.94 or newer (the 2024 edition): `rustup update stable`.
- **On Linux**, the desktop app needs the X11/Wayland development headers:
  `libxkbcommon`, `libwayland` and `libgl` (on Debian or Ubuntu:
  `sudo apt install libxkbcommon-dev libwayland-dev libgl1-mesa-dev`). SQL Server
  support builds OpenSSL from source there, which needs `perl` and `make`.
- **Docker**, to run the database tests against real servers (optional).

### Build and run

```sh
cargo run -p datadrop-gui                   # the desktop app
cargo run -p datadrop-cli -- --help         # the command line
cargo build --release                       # both, in target/release
```

Two features choose what's compiled in:

- `sqlserver` (on by default): SQL Server. Leave it out with `--no-default-features`.
- `duckdb` (off by default; the releases have it): DuckDB, built from its C++ source,
  which takes a while. `cargo run -p datadrop-gui --features duckdb`.

### Tests

```sh
cargo test --workspace --all-features
```

That runs everything that needs no server: the core's tests, SQLite and DuckDB, the
command line, and the desktop app's UI tests (with
[egui_kittest](https://crates.io/crates/egui_kittest), no window needed).

The PostgreSQL, MySQL, SQL Server and ClickHouse tests run when these variables point at
servers you can create schemas and databases on; they're skipped otherwise:

```sh
docker run -d --rm -p 5432:5432 -e POSTGRES_PASSWORD=test postgres:17
docker run -d --rm -p 3306:3306 -e MYSQL_ROOT_PASSWORD=test mysql:8.4
docker run -d --rm -p 1433:1433 -e ACCEPT_EULA=Y -e 'MSSQL_SA_PASSWORD=Test_pw_1234' \
  mcr.microsoft.com/mssql/server:2022-latest
docker run -d --rm -p 8123:8123 -e CLICKHOUSE_PASSWORD=test clickhouse/clickhouse-server

export DATADROP_TEST_POSTGRES_URL=postgres://postgres:test@127.0.0.1:5432/postgres
export DATADROP_TEST_MYSQL_URL=mysql://root:test@127.0.0.1:3306/mysql
export DATADROP_TEST_MSSQL_URL='sqlserver://sa:Test_pw_1234@127.0.0.1:1433/master'
export DATADROP_TEST_CLICKHOUSE_URL=clickhouse://default:test@127.0.0.1:8123/default
cargo test --workspace --all-features
```

The SSH tunnel tests need an SSH server in front of PostgreSQL: see
`crates/core/tests/ssh.rs` for their variables, and `.github/ssh-test/` and the CI
workflow for a server to run.

### Before a pull request

CI runs these on Linux, macOS and Windows, so run them first:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo deny --all-features check     # licences, advisories and sources
```

### Packaging, the icon and the designs

- `packaging/` makes the installers: `macos/package.sh` (the `.app` and `.dmg`),
  `windows/datadrop.iss` (Inno Setup) and `linux/package.sh` (`.deb` and `.rpm`). The
  release workflow runs them when a `v*` tag is pushed.
- `packaging/make-icon.py` draws the app icon with
  [photocraft-cli](https://github.com/storytold/photocraft) and writes every size.
- `design/0.3/` has mockups of every screen, made with
  [FigCraft](https://github.com/storytold/figcraft) from `screens.py`; its README says how
  to regenerate them.
- The desktop app's fonts (Inter and JetBrains Mono) are in `crates/gui/assets/fonts`,
  with their licences.

## Licence

MIT. See [LICENSE](LICENSE).
