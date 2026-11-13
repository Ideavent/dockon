use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    pub executable: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub port: u16,
}

impl Manifest {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            bail!("Unsupported manifest schema");
        }
        if self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            bail!("Project id must contain 1–64 ASCII letters, digits, '-' or '_'");
        }
        if self.name.trim().is_empty() || self.name.len() > 200 {
            bail!("Invalid project name");
        }
        if !self.root.is_absolute() || !self.root.is_dir() {
            bail!("Project root must be an existing absolute directory");
        }
        if !self.executable.is_absolute() || !self.executable.is_file() {
            bail!("Executable must be an existing absolute file");
        }
        if self.args.iter().any(|arg| arg.contains('\0')) {
            bail!("NUL is forbidden in arguments");
        }
        Ok(())
    }
    pub fn read(path: &Path) -> Result<Self> {
        let raw = std::fs::read(path).context("Read manifest")?;
        if raw.len() > 65536 {
            bail!("Manifest exceeds 64 KiB");
        }
        let mut value: Self = serde_json::from_slice(&raw)?;
        value.validate()?;
        value.root = value.root.canonicalize()?;
        value.executable = value.executable.canonicalize()?;
        Ok(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum State {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub manifest: Manifest,
    pub state: State,
    pub pid: Option<u32>,
    pub port: Option<u16>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub project_id: String,
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBatch {
    pub epoch: String,
    pub events: Vec<Event>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub id: String,
    pub project_id: String,
    pub action: String,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Command {
    Health,
    List,
    Register { manifest_path: PathBuf },
    Start { id: String },
    Stop { id: String },
    Operation { id: String },
    Events { after: u64 },
    Subscribe { after: u64 },
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub version: u32,
    pub token: String,
    pub command: Command,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub version: u32,
    pub ok: bool,
    pub data: serde_json::Value,
    pub error: Option<String>,
}
impl Response {
    pub fn success(data: impl Serialize) -> Self {
        Self {
            version: 1,
            ok: true,
            data: serde_json::to_value(data).unwrap_or_default(),
            error: None,
        }
    }
    pub fn failure(error: impl ToString) -> Self {
        Self {
            version: 1,
            ok: false,
            data: serde_json::Value::Null,
            error: Some(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unsafe_ids_and_unknown_fields() {
        let m = Manifest {
            schema_version: 1,
            id: "../escape".into(),
            name: "test".into(),
            root: std::env::current_dir().unwrap(),
            executable: std::env::current_exe().unwrap(),
            args: vec![],
            port: 0,
        };
        assert!(m.validate().is_err());
        assert!(serde_json::from_str::<Manifest>(
            r#"{"schema_version":1,"id":"a","name":"a","root":"/","executable":"/x","secret":"x"}"#
        )
        .is_err());
    }
}
