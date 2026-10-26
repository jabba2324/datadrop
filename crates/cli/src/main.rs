//! `datadrop`: everything the desktop app does, from the command line.

use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use datadrop_core::{
    Client, Command, ConfigDir, CoreError, Driver, Output, Profile, QueryOutput, SslMode,
    StatementResult,
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
    },
    /// Queries run before, from the app and the command line.
    #[command(subcommand)]
    History(HistoryCmd),
    /// Run a command given as JSON (see the docs for the shape), printing JSON.
    Exec { command: String },
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
}

#[derive(Args, Default)]
struct ProfileFields {
    /// postgres, mysql or sqlite (default: from --url, else postgres).
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
    /// Ask for the password (it's saved in the profile file, readable only by you).
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
            }
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode> {
    let config = ConfigDir::default_location()?;
    let client = Client::new(&config);
    let json = cli.json;
    let (cmd, format) = match cli.command {
        Cmd::Profile(p) => (profile_command(&client, p)?, Format::Table),
        Cmd::Schemas { profile } => (Command::Schemas { profile }, Format::Table),
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
        } => {
            let sql = match (sql, file) {
                (Some(s), _) => s,
                (None, Some(f)) => std::fs::read_to_string(&f)
                    .with_context(|| format!("reading {}", f.display()))?,
                (None, None) => {
                    if std::io::stdin().is_terminal() {
                        bail!("give the SQL as an argument, with --file, or on standard input");
                    }
                    let mut s = String::new();
                    std::io::stdin().read_to_string(&mut s)?;
                    s
                }
            };
            let format = if json { Format::Json } else { format };
            (
                Command::Query {
                    profile,
                    sql,
                    session: None,
                    max_rows: Some(max_rows),
                },
                format,
            )
        }
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
            let mut p = Profile {
                name,
                ..Profile::default()
            };
            fields.apply(&mut p)?;
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
        Output::HistoryCleared { removed } => {
            eprintln!(
                "Removed {removed} entr{}.",
                if *removed == 1 { "y" } else { "ies" }
            )
        }
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
        Format::Csv => render::delimited(out, &headers, &r.rows, b',')?,
        Format::Tsv => render::delimited(out, &headers, &r.rows, b'\t')?,
        Format::Json => {}
    }
    Ok(())
}
