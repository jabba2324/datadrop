use serde::{Serialize, Serializer};

pub type Result<T, E = CoreError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("no connection profile called '{0}'")]
    ProfileNotFound(String),
    #[error("a connection profile called '{0}' already exists")]
    ProfileExists(String),
    #[error("{0}")]
    InvalidProfile(String),
    #[error("{0}")]
    Database(String),
    #[error("the query was cancelled")]
    Cancelled,
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{0}")]
    Invalid(String),
    #[error("SSH: {0}")]
    Ssh(String),
    /// The SSH server's host key isn't known yet: check the fingerprint, then trust
    /// it (`SshTrustHost`) to connect.
    #[error(
        "the SSH server {host}:{port} isn't known yet. Its key fingerprint is {fingerprint}; \
         check that with the server's admin, then trust it"
    )]
    UnknownHostKey {
        host: String,
        port: u16,
        fingerprint: String,
        /// The key, in OpenSSH format (for `SshTrustHost`).
        key: String,
    },
    #[error(
        "the SSH server {host}:{port} has a different key from the one recorded in {file} \
         (now {fingerprint}). That can mean someone is intercepting the connection, so \
         DataDrop won't connect. If the server's key really changed, remove the old one"
    )]
    HostKeyChanged {
        host: String,
        port: u16,
        fingerprint: String,
        file: String,
    },
    #[error("keychain: {0}")]
    Keychain(String),
}

impl CoreError {
    pub fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    /// A short machine-readable name, used in JSON output.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ProfileNotFound(_) => "profile_not_found",
            Self::ProfileExists(_) => "profile_exists",
            Self::InvalidProfile(_) => "invalid_profile",
            Self::Database(_) => "database",
            Self::Cancelled => "cancelled",
            Self::Io { .. } => "io",
            Self::Invalid(_) => "invalid",
            Self::Ssh(_) => "ssh",
            Self::UnknownHostKey { .. } => "unknown_host_key",
            Self::HostKeyChanged { .. } => "host_key_changed",
            Self::Keychain(_) => "keychain",
        }
    }
}

impl From<sqlx::Error> for CoreError {
    fn from(e: sqlx::Error) -> Self {
        Self::Database(describe_db_error(&e))
    }
}

/// The database's own message (with its SQLSTATE code) when there is one; sqlx's
/// wrapper text otherwise.
pub fn describe_db_error(e: &sqlx::Error) -> String {
    match e {
        sqlx::Error::Database(db) => match db.code() {
            Some(code) => format!("{} ({code})", db.message()),
            None => db.message().to_owned(),
        },
        sqlx::Error::Io(io) => format!("couldn't reach the server: {io}"),
        sqlx::Error::Tls(tls) => format!("TLS: {tls}"),
        sqlx::Error::PoolTimedOut => "timed out connecting to the server".into(),
        other => other.to_string(),
    }
}

/// Serialised as `{"kind": "...", "message": "..."}`.
impl Serialize for CoreError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("CoreError", 6)?;
        st.serialize_field("kind", self.kind())?;
        st.serialize_field("message", &self.to_string())?;
        if let Self::UnknownHostKey {
            host,
            port,
            fingerprint,
            key,
        } = self
        {
            st.serialize_field("host", host)?;
            st.serialize_field("port", port)?;
            st.serialize_field("fingerprint", fingerprint)?;
            st.serialize_field("key", key)?;
        }
        st.end()
    }
}
