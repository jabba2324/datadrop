//! `datadrop`: everything the desktop app does, from the command line.

use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use datadrop_core::{
    Client, Command, ConfigDir, CoreError, Driver, ExportFormat, ObjectKind, Output,
    PasswordStorage, Profile, QueryOutput, SshAuth, SshTunnel, SslMode, StatementResult,
};

mod render;

#[derive(Parser)]
#[command(
    name = "datadrop",
    version,
    about = "Query PostgreSQL and MySQL databases"
)]
struct Cli {
    /// Print results and errors as JSON.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Manage connection profiles (saved in ~/.datadrop/profiles).
    #[command(subcommand)]
    Profile(ProfileCmd),
    /// List schemas (PostgreSQL), databases (MySQL) or attached databases (SQLite).
    Schemas { profile: String },
    /// List the tables and views in a schema.
    Tables {
        profile: String,
        /// Default: `public` (PostgreSQL) or the profile's database (MySQL).
        #[arg(long, short)]
        schema: Option<String>,
    },
    /// Show a table's columns.
    Columns {
        profile: String,
        table: String,
        #[arg(long, short)]
        schema: Option<String>,
    },
    /// Every table and view with its columns (what autocompletion uses).
    Catalog { profile: String },
    /// A schema's tables with their keys, indexes, foreign keys and triggers; views,
    /// routines and sequences.
    Schema {
        profile: String,
        #[arg(long, short)]
        schema: Option<String>,
    },
    /// The SQL that creates a table or view (with its indexes and triggers), a
    /// routine, a sequence, or (with --all) everything in the schema.
    Ddl {
        profile: String,
        /// The table or view (or, with --routine / --sequence, that).
        name: Option<String>,
        #[arg(long, short)]
        schema: Option<String>,
        #[arg(long, conflicts_with = "sequence")]
        routine: bool,
        #[arg(long)]
        sequence: bool,
        /// Everything in the schema.
        #[arg(long, conflicts_with_all = ["name", "routine", "sequence"])]
        all: bool,
    },
    /// Run SQL: from the argument, a file (-f), or standard input.
    Query {
        profile: String,
        /// The SQL. Several statements separated by `;` run in order.
        sql: Option<String>,
        /// Read the SQL from a file.
        #[arg(long, short, conflicts_with = "sql")]
        file: Option<PathBuf>,
        /// Keep at most this many rows per result.
        #[arg(long, default_value_t = datadrop_core::DEFAULT_MAX_ROWS)]
        max_rows: usize,
        #[arg(long, value_enum, default_value_t = Format::Table)]
        format: Format,
        /// Write the last statement's rows to this file instead (all of them: no row
        /// limit). The format comes from the extension (.csv, .tsv, .json) or --format.
        #[arg(long, short)]
        output: Option<PathBuf>,
        #[command(flatten)]
        params: Params,
    },
    /// Show how the database will run a statement (its query plan).
    Explain {
        profile: String,
        /// One statement.
        sql: Option<String>,
        #[arg(long, short, conflicts_with = "sql")]
        file: Option<PathBuf>,
        /// Run the statement and time each step. Statements that change data are
        /// rolled back afterwards.
        #[arg(long)]
        analyze: bool,
        #[command(flatten)]
        params: Params,
    },
    /// An entity-relationship diagram of a schema, as Mermaid (for Markdown) or
    /// Graphviz (`| dot -Tsvg > schema.svg`).
    Diagram {
        profile: String,
        #[arg(long, short)]
        schema: Option<String>,
        #[arg(long, value_parser = ["mermaid", "dot"], default_value = "mermaid")]
        format: String,
    },
    /// The SQL that makes the target schema like the source one: tables, columns,
    /// keys, indexes, triggers, views, routines and sequences.
    Diff {
        /// The profile with the schema as it should be.
        source: String,
        /// The profile whose schema changes.
        target: String,
        #[arg(long)]
        source_schema: Option<String>,
        #[arg(long)]
        target_schema: Option<String>,
        /// Run it on the target too.
        #[arg(long)]
        apply: bool,
    },
    /// Insert the rows of a CSV, TSV or JSON file into a table (in one transaction).
    Import {
        profile: String,
        table: String,
        file: PathBuf,
        #[arg(long, short)]
        schema: Option<String>,
        /// csv, tsv or json (default: from the extension).
        #[arg(long, value_parser = ["csv", "tsv", "json"])]
        format: Option<String>,
        /// The file has no header row (columns are then given with --columns).
        #[arg(long)]
        no_header: bool,
        /// The table columns the file's columns go into, in order; `-` leaves one out.
        /// Default: the header's names.
        #[arg(long, value_delimiter = ',')]
        columns: Option<Vec<String>>,
        /// Import empty fields as empty text rather than NULL.
        #[arg(long)]
        keep_empty: bool,
        /// Show the file's columns and first rows, and import nothing.
        #[arg(long)]
        preview: bool,
    },
    /// Queries run before, from the app and the command line.
    #[command(subcommand)]
    History(HistoryCmd),
    /// Whether passwords can be kept in the OS keychain here.
    Keychain,
    /// Format SQL (from a file or standard input): clauses on their own lines,
    /// keywords in capitals. Nothing is run.
    Format {
        file: Option<PathBuf>,
        /// The SQL dialect: postgres, mysql, sqlite or sqlserver.
        #[arg(long, default_value = "postgres")]
        driver: Driver,
        /// Rewrite the file instead of printing.
        #[arg(long, short, requires = "file")]
        in_place: bool,
    },
    /// Run a command given as JSON (see the docs for the shape), printing JSON.
    Exec { command: String },
}

