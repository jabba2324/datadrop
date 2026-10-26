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
        let mut st = s.serialize_struct("CoreError", 2)?;
        st.serialize_field("kind", self.kind())?;
        st.serialize_field("message", &self.to_string())?;
        st.end()
    }
}
