//! Connection profiles: one JSON file per profile in `<config>/profiles/`.
//!
//! Passwords are stored in those files as plain text. Files are created readable only
//! by the current user (0600 on Unix; on Windows they inherit the user-only ACL of the
//! home folder).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::keychain::Keychain;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Driver {
    #[default]
    Postgres,
    Mysql,
    Sqlite,
}

impl Driver {
    pub const ALL: [Driver; 3] = [Driver::Postgres, Driver::Mysql, Driver::Sqlite];

    pub fn default_port(self) -> u16 {
        match self {
            Self::Postgres => 5432,
            Self::Mysql => 3306,
            Self::Sqlite => 0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Postgres => "PostgreSQL",
            Self::Mysql => "MySQL",
            Self::Sqlite => "SQLite",
        }
    }

    /// A database file rather than a server.
    pub fn is_file(self) -> bool {
        self == Self::Sqlite
    }

    /// The driver a connection URL is for, from its scheme.
    pub fn from_url(url: &str) -> Option<Self> {
        let scheme = url
            .split_once("://")
            .map_or("", |s| s.0)
            .to_ascii_lowercase();
        match scheme.as_str() {
            "postgres" | "postgresql" => Some(Self::Postgres),
            "mysql" | "mariadb" => Some(Self::Mysql),
            _ => None,
        }
        .or_else(|| url.starts_with("sqlite:").then_some(Self::Sqlite))
    }
}

impl std::str::FromStr for Driver {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "postgres" | "postgresql" | "pg" => Ok(Self::Postgres),
            "mysql" | "mariadb" => Ok(Self::Mysql),
            "sqlite" | "sqlite3" => Ok(Self::Sqlite),
            _ => Err(format!(
                "unknown driver '{s}' (use postgres, mysql or sqlite)"
            )),
        }
    }
}

impl std::fmt::Display for Driver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Postgres => "postgres",
            Self::Mysql => "mysql",
            Self::Sqlite => "sqlite",
        })
    }
}

/// TLS for the connection, with PostgreSQL's names (MySQL's map onto them).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SslMode {
    /// Never use TLS.
    Disable,
    /// Use TLS if the server offers it (without checking its certificate).
    #[default]
    Prefer,
    /// Always use TLS, without checking the certificate.
    Require,
    /// TLS, and the certificate must be signed by a trusted CA.
    VerifyCa,
    /// TLS, a trusted certificate, and it must match the host name.
    VerifyFull,
}

impl SslMode {
    pub const ALL: [SslMode; 5] = [
        SslMode::Disable,
        SslMode::Prefer,
        SslMode::Require,
        SslMode::VerifyCa,
        SslMode::VerifyFull,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Prefer => "prefer",
            Self::Require => "require",
            Self::VerifyCa => "verify-ca",
            Self::VerifyFull => "verify-full",
        }
    }
}

impl std::str::FromStr for SslMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|m| m.as_str() == s.to_ascii_lowercase())
            .ok_or_else(|| {
                format!("unknown SSL mode '{s}' (disable, prefer, require, verify-ca, verify-full)")
            })
    }
}

/// Reach the database through an SSH server: DataDrop connects to it and forwards
/// the database connection from there (like `ssh -L`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SshTunnel {
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    pub user: String,
    pub auth: SshAuth,
}

/// How to sign in to the SSH server.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum SshAuth {
    /// The keys in your SSH agent (ssh-agent, or Pageant / the OpenSSH agent on
    /// Windows).
    #[default]
    Agent,
    Password {
        password: String,
    },
    /// A private key file, with its passphrase if it has one.
    Key {
        path: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        passphrase: Option<String>,
    },
}

impl SshTunnel {
    pub fn port(&self) -> u16 {
        self.port.unwrap_or(22)
    }
}

/// How to reach one database.
///
/// Either fill in the fields, or give a connection `url`. With a URL, its host, port
/// and database are used; `user` and `password` here fill in anything it leaves out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    pub driver: Driver,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub database: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub user: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    pub ssl_mode: SslMode,
    /// A CA certificate (PEM) to trust, for servers with a private CA.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssl_root_cert: Option<PathBuf>,
    /// SQLite: create the database file if it doesn't exist.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub create_if_missing: bool,
    /// Connect through an SSH server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh: Option<SshTunnel>,
    /// Where the passwords (and any key passphrase) are kept.
    #[serde(skip_serializing_if = "PasswordStorage::is_file")]
    pub passwords: PasswordStorage,
}

