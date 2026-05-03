use std::io::Read;
use std::net::TcpStream;
use std::path::{Path, PathBuf};

use ssh2::Session;

#[derive(Debug, Clone)]
pub(crate) enum SshAuthMethod {
    KeyFile { path: PathBuf },
    Agent,
    Password { password: String },
}

#[derive(Debug, Clone)]
pub(crate) struct SshServerConfig {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: SshAuthMethod,
    pub remote_path: String,
}

impl Default for SshServerConfig {
    fn default() -> Self {
        Self {
            name: "New Server".into(),
            host: String::new(),
            port: 22,
            username: String::new(),
            auth_method: SshAuthMethod::Agent,
            remote_path: "~".into(),
        }
    }
}

pub(crate) struct SshConnection {
    session: Session,
    pub config: SshServerConfig,
}

#[derive(Debug, Clone)]
pub(crate) struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

impl SshConnection {
    pub fn connect(config: &SshServerConfig) -> Result<Self, String> {
        let addr = format!("{}:{}", config.host, config.port);
        let tcp =
            TcpStream::connect(&addr).map_err(|e| format!("TCP connect to {}: {}", addr, e))?;
        tcp.set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .map_err(|e| format!("set read timeout: {}", e))?;

        let mut session = Session::new().map_err(|e| format!("create SSH session: {}", e))?;
        session.set_tcp_stream(tcp);
        session
            .handshake()
            .map_err(|e| format!("SSH handshake with {}: {}", addr, e))?;

        match &config.auth_method {
            SshAuthMethod::Agent => {
                session
                    .userauth_agent(&config.username)
                    .map_err(|e| format!("SSH agent auth as '{}': {}", config.username, e))?;
            }
            SshAuthMethod::KeyFile { path } => {
                session
                    .userauth_pubkey_file(&config.username, None, path, None)
                    .map_err(|e| {
                        format!(
                            "SSH key auth as '{}' with {}: {}",
                            config.username,
                            path.display(),
                            e
                        )
                    })?;
            }
            SshAuthMethod::Password { password } => {
                session
                    .userauth_password(&config.username, password)
                    .map_err(|e| format!("SSH password auth as '{}': {}", config.username, e))?;
            }
        }

        if !session.authenticated() {
            return Err("SSH authentication failed".into());
        }

        Ok(SshConnection {
            session,
            config: config.clone(),
        })
    }

    pub fn list_dir(&self, remote_path: &str) -> Result<Vec<RemoteEntry>, String> {
        let sftp = self
            .session
            .sftp()
            .map_err(|e| format!("open SFTP channel: {}", e))?;
        let resolved = self.resolve_path(remote_path)?;
        let dir = sftp
            .readdir(Path::new(&resolved))
            .map_err(|e| format!("list '{}': {}", resolved, e))?;

        let mut entries: Vec<RemoteEntry> = dir
            .into_iter()
            .filter_map(|(path, stat)| {
                let name = path.file_name()?.to_string_lossy().to_string();
                if name.starts_with('.') {
                    return None;
                }
                let full_path = format!("{}/{}", resolved.trim_end_matches('/'), name);
                Some(RemoteEntry {
                    name,
                    path: full_path,
                    is_dir: stat.is_dir(),
                    size: stat.size.unwrap_or(0),
                })
            })
            .collect();

        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));

        Ok(entries)
    }

    pub fn read_file(&self, remote_path: &str) -> Result<String, String> {
        let sftp = self
            .session
            .sftp()
            .map_err(|e| format!("open SFTP channel: {}", e))?;
        let resolved = self.resolve_path(remote_path)?;
        let mut file = sftp
            .open(Path::new(&resolved))
            .map_err(|e| format!("open '{}': {}", resolved, e))?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .map_err(|e| format!("read '{}': {}", resolved, e))?;
        Ok(contents)
    }

    pub fn home_dir(&self) -> Result<String, String> {
        let mut channel = self
            .session
            .channel_session()
            .map_err(|e| format!("open channel: {}", e))?;
        channel
            .exec("echo $HOME")
            .map_err(|e| format!("exec echo $HOME: {}", e))?;
        let mut output = String::new();
        channel
            .read_to_string(&mut output)
            .map_err(|e| format!("read home dir: {}", e))?;
        channel.wait_close().ok();
        Ok(output.trim().to_string())
    }

    fn resolve_path(&self, path: &str) -> Result<String, String> {
        if path.starts_with('~') {
            let home = self.home_dir()?;
            Ok(path.replacen('~', &home, 1))
        } else {
            Ok(path.to_string())
        }
    }

    pub fn disconnect(self) {
        let _ = self.session.disconnect(None, "bye", None);
    }
}
