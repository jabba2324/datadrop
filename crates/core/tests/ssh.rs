//! SSH tunnels, against a real SSH server in front of PostgreSQL. Set:
//!
//! - `DATADROP_TEST_SSH_HOST` / `_PORT` / `_USER` / `_PASSWORD`: the SSH server
//! - `DATADROP_TEST_SSH_KEY`: a private key it accepts (no passphrase)
//! - `DATADROP_TEST_SSH_KEY_PASSPHRASE_FILE` + `DATADROP_TEST_SSH_PASSPHRASE`: one
//!   with a passphrase
//! - `DATADROP_TEST_SSH_DB_URL`: PostgreSQL as the SSH server sees it
//!
//! Without them these tests pass without doing anything.

use datadrop_core::{
    CancellationToken, Client, Command, ConfigDir, CoreError, Driver, Output, Profile, SshAuth,
    SshTunnel,
};

struct Env {
    host: String,
    port: u16,
    user: String,
    password: String,
    key: String,
    key_with_passphrase: String,
    passphrase: String,
    db_url: String,
}

fn env() -> Option<Env> {
    let v = |k: &str| std::env::var(format!("DATADROP_TEST_SSH_{k}")).ok();
    let Some(host) = v("HOST") else {
        eprintln!("DATADROP_TEST_SSH_HOST isn't set; skipping");
        return None;
    };
    Some(Env {
        host,
        port: v("PORT").and_then(|p| p.parse().ok()).unwrap_or(22),
        user: v("USER")?,
        password: v("PASSWORD")?,
        key: v("KEY")?,
        key_with_passphrase: v("KEY_PASSPHRASE_FILE")?,
        passphrase: v("PASSPHRASE")?,
        db_url: v("DB_URL")?,
    })
}

fn profile(e: &Env, name: &str, auth: SshAuth) -> Profile {
    Profile {
        name: name.into(),
        driver: Driver::Postgres,
        url: Some(e.db_url.clone()),
        ssl_mode: datadrop_core::SslMode::Disable,
        ssh: Some(SshTunnel {
            host: e.host.clone(),
            port: Some(e.port),
            user: e.user.clone(),
            auth,
        }),
        ..Profile::default()
    }
}

async fn save(client: &Client, p: Profile) {
    client
        .execute(Command::ProfileSave {
            profile: p,
            previous_name: None,
            overwrite: true,
        })
        .await
        .unwrap();
}

async fn select_42(client: &Client, profile: &str) -> Result<String, CoreError> {
    match client
        .execute(Command::Query {
            profile: profile.into(),
            sql: "SELECT 40 + 2".into(),
            session: None,
            max_rows: None,
            autocommit: None,
        })
        .await?
    {
        Output::Query(q) => {
            if let Some(e) = q.error {
                panic!("{}", e.message);
            }
            Ok(q.results[0].rows[0][0].clone().unwrap())
        }
        o => panic!("{o:?}"),
    }
}

#[tokio::test]
async fn tunnels_after_trusting_the_host() {
    let Some(e) = env() else { return };
    let dir = tempfile::tempdir().unwrap();
    let client = Client::new(&ConfigDir::new(dir.path()));
    let password = SshAuth::Password {
        password: e.password.clone(),
    };
    save(&client, profile(&e, "pw", password.clone())).await;

    // First time: the host key isn't known.
    let err = select_42(&client, "pw").await.unwrap_err();
    let CoreError::UnknownHostKey {
        host,
        port,
        fingerprint,
        key,
    } = err
    else {
        panic!("{err}")
    };
    assert!(fingerprint.starts_with("SHA256:"), "{fingerprint}");
    client
        .execute(Command::SshTrustHost { host, port, key })
        .await
        .unwrap();
    assert_eq!(select_42(&client, "pw").await.unwrap(), "42");

    // Sessions, the schema browser and autocompletion go through it too, and cancel
    // works through it.
    let Output::Schemas { schemas } = client
        .execute(Command::Schemas {
            profile: "pw".into(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(schemas.contains(&"public".to_owned()));
    client
        .execute(Command::Catalog {
            profile: "pw".into(),
        })
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        stop.cancel();
    });
    let Output::Query(q) = client
        .execute_cancellable(
            Command::Query {
                profile: "pw".into(),
                sql: "SELECT pg_sleep(20)".into(),
                session: Some("tab".into()),
                max_rows: None,
                autocommit: None,
            },
            &cancel,
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(q.cancelled);

    // Keys, with and without a passphrase.
    save(
        &client,
        profile(
            &e,
            "key",
            SshAuth::Key {
                path: e.key.clone().into(),
                passphrase: None,
            },
        ),
    )
    .await;
    assert_eq!(select_42(&client, "key").await.unwrap(), "42");
    save(
        &client,
        profile(
            &e,
            "key-pp",
            SshAuth::Key {
                path: e.key_with_passphrase.clone().into(),
                passphrase: Some(e.passphrase.clone()),
            },
        ),
    )
    .await;
    assert_eq!(select_42(&client, "key-pp").await.unwrap(), "42");

    // Wrong credentials are reported as such.
    let wrong = profile(
        &e,
        "wrong",
        SshAuth::Password {
            password: "nope".into(),
        },
    );
    let err = client
        .execute(Command::ConnectionTest { profile: wrong })
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("didn't accept the password"),
        "{err}"
    );
    let bad_phrase = profile(
        &e,
        "bad",
        SshAuth::Key {
            path: e.key_with_passphrase.clone().into(),
            passphrase: Some("wrong".into()),
        },
    );
    let err = client
        .execute(Command::ConnectionTest {
            profile: bad_phrase,
        })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("passphrase"), "{err}");
}

#[tokio::test]
async fn refuses_a_changed_host_key() {
    let Some(e) = env() else { return };
    let dir = tempfile::tempdir().unwrap();
    // Record some other key for the host.
    let other = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ";
    let host = if e.port == 22 {
        e.host.clone()
    } else {
        format!("[{}]:{}", e.host, e.port)
    };
    std::fs::write(dir.path().join("known_hosts"), format!("{host} {other}\n")).unwrap();
    let client = Client::new(&ConfigDir::new(dir.path()));
    let p = profile(
        &e,
        "pw",
        SshAuth::Password {
            password: e.password.clone(),
        },
    );
    let err = client
        .execute(Command::ConnectionTest { profile: p })
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::HostKeyChanged { .. }), "{err}");
}