/// Where a profile's passwords are kept.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasswordStorage {
    /// In the profile's file, as plain text.
    #[default]
    File,
    /// In the OS keychain; the file has none.
    Keychain,
}

impl PasswordStorage {
    pub fn is_file(&self) -> bool {
        *self == Self::File
    }
}

impl std::str::FromStr for PasswordStorage {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "file" => Ok(Self::File),
            "keychain" => Ok(Self::Keychain),
            _ => Err(format!("unknown password storage '{s}' (keychain, file)")),
        }
    }
}

/// What a listing shows: no password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileSummary {
    pub name: String,
    pub driver: Driver,
    /// `user@host:port/database`, without the password.
    pub target: String,
}

impl Profile {
    pub fn summary(&self) -> ProfileSummary {
        ProfileSummary {
            name: self.name.clone(),
            driver: self.driver,
            target: self.target(),
        }
    }

    /// Where the database server is, as the profile says (the URL's host and port,
    /// or the fields), before any SSH tunnel.
    pub fn server(&self) -> (String, u16) {
        if let Some(u) = self.url.as_deref().and_then(|u| url::Url::parse(u).ok()) {
            return (
                u.host_str()
                    .unwrap_or("localhost")
                    .trim_matches(['[', ']'])
                    .to_owned(),
                u.port().unwrap_or(self.driver.default_port()),
            );
        }
        let host = if self.host.is_empty() {
            "localhost"
        } else {
            &self.host
        };
        (
            host.to_owned(),
            self.port.unwrap_or(self.driver.default_port()),
        )
    }

    /// `user@host:port/database` (the URL's, if there is one), never with a password;
    /// for SQLite, the file.
    pub fn target(&self) -> String {
        if self.driver.is_file() {
            return match &self.url {
                Some(u) => u.clone(),
                None => self.database.clone(),
            };
        }
        if let Some(url) = self.url.as_deref().and_then(|u| url::Url::parse(u).ok()) {
            let user = if url.username().is_empty() {
                self.user.clone()
            } else {
                decode(url.username())
            };
            let host = url.host_str().unwrap_or("localhost");
            let port = url.port().unwrap_or(self.driver.default_port());
            let db = url.path().trim_start_matches('/');
            return self.via_ssh(join_target(&user, host, port, &decode(db)));
        }
        let host = if self.host.is_empty() {
            "localhost"
        } else {
            &self.host
        };
        let port = self.port.unwrap_or(self.driver.default_port());
        self.via_ssh(join_target(&self.user, host, port, &self.database))
    }

    fn via_ssh(&self, target: String) -> String {
        match &self.ssh {
            Some(ssh) => format!("{target} via {}@{}", ssh.user, ssh.host),
            None => target,
        }
    }

    /// Whether the profile has any password or passphrase.
    pub fn has_secrets(&self) -> bool {
        let mut p = self.clone();
        let (db, ssh) = p.take_secrets();
        db.is_some() || ssh.is_some()
    }

    /// Remove the passwords (the database's, including one in the URL, and the SSH
    /// password or key passphrase), returning them.
    fn take_secrets(&mut self) -> (Option<String>, Option<String>) {
        let mut db = self.password.take().filter(|p| !p.is_empty());
        if let Some(u) = &self.url
            && let Ok(mut url) = url::Url::parse(u)
            && let Some(pw) = url.password()
        {
            // As when connecting, the URL's password wins.
            db = Some(decode(pw));
            let _ = url.set_password(None);
            self.url = Some(url.to_string());
        }
        let ssh = self.ssh.as_mut().and_then(|ssh| match &mut ssh.auth {
            SshAuth::Password { password } => Some(std::mem::take(password)),
            SshAuth::Key { passphrase, .. } => passphrase.take(),
            SshAuth::Agent => None,
        });
        (db, ssh.filter(|s| !s.is_empty()))
    }

