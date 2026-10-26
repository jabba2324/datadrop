//! SSH tunnels: connect to an SSH server and forward a local port to the database
//! behind it, like `ssh -L`.
//!
//! The server's host key must be known: in `~/.ssh/known_hosts`, or trusted in
//! DataDrop (`<config>/known_hosts`). An unknown key is an error carrying its
//! fingerprint, so the user can check and trust it; a changed key is refused.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, AuthResult};
use russh::keys::agent::client::AgentClient;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;

use crate::error::{CoreError, Result};
use crate::profile::{SshAuth, SshTunnel};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Where to look for known host keys: the user's own, then DataDrop's.
pub fn known_hosts_files(datadrop_known_hosts: PathBuf) -> Vec<PathBuf> {
    let mut files = vec![];
    if let Some(home) = std::env::home_dir() {
        files.push(home.join(".ssh").join("known_hosts"));
    }
    files.push(datadrop_known_hosts);
    files
}

/// Trust a host key: add it to DataDrop's known hosts.
pub fn trust_host(file: &std::path::Path, host: &str, port: u16, key_openssh: &str) -> Result<()> {
    let key = PublicKey::from_openssh(key_openssh)
        .map_err(|e| CoreError::Invalid(format!("that isn't an SSH public key: {e}")))?;
    russh::keys::known_hosts::learn_known_hosts_path(host, port, &key, file)
        .map_err(|e| CoreError::Ssh(format!("couldn't record the host key: {e}")))
}

pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

struct HostCheck {
    host: String,
    port: u16,
    known_hosts: Vec<PathBuf>,
    /// Why the key was refused, for the error the caller sees.
    refused: Arc<Mutex<Option<CoreError>>>,
}

impl client::Handler for HostCheck {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let refuse = |e: CoreError| {
            *self.refused.lock().expect("refused") = Some(e);
            Ok(false)
        };
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            return refuse(CoreError::Ssh(
                "the SSH server presented a certificate; only plain host keys are supported".into(),
            ));
        };
        for file in &self.known_hosts {
            match russh::keys::check_known_hosts_path(&self.host, self.port, key, file) {
                Ok(true) => return Ok(true),
                Ok(false) => {}
                Err(russh::keys::Error::KeyChanged { line }) => {
                    return refuse(CoreError::HostKeyChanged {
                        host: self.host.clone(),
                        port: self.port,
                        fingerprint: fingerprint(key),
                        file: format!("{}, line {line}", file.display()),
                    });
                }
                // No such file, or a line it can't read: look in the next one.
                Err(_) => {}
            }
        }
        refuse(CoreError::UnknownHostKey {
            host: self.host.clone(),
            port: self.port,
            fingerprint: fingerprint(key),
            key: key.to_openssh().unwrap_or_default(),
        })
    }
}

/// An open tunnel. Dropping it stops accepting connections and closes the SSH
/// connection once the forwarded ones finish.
pub struct Tunnel {
    pub local: (String, u16),
    handle: Arc<client::Handle<HostCheck>>,
    accept: tokio::task::JoinHandle<()>,
    /// Why the SSH server last refused to forward a connection, if it did.
    refused_forward: Arc<Mutex<Option<String>>>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

impl Tunnel {
    pub fn is_alive(&self) -> bool {
        !self.handle.is_closed() && !self.accept.is_finished()
    }

    /// The SSH server's reason for refusing the last forwarded connection (taken,
    /// so it's reported once).
    pub fn take_forward_error(&self) -> Option<String> {
        self.refused_forward.lock().expect("refused_forward").take()
    }

