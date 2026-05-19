use std::path::PathBuf;

use crate::session::Session;
use crate::Error;

#[derive(Debug, Clone, Copy)]
pub enum PermissionMode {
    AcceptEdits,
    Auto,
    BypassPermissions,
    Default,
    DontAsk,
    Plan,
}

impl PermissionMode {
    fn as_arg(self) -> &'static str {
        match self {
            PermissionMode::AcceptEdits => "acceptEdits",
            PermissionMode::Auto => "auto",
            PermissionMode::BypassPermissions => "bypassPermissions",
            PermissionMode::Default => "default",
            PermissionMode::DontAsk => "dontAsk",
            PermissionMode::Plan => "plan",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClaudeCodeBuilder {
    binary: Option<PathBuf>,
    cwd: Option<PathBuf>,
    model: Option<String>,
    permission_mode: Option<PermissionMode>,
    allowed_tools: Vec<String>,
    disallowed_tools: Vec<String>,
    extra_args: Vec<String>,
    rows: u16,
    cols: u16,
}

impl Default for ClaudeCodeBuilder {
    fn default() -> Self {
        Self {
            binary: None,
            cwd: None,
            model: None,
            permission_mode: None,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            extra_args: Vec::new(),
            rows: 40,
            cols: 120,
        }
    }
}

impl ClaudeCodeBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn binary(mut self, path: impl Into<PathBuf>) -> Self {
        self.binary = Some(path.into());
        self
    }

    pub fn cwd(mut self, path: impl Into<PathBuf>) -> Self {
        self.cwd = Some(path.into());
        self
    }

    pub fn model(mut self, name: impl Into<String>) -> Self {
        self.model = Some(name.into());
        self
    }

    pub fn permission_mode(mut self, mode: PermissionMode) -> Self {
        self.permission_mode = Some(mode);
        self
    }

    pub fn allowed_tools<I, S>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed_tools = tools.into_iter().map(Into::into).collect();
        self
    }

    pub fn disallowed_tools<I, S>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.disallowed_tools = tools.into_iter().map(Into::into).collect();
        self
    }

    pub fn extra_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.extra_args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn pty_size(mut self, rows: u16, cols: u16) -> Self {
        self.rows = rows;
        self.cols = cols;
        self
    }

    pub(crate) fn resolve(&self) -> Result<ResolvedSpec, Error> {
        let binary = match &self.binary {
            Some(p) => p.clone(),
            None => which::which("claude").map_err(|_| Error::BinaryNotFound)?,
        };

        let mut args: Vec<String> = Vec::new();
        if let Some(model) = &self.model {
            args.push("--model".into());
            args.push(model.clone());
        }
        if let Some(pm) = self.permission_mode {
            args.push("--permission-mode".into());
            args.push(pm.as_arg().into());
        }
        if !self.allowed_tools.is_empty() {
            args.push("--allowedTools".into());
            args.push(self.allowed_tools.join(","));
        }
        if !self.disallowed_tools.is_empty() {
            args.push("--disallowedTools".into());
            args.push(self.disallowed_tools.join(","));
        }
        for a in &self.extra_args {
            args.push(a.clone());
        }

        Ok(ResolvedSpec {
            binary,
            args,
            cwd: self.cwd.clone(),
            rows: self.rows,
            cols: self.cols,
        })
    }

    /// Spawn the interactive `claude` TUI under a PTY and return a
    /// `Session` you can read events from and write input to.
    pub fn open(self) -> Result<Session, Error> {
        Session::spawn(self.resolve()?)
    }
}

pub(crate) struct ResolvedSpec {
    pub binary: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub rows: u16,
    pub cols: u16,
}

/// Entry point: `ClaudeCode::builder()` -> [`ClaudeCodeBuilder`].
pub struct ClaudeCode;

impl ClaudeCode {
    pub fn builder() -> ClaudeCodeBuilder {
        ClaudeCodeBuilder::default()
    }
}