/// Values for `:name`, `?` and `$1` placeholders.
#[derive(Args, Default)]
struct Params {
    /// A value for a placeholder: `--param id=42` fills `:id`, `--param 1=x` the first
    /// `?` (or `$1`). Numbers go in as they are, `null` as NULL, the rest as text.
    #[arg(long = "param", value_name = "NAME=VALUE")]
    values: Vec<String>,
    /// A value written into the SQL as it is: `--param-sql since="now() - interval '1 day'"`.
    #[arg(long = "param-sql", value_name = "NAME=SQL")]
    sql_values: Vec<String>,
}

impl Params {
    /// The SQL with its placeholders filled in.
    fn bind(&self, client: &Client, profile: &str, sql: String) -> Result<String> {
        use datadrop_core::params::{self, ParamMode};
        let dialect: datadrop_core::sql::Dialect =
            client.store().get_without_secrets(profile)?.driver.into();
        let found = params::parameters(&sql, dialect);
        if found.is_empty() {
            if !self.values.is_empty() || !self.sql_values.is_empty() {
                bail!("the SQL has no placeholders (:name, ?, $1) for --param");
            }
            return Ok(sql);
        }
        let mut given = std::collections::HashMap::new();
        for (list, mode) in [
            (&self.values, ParamMode::Auto),
            (&self.sql_values, ParamMode::Sql),
        ] {
            for v in list {
                let (name, value) = v
                    .split_once('=')
                    .with_context(|| format!("give --param as NAME=VALUE, not '{v}'"))?;
                given.insert(params::short_name(name.trim()).to_owned(), (value, mode));
            }
        }
        let mut values = std::collections::HashMap::new();
        let mut missing = vec![];
        for p in &found {
            match given.remove(params::short_name(&p.name)) {
                Some((v, mode)) => {
                    values.insert(p.name.clone(), params::value_sql(v, mode, dialect));
                }
                None => missing.push(p.name.clone()),
            }
        }
        if !missing.is_empty() {
            bail!(
                "the SQL needs a value for {} (--param {}=...)",
                missing.join(", "),
                params::short_name(&missing[0])
            );
        }
        if let Some(extra) = given.keys().next() {
            bail!("the SQL has no placeholder called {extra}");
        }
        Ok(params::bind(&sql, &found, &values))
    }
}

