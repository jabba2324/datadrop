# DataDrop

A fast, open-source SQL client for **PostgreSQL** and **MySQL**: a desktop app with a
schema browser, a highlighted SQL editor and a results grid, plus a command line that
can do everything the app can.

It runs on Windows, macOS (Apple Silicon) and Linux, and is written in Rust with
[egui](https://github.com/emilk/egui).

## Features

- **Connections** saved as profiles: host and port, or a connection URL, user,
  password, SSL mode and an optional CA certificate. Test before saving.
- **Schema browser**: schemas (databases on MySQL), tables and views, columns with
  types, keys, nullability and defaults. Double-click a table to see its rows.
- **SQL editor** with syntax highlighting, line numbers and tabs. Open, save and close
  `.sql` files; open files come back next time.
- **Run** the selection or the statement under the cursor (`Ctrl+Enter`, `⌘↩` on a
  Mac), or the whole tab (`Ctrl+Shift+Enter`). Scripts run statement by statement and
  stop at the first error.
- **Stop** a running query: it's cancelled on the server, not just abandoned.
- **Results grid** for every result set, with NULLs marked, and copy as text (pastes
  into a spreadsheet).
- **Find and replace** (`Ctrl+F`, `Ctrl+R`), with match case.
- Each tab keeps its own connection, so `SET`, temporary tables and transactions carry
  over between runs, as in `psql`.

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
# Save a connection (the password is asked for, not typed on the command line)
datadrop profile add shop --url postgres://alice@db.example.com/shop --ask-password
datadrop profile add local --driver mysql --host 127.0.0.1 --user root --database app --ask-password
datadrop profile list
datadrop profile test shop

# Look around
datadrop schemas shop
datadrop tables shop --schema public
datadrop columns shop orders

# Run SQL: an argument, a file, or standard input
datadrop query shop "select id, total from orders order by id desc limit 5"
datadrop query shop -f report.sql --format csv > report.csv
echo "select now()" | datadrop query shop --format json
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
- `gui.json`: the desktop app's settings and open files.

**Passwords are stored in the profile files as plain text.** The files and folder are
created readable only by you (mode 0600/0700 on macOS and Linux; on Windows they're
in your user profile). Don't keep production credentials on a shared machine, and
prefer a read-only database user for browsing.

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

The database tests run when these point at servers you can create schemas and
databases on (they're skipped otherwise):

```sh
docker run -d --rm -p 5432:5432 -e POSTGRES_PASSWORD=test postgres:17
docker run -d --rm -p 3306:3306 -e MYSQL_ROOT_PASSWORD=test mysql:8.4
export DATADROP_TEST_POSTGRES_URL=postgres://postgres:test@127.0.0.1:5432/postgres
export DATADROP_TEST_MYSQL_URL=mysql://root:test@127.0.0.1:3306/mysql
cargo test --workspace
```

## Layout

| Crate | What it is |
|---|---|
| `crates/core` | Profiles, connections, running SQL, the schema queries, and the SQL lexer. Every action is a `Command`. |
| `crates/cli` | The `datadrop` command line. |
| `crates/gui` | The `datadrop-gui` desktop app (egui). |

See [docs/DESIGN.md](docs/DESIGN.md) for how it fits together and what's next.

## Licence

MIT. See [LICENSE](LICENSE).
