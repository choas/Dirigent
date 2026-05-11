use std::io::Read;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};

use ssh2::{CheckResult, KnownHostFileKind, Session};

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
}

#[derive(Debug, Clone)]
pub(crate) struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

fn verify_host_key(session: &Session, host: &str, port: u16) -> Result<(), String> {
    let (key, _key_type) = session
        .host_key()
        .ok_or("SSH server did not present a host key")?;

    let mut known_hosts = session
        .known_hosts()
        .map_err(|e| format!("init known hosts: {}", e))?;

    let known_hosts_path = dirs::home_dir()
        .ok_or("cannot determine home directory")?
        .join(".ssh")
        .join("known_hosts");

    if known_hosts_path.exists() {
        known_hosts
            .read_file(&known_hosts_path, KnownHostFileKind::OpenSSH)
            .map_err(|e| format!("read {}: {}", known_hosts_path.display(), e))?;
    }

    match known_hosts.check_port(host, port, key) {
        CheckResult::Match => Ok(()),
        CheckResult::NotFound => Err(format!(
            "host key for '{}' not found in known_hosts — connect with ssh first to trust the key",
            host
        )),
        CheckResult::Mismatch => Err(format!(
            "HOST KEY MISMATCH for '{}' — the server's key does not match known_hosts (possible MITM attack)",
            host
        )),
        CheckResult::Failure => Err(format!(
            "failed to verify host key for '{}'",
            host
        )),
    }
}

impl SshConnection {
    pub fn connect(config: &SshServerConfig) -> Result<Self, String> {
        let sock_addr: std::net::SocketAddr = {
            use std::net::ToSocketAddrs;
            (config.host.as_str(), config.port)
                .to_socket_addrs()
                .map_err(|e| format!("resolve {}:{}: {}", config.host, config.port, e))?
                .next()
                .ok_or_else(|| format!("no addresses for {}:{}", config.host, config.port))?
        };
        let addr = format!("{}:{}", config.host, config.port);
        let tcp = TcpStream::connect_timeout(&sock_addr, std::time::Duration::from_secs(10))
            .map_err(|e| format!("TCP connect to {}: {}", addr, e))?;
        tcp.set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .map_err(|e| format!("set read timeout: {}", e))?;
        tcp.set_write_timeout(Some(std::time::Duration::from_secs(10)))
            .map_err(|e| format!("set write timeout: {}", e))?;

        let mut session = Session::new().map_err(|e| format!("create SSH session: {}", e))?;
        session.set_tcp_stream(tcp);
        session
            .handshake()
            .map_err(|e| format!("SSH handshake with {}: {}", addr, e))?;

        verify_host_key(&session, &config.host, config.port)?;

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

        Ok(SshConnection { session })
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
        const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024; // 50 MB

        let sftp = self
            .session
            .sftp()
            .map_err(|e| format!("open SFTP channel: {}", e))?;
        let resolved = self.resolve_path(remote_path)?;

        let stat = sftp
            .stat(Path::new(&resolved))
            .map_err(|e| format!("stat '{}': {}", resolved, e))?;
        if let Some(size) = stat.size {
            if size > MAX_FILE_SIZE {
                return Err(format!(
                    "file too large: '{}' is {} bytes (limit: {} bytes)",
                    resolved, size, MAX_FILE_SIZE
                ));
            }
        }

        let file = sftp
            .open(Path::new(&resolved))
            .map_err(|e| format!("open '{}': {}", resolved, e))?;
        let mut contents = String::new();
        let bytes_read = file
            .take(MAX_FILE_SIZE + 1)
            .read_to_string(&mut contents)
            .map_err(|e| format!("read '{}': {}", resolved, e))?;
        if bytes_read as u64 > MAX_FILE_SIZE {
            return Err(format!(
                "file too large: '{}' exceeded {} bytes limit",
                resolved, MAX_FILE_SIZE
            ));
        }
        Ok(contents)
    }

    fn home_dir(&self) -> Result<String, String> {
        let sftp = self
            .session
            .sftp()
            .map_err(|e| format!("open SFTP channel: {}", e))?;
        let home = sftp
            .realpath(Path::new("."))
            .map_err(|e| format!("SFTP realpath for home dir: {}", e))?;
        let home_str = home.to_string_lossy().to_string();
        if !home_str.starts_with('/') {
            return Err(format!(
                "home directory is not an absolute path: '{}'",
                home_str
            ));
        }
        Ok(home_str)
    }

    fn resolve_path(&self, path: &str) -> Result<String, String> {
        if path == "~" {
            self.home_dir()
        } else if path.starts_with("~/") {
            let home = self.home_dir()?;
            Ok(format!("{}{}", home, &path[1..]))
        } else {
            Ok(path.to_string())
        }
    }

    pub fn disconnect(self) {
        let _ = self.session.disconnect(None, "bye", None);
    }
}

// ---------------------------------------------------------------------------
// Dedicated SSH worker thread
// ---------------------------------------------------------------------------

pub(crate) enum SshRequest {
    ListDir(String),
    ReadFile(String),
    Disconnect,
}

pub(crate) enum SshResponse {
    ListDir(Result<(String, Vec<RemoteEntry>), String>),
    ReadFile(Result<(String, String), String>),
    Disconnected,
}

pub(crate) struct SshWorkerHandle {
    pub config: SshServerConfig,
    tx: mpsc::Sender<SshRequest>,
    pub rx: mpsc::Receiver<SshResponse>,
}

impl SshWorkerHandle {
    pub fn send(&self, req: SshRequest) {
        let _ = self.tx.send(req);
    }
}

pub(crate) fn spawn_ssh_worker(
    config: SshServerConfig,
    ctx: Arc<OnceLock<eframe::egui::Context>>,
) -> mpsc::Receiver<Result<SshWorkerHandle, String>> {
    let (init_tx, init_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let conn = match SshConnection::connect(&config) {
            Ok(c) => c,
            Err(e) => {
                let _ = init_tx.send(Err(e));
                repaint(&ctx);
                return;
            }
        };
        let (req_tx, req_rx) = mpsc::channel::<SshRequest>();
        let (resp_tx, resp_rx) = mpsc::channel::<SshResponse>();
        let handle = SshWorkerHandle {
            config: config.clone(),
            tx: req_tx,
            rx: resp_rx,
        };
        let _ = init_tx.send(Ok(handle));
        repaint(&ctx);

        worker_loop(conn, req_rx, resp_tx, &ctx);
    });
    init_rx
}

fn worker_loop(
    conn: SshConnection,
    rx: mpsc::Receiver<SshRequest>,
    tx: mpsc::Sender<SshResponse>,
    ctx: &Arc<OnceLock<eframe::egui::Context>>,
) {
    while let Ok(req) = rx.recv() {
        let resp = match req {
            SshRequest::ListDir(path) => {
                let result = conn.list_dir(&path).map(|entries| (path, entries));
                SshResponse::ListDir(result)
            }
            SshRequest::ReadFile(path) => {
                let result = conn.read_file(&path).map(|contents| (path, contents));
                SshResponse::ReadFile(result)
            }
            SshRequest::Disconnect => {
                conn.disconnect();
                let _ = tx.send(SshResponse::Disconnected);
                repaint(ctx);
                return;
            }
        };
        let _ = tx.send(resp);
        repaint(ctx);
    }
    conn.disconnect();
}

fn repaint(ctx: &Arc<OnceLock<eframe::egui::Context>>) {
    if let Some(c) = ctx.get() {
        c.request_repaint();
    }
}