    /// The same profile with the password (in the field and in any URL) replaced by
    /// `********`, for display.
    pub fn masked(&self) -> Profile {
        let mut p = self.clone();
        if p.password.as_deref().is_some_and(|pw| !pw.is_empty()) {
            p.password = Some(MASK.into());
        }
        if let Some(ssh) = &mut p.ssh {
            match &mut ssh.auth {
                SshAuth::Password { password } => *password = MASK.into(),
                SshAuth::Key {
                    passphrase: Some(pp),
                    ..
                } => *pp = MASK.into(),
                _ => {}
            }
        }
        p.url = p.url.as_deref().map(mask_url_password);
        p
    }

    /// Check the profile is usable before saving it.
    pub fn validate(&self) -> Result<()> {
        validate_name(&self.name)?;
        if self.driver.is_file() && self.url.is_none() && self.database.trim().is_empty() {
            return Err(CoreError::InvalidProfile(
                "choose the SQLite database file".into(),
            ));
        }
        if let Some(ssh) = &self.ssh {
            if self.driver.is_file() {
                return Err(CoreError::InvalidProfile(
                    "SQLite opens a local file, so it can't go through SSH".into(),
                ));
            }
            if ssh.host.trim().is_empty() || ssh.user.trim().is_empty() {
                return Err(CoreError::InvalidProfile(
                    "the SSH tunnel needs a host and a user".into(),
                ));
            }
            if let SshAuth::Key { path, .. } = &ssh.auth
                && path.as_os_str().is_empty()
            {
                return Err(CoreError::InvalidProfile(
                    "choose the SSH private key file".into(),
                ));
            }
        }
        if let Some(url) = &self.url {
            let driver = Driver::from_url(url).ok_or_else(|| {
                CoreError::InvalidProfile(
                    "the connection URL must start with postgres://, postgresql://, mysql:// \
                     or sqlite:"
                        .into(),
                )
            })?;
            if driver != self.driver {
                return Err(CoreError::InvalidProfile(format!(
                    "the URL is for {} but the profile's driver is {}",
                    driver.label(),
                    self.driver.label()
                )));
            }
            if !driver.is_file() {
                url::Url::parse(url).map_err(|e| {
                    CoreError::InvalidProfile(format!("the connection URL isn't valid: {e}"))
                })?;
            }
        }
        Ok(())
    }
}

pub const MASK: &str = "********";

fn join_target(user: &str, host: &str, port: u16, db: &str) -> String {
    let mut s = String::new();
    if !user.is_empty() {
        s.push_str(user);
        s.push('@');
    }
    if host.contains(':') {
        s.push_str(&format!("[{host}]:{port}"));
    } else {
        s.push_str(&format!("{host}:{port}"));
    }
    if !db.is_empty() {
        s.push('/');
        s.push_str(db);
    }
    s
}

fn decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={s}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| s.to_owned())
}

fn mask_url_password(raw: &str) -> String {
    match url::Url::parse(raw) {
        Ok(mut u) if u.password().is_some() => {
            let _ = u.set_password(Some(MASK));
            u.to_string()
        }
        _ => raw.to_owned(),
    }
}

