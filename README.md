# DataDrop

A fast, open-source SQL client for **PostgreSQL**, **MySQL** and **SQLite**: a desktop
app with a schema browser, a highlighted SQL editor with autocompletion and a results
grid you can edit, plus a command line that can do everything the app can.

It runs on Windows, macOS (Apple Silicon) and Linux, and is written in Rust with
[egui](https://github.com/emilk/egui).

## Features

- **Connections** saved as profiles: host and port, or a connection URL, user,
  password, SSL mode and an optional CA certificate; or a SQLite file. Test before
  saving. Passwords go in the OS keychain (macOS Keychain, Windows Credential
  Manager, or the Secret Service on Linux), or in the profile file if you prefer.
- **SSH tunnels**: reach a database through an SSH server, signing in with your SSH
  agent, a key file or a password. Host keys are checked (unknown ones show their
  fingerprint to confirm).
- **Schema browser**: schemas (databases on MySQL); tables with their columns, keys,
  foreign keys, indexes and triggers; views, functions and procedures, sequences.
  Double-click a table to see its rows; right-click for its DDL (or the whole
  schema's).
- **SQL editor** with syntax highlighting, line numbers and tabs. Open, save and close
  `.sql` files; open files come back next time.
- **Autocompletion** of tables, columns (of the tables the statement uses, or after
  `alias.`), schemas and keywords, as you type or with `Ctrl+Space`.
- **Run** the selection or the statement under the cursor (`Ctrl+Enter`, `⌘↩` on a
  Mac), or the whole tab (`Ctrl+Shift+Enter`). Scripts run statement by statement and
  stop at the first error. MySQL scripts can change the `DELIMITER`.
- **Stop** a running query: it's cancelled on the server, not just abandoned.
- **Parameters**: write `:name` (or `?`, or `$1` on PostgreSQL) and you're asked for
  the values when it runs; the last ones are remembered per tab.
- **Results grid** for every result set, with NULLs marked, copy as text, and
  **export** to CSV, TSV or JSON (every row, not just those shown). Click a header
  to sort; type a `WHERE` condition to filter. Both run on the server when the
  result was cut short, so they cover every row.
- **Edit data in the grid**: double-click a cell, set NULL, add or delete rows. The
  `UPDATE`/`INSERT`/`DELETE` statements are shown before they run, in one transaction.
- **Explain** (`Ctrl+E`) and **Explain analyze** (`Ctrl+Shift+E`): the query plan as a
  tree, with each step's estimated and actual rows and its share of the time.
- **Query history** of everything run, searchable, from the app and the command line.
- **Find and replace** (`Ctrl+F`, `Ctrl+R`), with match case.
- Each tab keeps its own connection, so `SET`, temporary tables and transactions carry
  over between runs, as in `psql`.
- **Transactions**: turn off **Auto-commit** for a tab and what you run stays in a
  transaction until you **Commit** or **Roll back**; the tab is flagged while one is
  open (also when you begin one yourself), and closing it asks first.

## Install

Download a build for your system from the
[releases page](https://github.com/jabba2324/datadrop/releases): `datadrop-gui` is the
desktop app, `datadrop` the command line.

Each release has a `SHA256SUMS` file signed with [minisign](https://jedisct1.github.io/minisign/)
(when signing is set up for that release):

```sh
minisign -Vm SHA256SUMS -p minisign.pub
sha256sum -c --ignore-missing SHA256SUMS
```

On macOS the app isn't notarised yet; the first time, right-click it and choose Open.

## Command line

```sh
# Save a connection (the password is asked for, not typed on the command line, and
# kept in the OS keychain; --passwords file keeps it in the profile file instead)
datadrop profile add shop --url postgres://alice@db.example.com/shop --ask-password
datadrop profile add local --driver mysql --host 127.0.0.1 --user root --database app --ask-password
datadrop profile add notes --driver sqlite --database ./notes.db --create-if-missing
datadrop profile list
datadrop profile test shop

# Through an SSH server (agent by default; or --ssh-key FILE, --ssh-ask-password)
datadrop profile add prod --url postgres://app@10.0.0.5/app --ask-password --ssh ops@bastion.example.com
datadrop profile trust-host prod      # shows the server's key fingerprint to confirm

# Look around
datadrop schemas shop
datadrop tables shop --schema public
datadrop columns shop orders
datadrop catalog shop                 # every table with its columns
datadrop schema shop                  # tables with keys, indexes, triggers; routines...
datadrop ddl shop orders              # the SQL that creates it (--routine, --sequence)
datadrop ddl shop --all > schema.sql  # everything in the schema, in an order that runs

# Run SQL: an argument, a file, or standard input
datadrop query shop "select id, total from orders order by id desc limit 5"
datadrop query shop -f report.sql --format csv
echo "select now()" | datadrop query shop --format json

# Placeholders: :name, ? (MySQL, SQLite) or $1 (PostgreSQL)
datadrop query shop "select * from orders where customer_id = :id" --param id=42
datadrop query shop "select * from orders where placed > :since" \
  --param-sql since="now() - interval '1 day'"

# Export every row to a file (format from the extension)
datadrop query shop "select * from orders" -o orders.csv

# Query plans
datadrop explain shop "select * from orders where customer_id = 42"
datadrop explain shop --analyze "select * from orders where customer_id = 42"

# History
datadrop history list --search orders
datadrop history clear
```

Every command takes `--json` for machine-readable output, and `datadrop exec` runs a
command given as JSON, exactly as the desktop app sends it:

```sh
datadrop exec '{"command": "tables", "profile": "shop", "schema": "public"}'
```

`Ctrl+C` during a query cancels it on the server.

## Where things are kept

Everything lives in `~/.datadrop` (set `DATADROP_HOME` to use another folder):

- `profiles/<name>.json`: one file per connection.
- `history.jsonl`: the query history (the newest 5,000 queries).
- `known_hosts`: SSH servers trusted in DataDrop (your `~/.ssh/known_hosts` is also
  read).
- `gui.json`: the desktop app's settings and open files.

Passwords (database and SSH) and key passphrases are kept in the OS keychain, under
"DataDrop": the macOS Keychain, Windows Credential Manager, or the Secret Service on
Linux (GNOME Keyring, KWallet). `datadrop keychain` says whether there's one to use.
Without one (a server with no desktop session, say), or if you choose
`--passwords file` / "In the profile file", **they're stored in the profile file as
plain text**. Move an existing profile with
`datadrop profile edit <name> --passwords keychain`.

The files and folder are created readable only by you (mode 0600/0700 on macOS and
Linux; on Windows they're in your user profile). Prefer a read-only database user for
browsing.

## Building from source

You need Rust (stable). On Linux, the desktop app also needs the usual X11/Wayland
development headers (`libxkbcommon`, `libwayland`, `libgl`).

```sh
cargo build --release
./target/release/datadrop --help
./target/release/datadrop-gui
```

Tests:

```sh
cargo test --workspace
```

SQLite tests always run. The PostgreSQL and MySQL tests run when these point at
servers you can create schemas and databases on (they're skipped otherwise):

```sh
docker run -d --rm -p 5432:5432 -e POSTGRES_PASSWORD=test postgres:17
docker run -d --rm -p 3306:3306 -e MYSQL_ROOT_PASSWORD=test mysql:8.4
export DATADROP_TEST_POSTGRES_URL=postgres://postgres:test@127.0.0.1:5432/postgres
export DATADROP_TEST_MYSQL_URL=mysql://root:test@127.0.0.1:3306/mysql
cargo test --workspace
```

The SSH tunnel tests need an SSH server in front of PostgreSQL: see
`crates/core/tests/ssh.rs` for the variables, and the CI workflow for a setup.

## Layout

| Crate | What it is |
|---|---|
| `crates/core` | Profiles, connections and SSH tunnels, running SQL, schema queries, plans, history, export, autocompletion, and the SQL lexer. Every action is a `Command`. |
| `crates/cli` | The `datadrop` command line. |
| `crates/gui` | The `datadrop-gui` desktop app (egui). |

See [docs/DESIGN.md](docs/DESIGN.md) for how it fits together and what's next.

## Licence

MIT. See [LICENSE](LICENSE).
