# The `datadrop` command line

Everything the desktop app does, from a terminal: for scripts, CI and servers without a
desktop. Each command is the same `Command` the app sends to `datadrop-core`.

## Connections

```sh
# Save a connection (the password is asked for, not typed on the command line, and
# kept in the OS keychain; --passwords file keeps it in the profile file instead)
datadrop profile add shop --url postgres://alice@db.example.com/shop --ask-password
datadrop profile add local --driver mysql --host 127.0.0.1 --user root --database app --ask-password
datadrop profile add notes --driver sqlite --database ./notes.db --create-if-missing
datadrop profile add erp --url sqlserver://sa@erp.example.com:1433/sales --ask-password
datadrop profile add logs --url clickhouse://default@ch.example.com:8443/logs --ssl-mode require
datadrop profile add lake --driver duckdb --database ./lake.duckdb --create-if-missing
datadrop profile list
datadrop profile test shop

# Through an SSH server (agent by default; or --ssh-key FILE, --ssh-ask-password)
datadrop profile add prod --url postgres://app@10.0.0.5/app --ask-password --ssh ops@bastion.example.com
datadrop profile trust-host prod      # shows the server's key fingerprint to confirm

# Where passwords are kept
datadrop keychain                     # is there an OS keychain to use?
datadrop profile edit prod --passwords keychain
```

## Looking around

```sh
datadrop schemas shop
datadrop tables shop --schema public
datadrop columns shop orders
datadrop catalog shop                 # every table with its columns
datadrop schema shop                  # tables with keys, indexes, triggers; routines...
datadrop ddl shop orders              # the SQL that creates it (--routine, --sequence)
datadrop ddl shop --all > schema.sql  # everything in the schema, in an order that runs
```

## Running SQL

```sh
# An argument, a file, or standard input
datadrop query shop "select id, total from orders order by id desc limit 5"
datadrop query shop -f report.sql --format csv
echo "select now()" | datadrop query shop --format json

# Placeholders: :name, ? (MySQL, SQLite) or $1 (PostgreSQL)
datadrop query shop "select * from orders where customer_id = :id" --param id=42
datadrop query shop "select * from orders where placed > :since" \
  --param-sql since="now() - interval '1 day'"

# Export every row to a file (the format comes from the extension)
datadrop query shop "select * from orders" -o orders.csv

# Query plans
datadrop explain shop "select * from orders where customer_id = 42"
datadrop explain shop --analyze "select * from orders where customer_id = 42"
```

`Ctrl+C` during a query cancels it on the server.

## Schemas

```sh
# An ER diagram, as Mermaid (for Markdown) or Graphviz
datadrop diagram shop > schema.mmd
datadrop diagram shop --format dot | dot -Tsvg > schema.svg

# The SQL that makes staging's schema like dev's (--apply runs it)
datadrop diff dev staging --source-schema public --target-schema public

# Import a file into a table (columns matched by the header's names)
datadrop import shop customers customers.csv
datadrop import shop customers dump.csv --no-header --columns id,name,-,email
datadrop import shop customers customers.csv --preview   # just show what's in it
```

## Formatting and history

```sh
# Format SQL (no connection needed)
datadrop format report.sql --driver mysql
datadrop format -i report.sql          # rewrite the file

datadrop history list --search orders
datadrop history clear
```

## Machine-readable output

Every command takes `--json`, and `datadrop exec` runs a command given as JSON, exactly
as the desktop app sends it:

```sh
datadrop exec '{"command": "tables", "profile": "shop", "schema": "public"}'
```

`datadrop <command> --help` lists every option.