/// Names become file names, so they're kept simple: letters, digits, spaces and
/// `-_.`, not starting with a dot, at most 64 characters.
pub fn validate_name(name: &str) -> Result<()> {
    let bad = |why: &str| Err(CoreError::InvalidProfile(why.to_owned()));
    if name.trim().is_empty() {
        return bad("the profile needs a name");
    }
    if name != name.trim() {
        return bad("the profile name can't start or end with a space");
    }
    if name.chars().count() > 64 {
        return bad("the profile name can be at most 64 characters");
    }
    if name.starts_with('.') {
        return bad("the profile name can't start with a dot");
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
    {
        return bad("the profile name can only use letters, digits, spaces and - _ .");
    }
    Ok(())
}

/// The folder profiles are saved in.
#[derive(Debug, Clone)]
pub struct ProfileStore {
    dir: PathBuf,
    keychain: Keychain,
}

/// The keychain account names for a profile's secrets.
fn db_account(name: &str) -> String {
    name.to_owned()
}

fn ssh_account(name: &str) -> String {
    format!("{name} (SSH)")
}

impl ProfileStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self::with_keychain(dir, Keychain::system())
    }

    pub fn with_keychain(dir: impl Into<PathBuf>, keychain: Keychain) -> Self {
        Self {
            dir: dir.into(),
            keychain,
        }
    }

    pub fn keychain(&self) -> &Keychain {
        &self.keychain
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.json"))
    }

    pub fn list(&self) -> Result<Vec<Profile>> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(CoreError::io(format!("reading {}", self.dir.display()), e)),
        };
        let mut out = vec![];
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            match read_profile(&path) {
                Ok(p) => out.push(p),
                Err(e) => tracing::warn!("skipping {}: {e}", path.display()),
            }
        }
        out.sort_by_key(|p| p.name.to_lowercase());
        Ok(out)
    }

    /// A profile, with its passwords (from the keychain, if they're kept there).
    pub fn get(&self, name: &str) -> Result<Profile> {
        let mut p = self.get_file(name)?;
        if p.passwords == PasswordStorage::Keychain {
            p.password = self.keychain.get(&db_account(name))?;
            if let Some(ssh) = &mut p.ssh {
                let secret = || self.keychain.get(&ssh_account(name));
                match &mut ssh.auth {
                    SshAuth::Password { password } => *password = secret()?.unwrap_or_default(),
                    SshAuth::Key { passphrase, .. } => *passphrase = secret()?,
                    SshAuth::Agent => {}
                }
            }
        }
        Ok(p)
    }

    /// A profile as its file has it (without passwords kept in the keychain).
    pub fn get_without_secrets(&self, name: &str) -> Result<Profile> {
        self.get_file(name)
    }

    fn get_file(&self, name: &str) -> Result<Profile> {
        validate_name(name).map_err(|_| CoreError::ProfileNotFound(name.to_owned()))?;
        let path = self.path(name);
        if !path.exists() {
            return Err(CoreError::ProfileNotFound(name.to_owned()));
        }
        read_profile(&path)
    }

    pub fn exists(&self, name: &str) -> bool {
        validate_name(name).is_ok() && self.path(name).exists()
    }

    /// Save `profile`. With `previous_name` set this is an edit of that profile (which
    /// may be a rename); otherwise it's new, and an existing profile with the same name
    /// is only replaced when `overwrite` is set.
    pub fn save(
        &self,
        profile: &Profile,
        previous_name: Option<&str>,
        overwrite: bool,
    ) -> Result<()> {
        profile.validate()?;
        let renaming = previous_name.is_some_and(|old| old != profile.name);
        let is_edit_in_place = previous_name == Some(profile.name.as_str());
        if !is_edit_in_place && !overwrite && self.exists(&profile.name) {
            return Err(CoreError::ProfileExists(profile.name.clone()));
        }
        if let Some(old) = previous_name
            && !self.exists(old)
        {
            return Err(CoreError::ProfileNotFound(old.to_owned()));
        }
        let before = previous_name
            .or(Some(profile.name.as_str()))
            .and_then(|n| self.get_file(n).ok());
        let mut file = profile.clone();
        if profile.passwords == PasswordStorage::Keychain {
            // Into the keychain first: if that fails, nothing has changed.
            let (db, ssh) = file.take_secrets();
            self.keychain
                .set(&db_account(&profile.name), db.as_deref())?;
            self.keychain
                .set(&ssh_account(&profile.name), ssh.as_deref())?;
        }
        fs::create_dir_all(&self.dir)
            .map_err(|e| CoreError::io(format!("creating {}", self.dir.display()), e))?;
        restrict_dir(&self.dir);
        let json = serde_json::to_vec_pretty(&file).expect("profile serialises");
        write_private(&self.path(&profile.name), &json)?;
        if renaming && let Some(old) = previous_name {
            let _ = fs::remove_file(self.path(old));
        }
        // Tidy up the keychain entries a rename or a move to the file left behind.
        if let Some(before) = before
            && before.passwords == PasswordStorage::Keychain
            && (renaming || profile.passwords == PasswordStorage::File)
        {
            self.forget_secrets(&before.name);
        }
        Ok(())
    }

    fn forget_secrets(&self, name: &str) {
        for account in [db_account(name), ssh_account(name)] {
            if let Err(e) = self.keychain.delete(&account) {
                tracing::warn!("{e}");
            }
        }
    }

    pub fn remove(&self, name: &str) -> Result<()> {
        if !self.exists(name) {
            return Err(CoreError::ProfileNotFound(name.to_owned()));
        }
        let keychain = self
            .get_file(name)
            .is_ok_and(|p| p.passwords == PasswordStorage::Keychain);
        let path = self.path(name);
        fs::remove_file(&path)
            .map_err(|e| CoreError::io(format!("removing {}", path.display()), e))?;
        if keychain {
            self.forget_secrets(name);
        }
        Ok(())
    }
}