    /// Connect to the SSH server and forward a new local port to `target` (the
    /// database server, as the SSH server sees it).
    pub async fn open(
        ssh: &SshTunnel,
        target: (String, u16),
        known_hosts: Vec<PathBuf>,
    ) -> Result<Tunnel> {
        let refused = Arc::new(Mutex::new(None));
        let check = HostCheck {
            host: ssh.host.clone(),
            port: ssh.port(),
            known_hosts,
            refused: refused.clone(),
        };
        let config = Arc::new(client::Config {
            keepalive_interval: Some(Duration::from_secs(30)),
            ..client::Config::default()
        });
        let connect = client::connect(config, (ssh.host.as_str(), ssh.port()), check);
        let mut handle = match tokio::time::timeout(CONNECT_TIMEOUT, connect).await {
            Err(_) => {
                return Err(CoreError::Ssh(format!(
                    "timed out connecting to the SSH server {}",
                    ssh.host
                )));
            }
            Ok(Err(e)) => {
                if let Some(why) = refused.lock().expect("refused").take() {
                    return Err(why);
                }
                return Err(CoreError::Ssh(format!(
                    "couldn't connect to the SSH server {}: {e}",
                    ssh.host
                )));
            }
            Ok(Ok(h)) => h,
        };

        let user = ssh.user.clone();
        let ok = match &ssh.auth {
            SshAuth::Password { password } => {
                success(handle.authenticate_password(user, password.clone()).await)?
            }
            SshAuth::Key { path, passphrase } => {
                let key =
                    russh::keys::load_secret_key(path, passphrase.as_deref()).map_err(|e| {
                        CoreError::Ssh(format!(
                            "couldn't read the key {} (is the passphrase right?): {e}",
                            path.display()
                        ))
                    })?;
                let hash = handle
                    .best_supported_rsa_hash()
                    .await
                    .ok()
                    .flatten()
                    .flatten();
                let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
                success(handle.authenticate_publickey(user, key).await)?
            }
            SshAuth::Agent => agent_auth(&mut handle, &user).await?,
        };
        if !ok {
            let how = match &ssh.auth {
                SshAuth::Password { .. } => "the password",
                SshAuth::Key { .. } => "the key",
                SshAuth::Agent => "any key in your SSH agent",
            };
            return Err(CoreError::Ssh(format!(
                "the SSH server {} didn't accept {how} for {}",
                ssh.host, ssh.user
            )));
        }

        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| CoreError::io("opening the tunnel's local port", e))?;
        let port = listener
            .local_addr()
            .map_err(|e| CoreError::io("opening the tunnel's local port", e))?
            .port();
        let handle = Arc::new(handle);
        let forward = handle.clone();
        let refused_forward = Arc::new(Mutex::new(None));
        let refused = refused_forward.clone();
        let accept = tokio::spawn(async move {
            while let Ok((mut socket, peer)) = listener.accept().await {
                let h = forward.clone();
                let (host, port) = target.clone();
                let refused = refused.clone();
                tokio::spawn(async move {
                    let channel = h
                        .channel_open_direct_tcpip(
                            host.clone(),
                            u32::from(port),
                            peer.ip().to_string(),
                            u32::from(peer.port()),
                        )
                        .await;
                    match channel {
                        Ok(ch) => {
                            let mut stream = ch.into_stream();
                            let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
                        }
                        Err(e) => {
                            tracing::warn!("SSH tunnel: couldn't reach the database: {e}");
                            *refused.lock().expect("refused_forward") = Some(format!(
                                "the SSH server wouldn't forward the connection to {host}:{port} \
                                 ({e}). Check that the database address is right as the SSH \
                                 server sees it, and that the server allows TCP forwarding"
                            ));
                        }
                    }
                });
            }
        });
        Ok(Tunnel {
            local: ("127.0.0.1".into(), port),
            handle,
            accept,
            refused_forward,
        })
    }
}

fn success(r: std::result::Result<AuthResult, russh::Error>) -> Result<bool> {
    match r {
        Ok(AuthResult::Success) => Ok(true),
        Ok(AuthResult::Failure { .. }) => Ok(false),
        Err(e) => Err(CoreError::Ssh(format!(
            "signing in to the SSH server failed: {e}"
        ))),
    }
}

/// Try each key the SSH agent holds.
async fn agent_auth(handle: &mut client::Handle<HostCheck>, user: &str) -> Result<bool> {
    #[cfg(unix)]
    {
        let agent = AgentClient::connect_env().await.map_err(|e| {
            CoreError::Ssh(format!(
                "couldn't reach your SSH agent (is SSH_AUTH_SOCK set?): {e}"
            ))
        })?;
        try_agent_keys(handle, user, agent).await
    }
    #[cfg(windows)]
    {
        match AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
            Ok(agent) => try_agent_keys(handle, user, agent).await,
            Err(_) => {
                let agent = AgentClient::connect_pageant().await.map_err(|e| {
                    CoreError::Ssh(format!(
                        "couldn't reach an SSH agent (the OpenSSH agent service or Pageant): {e}"
                    ))
                })?;
                try_agent_keys(handle, user, agent).await
            }
        }
    }
}

async fn try_agent_keys<S>(
    handle: &mut client::Handle<HostCheck>,
    user: &str,
    mut agent: AgentClient<S>,
) -> Result<bool>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let identities = agent
        .request_identities()
        .await
        .map_err(|e| CoreError::Ssh(format!("couldn't list the keys in your SSH agent: {e}")))?;
    if identities.is_empty() {
        return Err(CoreError::Ssh("your SSH agent has no keys loaded".into()));
    }
    let hash = handle
        .best_supported_rsa_hash()
        .await
        .ok()
        .flatten()
        .flatten();
    for id in identities {
        let key = match id {
            russh::keys::agent::AgentIdentity::PublicKey { key, .. } => key,
            _ => continue,
        };
        let r = handle
            .authenticate_publickey_with(user, key, hash, &mut agent)
            .await;
        if let Ok(AuthResult::Success) = r {
            return Ok(true);
        }
    }
    Ok(false)
}