#[derive(Subcommand)]
enum HistoryCmd {
    /// List queries, newest first.
    List {
        #[arg(long, short)]
        profile: Option<String>,
        /// Only queries containing this text (ignoring case).
        #[arg(long, short)]
        search: Option<String>,
        #[arg(long, short = 'n', default_value_t = 50)]
        limit: usize,
        /// Print each query in full.
        #[arg(long)]
        full: bool,
    },
    /// Forget the history (all of it, or one profile's).
    Clear {
        #[arg(long, short)]
        profile: Option<String>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Table,
    Csv,
    Tsv,
    Json,
}

#[derive(Subcommand)]
enum ProfileCmd {
    /// List profiles.
    List,
    /// Show a profile (password masked).
    Show {
        name: String,
        #[arg(long)]
        show_password: bool,
    },
    /// Add a profile.
    Add {
        name: String,
        #[command(flatten)]
        fields: ProfileFields,
        /// Replace a profile with the same name.
        #[arg(long)]
        force: bool,
    },
    /// Change some settings of a profile (and rename it with --rename).
    Edit {
        name: String,
        #[command(flatten)]
        fields: ProfileFields,
        #[arg(long)]
        rename: Option<String>,
        /// Remove the saved password.
        #[arg(long)]
        clear_password: bool,
    },
    /// Delete a profile.
    Remove { name: String },
    /// Connect and show the server version.
    Test { name: String },
    /// Trust the SSH server a profile tunnels through (shows its key fingerprint
    /// first, to check).
    TrustHost {
        name: String,
        /// Don't ask; trust the key whatever it is.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Args, Default)]
struct ProfileFields {
    /// postgres, mysql, sqlite or sqlserver (default: from --url, else postgres).
    #[arg(long)]
    driver: Option<Driver>,
    /// A connection URL, e.g. postgres://user@host:5432/db or sqlite:data.db. Its
    /// host, port and database are used instead of --host, --port and --database.
    #[arg(long)]
    url: Option<String>,
    #[arg(long)]
    host: Option<String>,
    #[arg(long)]
    port: Option<u16>,
    /// The database name; for SQLite, the database file.
    #[arg(long)]
    database: Option<String>,
    /// SQLite: create the database file if it doesn't exist.
    #[arg(long)]
    create_if_missing: bool,
    #[arg(long, short)]
    user: Option<String>,
    /// Ask for the password.
    #[arg(long, conflicts_with = "password_stdin")]
    ask_password: bool,
    /// Read the password from standard input.
    #[arg(long)]
    password_stdin: bool,
    /// disable, prefer, require, verify-ca or verify-full.
    #[arg(long)]
    ssl_mode: Option<SslMode>,
    /// A CA certificate (PEM) to trust.
    #[arg(long)]
    ssl_root_cert: Option<PathBuf>,
    /// Connect through this SSH server (user@host or user@host:port).
    #[arg(long)]
    ssh: Option<String>,
    /// SSH sign-in with this private key file (default: your SSH agent).
    #[arg(long, requires = "ssh")]
    ssh_key: Option<PathBuf>,
    /// Ask for the SSH key's passphrase.
    #[arg(long, requires = "ssh_key")]
    ssh_ask_passphrase: bool,
    /// SSH sign-in with a password (asked for).
    #[arg(long, requires = "ssh", conflicts_with = "ssh_key")]
    ssh_ask_password: bool,
    /// Stop using an SSH tunnel.
    #[arg(long, conflicts_with = "ssh")]
    no_ssh: bool,
    /// Where to keep the passwords: keychain (the OS keychain) or file (the profile
    /// file, as plain text). New profiles use the keychain when there is one.
    #[arg(long)]
    passwords: Option<PasswordStorage>,
}

impl ProfileFields {
    fn apply(self, p: &mut Profile) -> Result<()> {
        if let Some(url) = self.url {
            if self.driver.is_none()
                && let Some(d) = Driver::from_url(&url)
            {
                p.driver = d;
            }
            p.url = Some(url);
        }
        if let Some(d) = self.driver {
            p.driver = d;
        }
        if let Some(h) = self.host {
            p.host = h;
        }
        if self.port.is_some() {
            p.port = self.port;
        }
        if let Some(d) = self.database {
            // A relative SQLite path would depend on where you run DataDrop from.
            p.database = if p.driver.is_file() {
                std::path::absolute(&d)
                    .map(|a| a.display().to_string())
                    .unwrap_or(d)
            } else {
                d
            };
        }
        if self.create_if_missing {
            p.create_if_missing = true;
        }
        if let Some(u) = self.user {
            p.user = u;
        }
        if let Some(m) = self.ssl_mode {
            p.ssl_mode = m;
        }
        if self.ssl_root_cert.is_some() {
            p.ssl_root_cert = self.ssl_root_cert;
        }
        if let Some(target) = self.ssh {
            let (user, host) = target
                .split_once('@')
                .context("give the SSH server as user@host or user@host:port")?;
            let (host, port) = match host.rsplit_once(':') {
                Some((h, port)) if !h.contains(':') || h.starts_with('[') => (
                    h.trim_matches(['[', ']']).to_owned(),
                    Some(port.parse::<u16>().context("the SSH port isn't a number")?),
                ),
                _ => (host.trim_matches(['[', ']']).to_owned(), None),
            };
            let auth = if let Some(path) = self.ssh_key {
                let passphrase = if self.ssh_ask_passphrase {
                    Some(rpassword::prompt_password("SSH key passphrase: ")?)
                } else {
                    None
                };
                SshAuth::Key {
                    path: std::path::absolute(&path).unwrap_or(path),
                    passphrase,
                }
            } else if self.ssh_ask_password {
                SshAuth::Password {
                    password: rpassword::prompt_password("SSH password: ")?,
                }
            } else {
                SshAuth::Agent
            };
            p.ssh = Some(SshTunnel {
                host,
                port,
                user: user.to_owned(),
                auth,
            });
        }
        if self.no_ssh {
            p.ssh = None;
        }
        if let Some(storage) = self.passwords {
            p.passwords = storage;
        }
        if self.ask_password {
            p.password = Some(rpassword::prompt_password("Password: ")?);
        } else if self.password_stdin {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            p.password = Some(s.trim_end_matches(['\r', '\n']).to_owned());
        }
        Ok(())
    }
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("DATADROP_LOG")
                .unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let json = cli.json || matches!(cli.command, Cmd::Exec { .. });
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    match rt.block_on(run(cli)) {
        Ok(code) => code,
        Err(e) => {
            if json {
                let err = match e.downcast_ref::<CoreError>() {
                    Some(ce) => serde_json::to_value(ce).expect("serialisable"),
                    None => serde_json::json!({ "kind": "cli", "message": format!("{e:#}") }),
                };
                println!("{}", serde_json::json!({ "error": err }));
            } else {
                eprintln!("datadrop: {e:#}");
                if let Some(CoreError::UnknownHostKey { .. }) = e.downcast_ref::<CoreError>() {
                    eprintln!("To trust it: datadrop profile trust-host <profile>");
                }
            }
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode> {
    if let Cmd::Format {
        file,
        driver,
        in_place,
    } = &cli.command
    {
        let sql = read_sql(None, file.clone())?;
        let formatted = datadrop_core::format::format_sql(&sql, (*driver).into());
        match file {
            Some(f) if *in_place => {
                std::fs::write(f, formatted).with_context(|| format!("writing {}", f.display()))?
            }
            _ => print!("{formatted}"),
        }
        return Ok(ExitCode::SUCCESS);
    }
    let config = ConfigDir::default_location()?;
    let client = Client::new(&config);
    let json = cli.json;
    let (cmd, format) = match cli.command {
        Cmd::Profile(ProfileCmd::TrustHost { name, yes }) => {
            return trust_host(&client, &name, yes).await;
        }
        Cmd::Profile(p) => (profile_command(&client, p)?, Format::Table),
        Cmd::Schemas { profile } => (Command::Schemas { profile }, Format::Table),
        Cmd::Catalog { profile } => (Command::Catalog { profile }, Format::Table),
        Cmd::Schema { profile, schema } => (Command::Schema { profile, schema }, Format::Table),
        Cmd::Diagram {
            profile,
            schema,
            format,
        } => {
            let out = client.execute(Command::Schema { profile, schema }).await?;
            let Output::Schema(m) = out else {
                bail!("unexpected reply");
            };
            let text = if format == "dot" {
                datadrop_core::diagram::dot(&m)
            } else {
                datadrop_core::diagram::mermaid(&m)
            };
            print!("{text}");
            return Ok(ExitCode::SUCCESS);
        }
        Cmd::Ddl {
            profile,
            name,
            schema,
            routine,
            sequence,
            all,
        } => {
            let kind = if all {
                ObjectKind::Schema
            } else if routine {
                ObjectKind::Routine
            } else if sequence {
                ObjectKind::Sequence
            } else {
                ObjectKind::Table
            };
            if kind != ObjectKind::Schema && name.is_none() {
                bail!("say which object, or --all for the whole schema");
            }
            (
                Command::Ddl {
                    profile,
                    schema,
                    kind,
                    name: name.unwrap_or_default(),
                },
                Format::Table,
            )
        }
        Cmd::Tables { profile, schema } => (Command::Tables { profile, schema }, Format::Table),
        Cmd::Columns {
            profile,
            table,
            schema,
        } => (
            Command::Columns {
                profile,
                schema,
                table,
            },
            Format::Table,
        ),
        Cmd::Query {
            profile,
            sql,
            file,
            max_rows,
            format,
            output,
            params,
        } => {
            let sql = params.bind(&client, &profile, read_sql(sql, file)?)?;
            if let Some(path) = output {
                let format = match format {
                    Format::Csv => Some(ExportFormat::Csv),
                    Format::Tsv => Some(ExportFormat::Tsv),
                    Format::Json => Some(ExportFormat::Json),
                    Format::Table => None,
                };
                (
                    Command::Export {
                        profile,
                        sql,
                        path,
                        format,
                        session: None,
                    },
                    Format::Table,
                )
            } else {
                let format = if json { Format::Json } else { format };
                (
                    Command::Query {
                        profile,
                        sql,
                        session: None,
                        max_rows: Some(max_rows),
                        autocommit: None,
                    },
                    format,
                )
            }
        }
        Cmd::Explain {
            profile,
            sql,
            file,
            analyze,
            params,
        } => (
            Command::Explain {
                sql: params.bind(&client, &profile, read_sql(sql, file)?)?,
                profile,
                analyze,
                session: None,
            },
            Format::Table,
        ),
        Cmd::History(HistoryCmd::List {
            profile,
            search,
            limit,
            full,
        }) => (
            Command::HistoryList {
                profile,
                search,
                limit: Some(limit),
            },
            if full { Format::Tsv } else { Format::Table },
        ),
        Cmd::History(HistoryCmd::Clear { profile }) => {
            (Command::HistoryClear { profile }, Format::Table)
        }
        Cmd::Keychain => (Command::KeychainStatus, Format::Table),
        Cmd::Diff {
            source,
            target,
            source_schema,
            target_schema,
            apply,
        } => (
            Command::SchemaDiff {
                source_profile: source,
                source_schema,
                target_profile: target,
                target_schema,
                apply,
            },
            Format::Table,
        ),
        Cmd::Import {
            profile,
            table,
            file,
            schema,
            format,
            no_header,
            columns,
            keep_empty,
            preview,
        } => {
            let format = format.map(|f| match f.as_str() {
                "tsv" => ExportFormat::Tsv,
                "json" => ExportFormat::Json,
                _ => ExportFormat::Csv,
            });
            if preview {
                (
                    Command::ImportPreview {
                        path: file,
                        format,
                        header: !no_header,
                    },
                    Format::Table,
                )
            } else {
                (
                    Command::Import {
                        profile,
                        schema,
                        table,
                        path: file,
                        format,
                        header: !no_header,
                        empty_as_null: !keep_empty,
                        columns: columns.map(|c| {
                            c.into_iter()
                                .map(|n| (n.trim() != "-").then(|| n.trim().to_owned()))
                                .collect()
                        }),
                    },
                    Format::Table,
                )
            }
        }
        Cmd::Format { .. } => unreachable!("handled above"),
        Cmd::Exec { command } => {
            let cmd: Command =
                serde_json::from_str(&command).context("parsing the JSON command")?;
            let out = client.execute(cmd).await?;
            println!("{}", serde_json::to_string_pretty(&out)?);
            return Ok(exit_code(&out));
        }
    };

    // Ctrl+C cancels a running query on the server rather than leaving it running.
    let cancel = datadrop_core::CancellationToken::new();
    let stop = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            stop.cancel();
        }
    });
    let out = client.execute_cancellable(cmd, &cancel).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        print_output(&out, format)?;
    }
    Ok(exit_code(&out))
}

/// Connect through a profile's SSH server; if its key isn't known yet, show the
/// fingerprint and (once confirmed) trust it.
async fn trust_host(client: &Client, name: &str, yes: bool) -> Result<ExitCode> {
    let p = client.store().get(name)?;
    if p.ssh.is_none() {
        bail!("the profile '{name}' doesn't use an SSH tunnel");
    }
    match client
        .execute(Command::ProfileTest {
            name: name.to_owned(),
        })
        .await
    {
        Ok(_) => {
            eprintln!("The SSH server is already trusted, and the connection works.");
            Ok(ExitCode::SUCCESS)
        }
        Err(CoreError::UnknownHostKey {
            host,
            port,
            fingerprint,
            key,
        }) => {
            eprintln!(
                "The SSH server {host}:{port} has the key fingerprint\n\n    {fingerprint}\n"
            );
            if !yes {
                if !std::io::stdin().is_terminal() {
                    bail!("check the fingerprint, then run again with --yes to trust it");
                }
                eprint!("Is that the server's real key (check with its admin)? Trust it? [y/N] ");
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !matches!(answer.trim(), "y" | "Y" | "yes") {
                    eprintln!("Not trusted.");
                    return Ok(ExitCode::FAILURE);
                }
            }
            client
                .execute(Command::SshTrustHost { host, port, key })
                .await?;
            eprintln!("Trusted.");
            let out = client
                .execute(Command::ProfileTest {
                    name: name.to_owned(),
                })
                .await?;
            if let Output::Connected { server_version } = out {
                println!("{server_version}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => Err(e.into()),
    }
}

/// SQL from the argument, a file, or standard input.
fn read_sql(sql: Option<String>, file: Option<PathBuf>) -> Result<String> {
    Ok(match (sql, file) {
        (Some(s), _) => s,
        (None, Some(f)) => {
            std::fs::read_to_string(&f).with_context(|| format!("reading {}", f.display()))?
        }
        (None, None) => {
            if std::io::stdin().is_terminal() {
                bail!("give the SQL as an argument, with --file, or on standard input");
            }
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            s
        }
    })
}

fn exit_code(out: &Output) -> ExitCode {
    match out {
        Output::Query(q) if q.error.is_some() => ExitCode::FAILURE,
        _ => ExitCode::SUCCESS,
    }
}

fn profile_command(client: &Client, cmd: ProfileCmd) -> Result<Command> {
    Ok(match cmd {
        ProfileCmd::List => Command::ProfileList,
        ProfileCmd::Show {
            name,
            show_password,
        } => Command::ProfileGet {
            name,
            show_password,
        },
        ProfileCmd::Add {
            name,
            fields,
            force,
        } => {
            let explicit = fields.passwords.is_some();
            let mut p = Profile {
                name,
                ..Profile::default()
            };
            fields.apply(&mut p)?;
            if !explicit {
                match client.store().keychain().check() {
                    Ok(()) => p.passwords = PasswordStorage::Keychain,
                    Err(why) => {
                        if p.has_secrets() {
                            eprintln!("Note: the passwords are saved in the profile file: {why}.");
                        }
                    }
                }
            }
            Command::ProfileSave {
                profile: p,
                previous_name: None,
                overwrite: force,
            }
        }
        ProfileCmd::Edit {
            name,
            fields,
            rename,
            clear_password,
        } => {
            let mut p = client.store().get(&name)?;
            fields.apply(&mut p)?;
            if clear_password {
                p.password = None;
            }
            if let Some(new) = rename {
                p.name = new;
            }
            Command::ProfileSave {
                profile: p,
                previous_name: Some(name),
                overwrite: false,
            }
        }
        ProfileCmd::Remove { name } => Command::ProfileRemove { name },
        ProfileCmd::Test { name } => Command::ProfileTest { name },
        ProfileCmd::TrustHost { .. } => unreachable!("handled in run"),
    })
}

fn print_output(out: &Output, format: Format) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    match out {
        Output::Profiles { profiles } => {
            if profiles.is_empty() {
                eprintln!("No profiles yet. Add one with `datadrop profile add <name> --url ...`.");
            }
            let rows: Vec<Vec<Option<String>>> = profiles
                .iter()
                .map(|p| {
                    vec![
                        Some(p.name.clone()),
                        Some(p.driver.to_string()),
                        Some(p.target.clone()),
                    ]
                })
                .collect();
            render::table(&mut stdout, &["NAME", "DRIVER", "TARGET"], &rows)?;
        }
        Output::Profile { profile } => {
            writeln!(stdout, "{}", serde_json::to_string_pretty(profile)?)?;
        }
        Output::ProfileSaved { name } => eprintln!("Saved profile '{name}'."),
        Output::ProfileRemoved { name } => eprintln!("Removed profile '{name}'."),
        Output::Connected { server_version } => writeln!(stdout, "{server_version}")?,
        Output::Schemas { schemas } => {
            for s in schemas {
                writeln!(stdout, "{s}")?;
            }
        }
        Output::Tables { tables, .. } => {
            let rows: Vec<Vec<Option<String>>> = tables
                .iter()
                .map(|t| vec![Some(t.name.clone()), Some(render::kind(t.kind).to_owned())])
                .collect();
            render::table(&mut stdout, &["NAME", "KIND"], &rows)?;
        }
        Output::Columns { columns, .. } => {
            let rows: Vec<Vec<Option<String>>> = columns
                .iter()
                .map(|c| {
                    vec![
                        Some(c.name.clone()),
                        Some(c.data_type.clone()),
                        Some(if c.nullable { "yes" } else { "no" }.into()),
                        c.default.clone(),
                        Some(if c.primary_key { "yes" } else { "" }.into()),
                    ]
                })
                .collect();
            render::table(
                &mut stdout,
                &["COLUMN", "TYPE", "NULL", "DEFAULT", "KEY"],
                &rows,
            )?;
        }
        Output::Query(q) => print_query(&mut stdout, q, format)?,
        Output::SessionClosed => {}
        Output::History { entries } => {
            let now = datadrop_core::history::now();
            if matches!(format, Format::Tsv) {
                // --full: every query as it was run.
                for e in entries.iter().rev() {
                    writeln!(
                        stdout,
                        "-- {} on {} ({} ms){}\n{}\n",
                        datadrop_core::history::ago(e.at, now),
                        e.profile,
                        e.elapsed_ms,
                        e.error
                            .as_deref()
                            .map(|m| format!(", failed: {m}"))
                            .unwrap_or_default(),
                        e.sql
                    )?;
                }
                return Ok(());
            }
            let rows: Vec<Vec<Option<String>>> = entries
                .iter()
                .map(|e| {
                    vec![
                        Some(datadrop_core::history::ago(e.at, now)),
                        Some(e.profile.clone()),
                        Some(format!("{} ms", e.elapsed_ms)),
                        Some(match (&e.error, e.rows) {
                            (Some(_), _) => "failed".into(),
                            (None, Some(n)) => n.to_string(),
                            (None, None) => String::new(),
                        }),
                        Some(render::one_line(&e.sql, 80)),
                    ]
                })
                .collect();
            render::table(
                &mut stdout,
                &["WHEN", "PROFILE", "TIME", "ROWS", "SQL"],
                &rows,
            )?;
        }
        Output::Plan(p) => render::plan(&mut stdout, p)?,
        Output::HostTrusted { host } => eprintln!("Trusted the SSH server {host}."),
        Output::EditsApplied { statements } => {
            eprintln!("Saved: {statements} statement(s) committed.")
        }
        Output::Catalog(c) => {
            for t in &c.tables {
                let name = if t.schema == c.default_schema {
                    t.name.clone()
                } else {
                    format!("{}.{}", t.schema, t.name)
                };
                let kind = if t.view { " (view)" } else { "" };
                writeln!(stdout, "{name}{kind}: {}", t.columns.join(", "))?;
            }
        }
        Output::Exported { path, rows } => {
            eprintln!(
                "Wrote {rows} row{} to {path}.",
                if *rows == 1 { "" } else { "s" }
            )
        }
        Output::HistoryCleared { removed } => {
            eprintln!(
                "Removed {removed} entr{}.",
                if *removed == 1 { "y" } else { "ies" }
            )
        }
        Output::Schema(m) => render::schema(&mut stdout, m)?,
        Output::Migration { migration, applied } => {
            if migration.statements.is_empty() {
                eprintln!("Nothing to change: they're the same.");
            } else {
                write!(stdout, "{}", migration.script())?;
                if *applied {
                    eprintln!("Applied {} statements.", migration.statements.len());
                }
            }
        }
        Output::ImportPreview(p) => {
            let names: Vec<&str> = p.columns.iter().map(String::as_str).collect();
            render::table(&mut stdout, &names, &p.rows)?;
        }
        Output::Imported { rows } => {
            eprintln!("Imported {rows} row{}.", if *rows == 1 { "" } else { "s" })
        }
        Output::Transaction { in_transaction } => {
            if *in_transaction {
                eprintln!("A transaction is open.");
            }
        }
        Output::Ddl { sql } => write!(stdout, "{sql}")?,
        Output::Keychain {
            name,
            available,
            error,
        } => match error {
            None if *available => writeln!(stdout, "Passwords can be kept in {name}.")?,
            _ => writeln!(
                stdout,
                "Passwords can't be kept in a keychain: {}.",
                error.as_deref().unwrap_or("unknown reason")
            )?,
        },
    }
    Ok(())
}

fn print_query(out: &mut impl Write, q: &QueryOutput, format: Format) -> Result<()> {
    let many = q.results.len() > 1;
    for (i, r) in q.results.iter().enumerate() {
        if many && matches!(format, Format::Table) {
            if i > 0 {
                writeln!(out)?;
            }
            writeln!(out, "-- {}", render::one_line(&r.sql, 70))?;
        }
        print_result(out, r, format)?;
    }
    if let Some(e) = &q.error {
        if q.cancelled {
            eprintln!("Cancelled.");
        } else {
            if many || !q.results.is_empty() {
                eprintln!(
                    "Statement {} failed: {}",
                    e.index + 1,
                    render::one_line(&e.sql, 70)
                );
            }
            eprintln!("error: {}", e.message);
        }
    }
    Ok(())
}

fn print_result(out: &mut impl Write, r: &StatementResult, format: Format) -> Result<()> {
    if !r.returns_rows {
        if matches!(format, Format::Table) {
            eprintln!(
                "OK, {} row(s) affected ({} ms)",
                r.rows_affected, r.elapsed_ms
            );
        }
        return Ok(());
    }
    let headers: Vec<&str> = r.columns.iter().map(|c| c.name.as_str()).collect();
    match format {
        Format::Table => {
            render::table(out, &headers, &r.rows)?;
            let more = if r.truncated {
                " (more not shown; see --max-rows)"
            } else {
                ""
            };
            eprintln!("{} row(s){more} ({} ms)", r.rows.len(), r.elapsed_ms);
        }
        Format::Csv => {
            datadrop_core::export::write_all(out, ExportFormat::Csv, &headers, &r.rows)?;
        }
        Format::Tsv => {
            datadrop_core::export::write_all(out, ExportFormat::Tsv, &headers, &r.rows)?;
        }
        Format::Json => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn cli_is_consistent() {
        use clap::CommandFactory;
        super::Cli::command().debug_assert();
    }
}