fn read_profile(path: &Path) -> Result<Profile> {
    let bytes =
        fs::read(path).map_err(|e| CoreError::io(format!("reading {}", path.display()), e))?;
    let mut p: Profile = serde_json::from_slice(&bytes)
        .map_err(|e| CoreError::InvalidProfile(format!("{}: {e}", path.display())))?;
    // The file name is the source of truth for the name.
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        p.name = stem.to_owned();
    }
    Ok(p)
}

/// Write via a temporary file and rename, so a crash never leaves half a profile.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    let ctx = |e| CoreError::io(format!("writing {}", path.display()), e);
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).map_err(ctx)?;
        f.write_all(bytes).map_err(ctx)?;
        f.sync_all().map_err(ctx)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600)).map_err(ctx)?;
    }
    fs::rename(&tmp, path).map_err(ctx)
}

fn restrict_dir(_dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(_dir, fs::Permissions::from_mode(0o700));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pg(name: &str) -> Profile {
        Profile {
            name: name.into(),
            host: "db.example.com".into(),
            database: "shop".into(),
            user: "alice".into(),
            password: Some("s3cret".into()),
            ..Profile::default()
        }
    }

    #[test]
    fn save_list_get_remove() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(dir.path().join("profiles"));
        assert!(store.list().unwrap().is_empty());
        store.save(&pg("shop"), None, false).unwrap();
        store.save(&pg("Analytics"), None, false).unwrap();
        let names: Vec<_> = store.list().unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Analytics", "shop"]);
        assert_eq!(store.get("shop").unwrap(), pg("shop"));
        assert!(matches!(
            store.save(&pg("shop"), None, false),
            Err(CoreError::ProfileExists(_))
        ));
        store.save(&pg("shop"), None, true).unwrap();
        store.remove("shop").unwrap();
        assert!(matches!(
            store.get("shop"),
            Err(CoreError::ProfileNotFound(_))
        ));
    }

    #[test]
    fn edit_and_rename() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(dir.path());
        store.save(&pg("a"), None, false).unwrap();
        store.save(&pg("b"), None, false).unwrap();
        // Editing in place needs no overwrite flag.
        let mut a = pg("a");
        a.database = "other".into();
        store.save(&a, Some("a"), false).unwrap();
        assert_eq!(store.get("a").unwrap().database, "other");
        // Renaming onto an existing profile is refused.
        assert!(matches!(
            store.save(&pg("b"), Some("a"), false),
            Err(CoreError::ProfileExists(_))
        ));
        store.save(&pg("c"), Some("a"), false).unwrap();
        assert!(!store.exists("a"));
        assert!(store.exists("c"));
    }

    #[test]
    fn passwords_in_the_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let keychain = Keychain::memory();
        let store = ProfileStore::with_keychain(dir.path(), keychain.clone());
        let mut p = pg("shop");
        p.url = Some("postgres://alice:url%20pw@db/shop".into());
        p.passwords = PasswordStorage::Keychain;
        p.ssh = Some(SshTunnel {
            host: "bastion".into(),
            user: "ops".into(),
            auth: SshAuth::Key {
                path: "/k".into(),
                passphrase: Some("pp".into()),
            },
            ..SshTunnel::default()
        });
        store.save(&p, None, false).unwrap();

        // None of them are in the file...
        let file = fs::read_to_string(dir.path().join("shop.json")).unwrap();
        for secret in ["s3cret", "url%20pw", "\"pp\""] {
            assert!(!file.contains(secret), "{secret} is in the file: {file}");
        }
        assert!(file.contains("\"passwords\": \"keychain\""), "{file}");
        // ...they're in the keychain, and come back with the profile.
        assert_eq!(keychain.get("shop").unwrap().as_deref(), Some("url pw"));
        let got = store.get("shop").unwrap();
        assert_eq!(got.password.as_deref(), Some("url pw"));
        assert_eq!(got.url.as_deref(), Some("postgres://alice@db/shop"));
        let Some(SshTunnel {
            auth: SshAuth::Key { passphrase, .. },
            ..
        }) = &got.ssh
        else {
            panic!()
        };
        assert_eq!(passphrase.as_deref(), Some("pp"));

        // Renamed: the keychain entries move with it.
        store.save(&got, Some("shop"), false).unwrap();
        let mut renamed = got.clone();
        renamed.name = "store".into();
        store.save(&renamed, Some("shop"), false).unwrap();
        assert_eq!(keychain.get("shop").unwrap(), None);
        assert_eq!(keychain.get("store").unwrap().as_deref(), Some("url pw"));
        assert_eq!(keychain.get("store (SSH)").unwrap().as_deref(), Some("pp"));

        // Moved back to the file: the keychain is tidied up.
        let mut back = store.get("store").unwrap();
        back.passwords = PasswordStorage::File;
        store.save(&back, Some("store"), false).unwrap();
        assert_eq!(keychain.get("store").unwrap(), None);
        assert_eq!(
            store.get("store").unwrap().password.as_deref(),
            Some("url pw")
        );

        // Removing a keychain profile removes its entries.
        back.passwords = PasswordStorage::Keychain;
        store.save(&back, Some("store"), false).unwrap();
        assert!(keychain.get("store").unwrap().is_some());
        store.remove("store").unwrap();
        assert_eq!(keychain.get("store").unwrap(), None);
        assert_eq!(keychain.get("store (SSH)").unwrap(), None);
    }

    #[cfg(unix)]
    #[test]
    fn files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(dir.path().join("profiles"));
        store.save(&pg("shop"), None, false).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.path().join("profiles/shop.json")), 0o600);
        assert_eq!(mode(&dir.path().join("profiles")), 0o700);
    }

    #[test]
    fn names_are_checked() {
        for bad in ["", " x", "x ", "../x", "a/b", "a\\b", ".hidden", "a:b"] {
            assert!(validate_name(bad).is_err(), "{bad:?} should be refused");
        }
        for good in ["shop", "Prod DB", "eu-west_1.replica", "Café"] {
            validate_name(good).unwrap();
        }
    }

    #[test]
    fn target_and_masking() {
        assert_eq!(pg("x").target(), "alice@db.example.com:5432/shop");
        let p = Profile {
            name: "u".into(),
            driver: Driver::Mysql,
            url: Some("mysql://bob:hunter2@10.0.0.5/app%20db".into()),
            ..Profile::default()
        };
        assert_eq!(p.target(), "bob@10.0.0.5:3306/app db");
        let m = p.masked();
        assert!(!m.url.as_deref().unwrap().contains("hunter2"));
        assert!(pg("x").masked().password.as_deref() == Some(MASK));
        assert_eq!(Profile::default().target(), "localhost:5432");
    }

    #[test]
    fn ssh_tunnels() {
        let mut p = pg("x");
        p.ssh = Some(SshTunnel {
            host: "bastion.example.com".into(),
            port: None,
            user: "ops".into(),
            auth: SshAuth::Password {
                password: "pw".into(),
            },
        });
        p.validate().unwrap();
        assert_eq!(
            p.target(),
            "alice@db.example.com:5432/shop via ops@bastion.example.com"
        );
        assert_eq!(p.server(), ("db.example.com".into(), 5432));
        let m = p.masked();
        assert_eq!(
            m.ssh.unwrap().auth,
            SshAuth::Password {
                password: MASK.into()
            }
        );
        // Saved and read back as it was.
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"method\":\"password\""), "{json}");
        assert_eq!(serde_json::from_str::<Profile>(&json).unwrap(), p);
        p.driver = Driver::Sqlite;
        p.database = "x.db".into();
        assert!(p.validate().is_err());
    }

    #[test]
    fn url_must_match_driver() {
        let mut p = pg("x");
        p.url = Some("mysql://h/db".into());
        assert!(p.validate().is_err());
        p.driver = Driver::Mysql;
        p.validate().unwrap();
        p.url = Some("redis://h".into());
        assert!(p.validate().is_err());
    }
}
