use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LaunchArg {
    Literal(String),
    Template(ArgumentTemplate),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArgumentTemplate {
    pub template: String,
}
impl From<&str> for LaunchArg {
    fn from(value: &str) -> Self {
        Self::Literal(value.into())
    }
}
impl From<String> for LaunchArg {
    fn from(value: String) -> Self {
        Self::Literal(value)
    }
}
impl LaunchArg {
    pub fn template(value: impl Into<String>) -> Self {
        Self::Template(ArgumentTemplate {
            template: value.into(),
        })
    }
    pub fn text(&self) -> &str {
        match self {
            Self::Literal(value) => value,
            Self::Template(value) => &value.template,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.text().contains('\0') {
            bail!("NUL is forbidden in launch arguments");
        }
        if let Self::Template(value) = self {
            let mut rest = value.template.as_str();
            while let Some(start) = rest.find("${") {
                let end = rest[start..]
                    .find('}')
                    .context("Unclosed launch argument template")?
                    + start;
                let variable = &rest[start + 2..end];
                if !matches!(variable, "DOCKON_PORT" | "DOCKON_HOST" | "DOCKON_DATA") {
                    bail!("Unsupported launch argument template variable");
                }
                rest = &rest[end + 1..];
            }
        }
        Ok(())
    }
    pub fn resolve(&self, port: u16, data: &Path) -> Result<String> {
        self.validate()?;
        Ok(match self {
            Self::Literal(value) => value.clone(),
            Self::Template(value) => value
                .template
                .replace("${DOCKON_PORT}", &port.to_string())
                .replace("${DOCKON_HOST}", "127.0.0.1")
                .replace("${DOCKON_DATA}", &data.to_string_lossy()),
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeBinding {
    pub installation_id: String,
    #[serde(default)]
    pub isolated_python: bool,
}
fn default_start_timeout() -> u64 {
    30
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Native,
    Container,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerEngine {
    #[default]
    Docker,
    Podman,
}
impl ContainerEngine {
    pub fn executable(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerSpec {
    #[serde(default)]
    pub engine: ContainerEngine,
    pub image: String,
    pub internal_port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_port: Option<u16>,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub entrypoint: Option<String>,
}
pub const POSTGRES_IMAGE: &str = "docker.io/library/postgres@sha256:d3244e8ce3f165ee3c93d9c4a4b1d0fe48b3a89e2287b717fdcbbde92d405065";
pub const MYSQL_IMAGE: &str = "docker.io/library/mysql@sha256:a81d043930e51745522bd5c430961cf8286f0a23f0a9a2a6e0108f7ea71b0902";
pub const MARIADB_IMAGE: &str = "docker.io/library/mariadb@sha256:d36f18d65a0459510b8b0f643b226a6d299e3206486e2b64ff20754a6d291707";
pub const MONGO_IMAGE: &str = "docker.io/library/mongo@sha256:d97fea17ccb8d004cd137be8dab54d70d0df11be69c9f422aefe4484c8414ab7";
impl ContainerSpec {
    pub fn validate(&self) -> Result<()> {
        if self
            .smtp_port
            .is_some_and(|port| port == 0 || port == self.internal_port)
        {
            bail!("SMTP must use a distinct nonzero container port");
        }
        let (image, digest) = self
            .image
            .split_once("@sha256:")
            .context("Container image must be pinned with @sha256:<64 hex digits>")?;
        if image.is_empty()
            || image.starts_with('-')
            || image.chars().any(|c| c.is_whitespace() || c.is_control())
            || digest.len() != 64
            || !digest.bytes().all(|b| b.is_ascii_hexdigit())
        {
            bail!("Invalid pinned image reference");
        }
        if self.internal_port == 0 {
            bail!("Container internal port must be nonzero");
        }
        if self.engine == ContainerEngine::Podman {
            let registry = image.split('/').next().unwrap_or_default();
            if !image.contains('/')
                || !(registry.contains('.') || registry.contains(':') || registry == "localhost")
            {
                bail!("Podman requires an explicit image registry");
            }
        }
        if self.command.iter().any(|s| s.contains('\0')) {
            bail!("NUL is forbidden in container arguments");
        }
        if self
            .entrypoint
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.contains('\0'))
        {
            bail!("Invalid container entrypoint");
        }
        Ok(())
    }
}

pub fn project_key(id: &str) -> String {
    id.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HealthCheck {
    Http { path: String },
    Tcp,
}
impl Default for HealthCheck {
    fn default() -> Self {
        Self::Http {
            path: "/health".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceSpec {
    pub id: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub executable: PathBuf,
    #[serde(default)]
    pub args: Vec<LaunchArg>,
    #[serde(default)]
    pub runtime: Option<RuntimeBinding>,
    #[serde(default = "default_start_timeout")]
    pub startup_timeout_seconds: u64,
    #[serde(default)]
    pub provider: Provider,
    #[serde(default)]
    pub container: Option<ContainerSpec>,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub health: HealthCheck,
    #[serde(default = "default_working_dir")]
    pub working_dir: PathBuf,
}
fn default_working_dir() -> PathBuf {
    PathBuf::from(".")
}
impl ServiceSpec {
    pub fn owner(&self, parent: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(parent.as_bytes());
        hash.update([0]);
        hash.update(self.id.as_bytes());
        format!("{:x}", hash.finalize())
    }
    pub fn manifest(&self, parent: &Manifest) -> Result<Manifest> {
        let root = parent
            .root
            .join(&self.working_dir)
            .canonicalize()
            .context("Service working directory unavailable")?;
        if !root.starts_with(parent.root.canonicalize()?) {
            bail!("Service working directory escapes project root");
        }
        let manifest = Manifest {
            schema_version: 1,
            id: self.owner(&parent.id),
            name: format!("{} / {}", parent.name, self.id),
            root,
            executable: self.executable.clone(),
            args: self.args.clone(),
            provider: self.provider,
            container: self.container.clone(),
            port: self.port,
            health: self.health.clone(),
            services: vec![],
            runtime: self.runtime.clone(),
            startup_timeout_seconds: self.startup_timeout_seconds,
        };
        manifest.validate()?;
        Ok(manifest)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    #[serde(default)]
    pub executable: PathBuf,
    #[serde(default)]
    pub provider: Provider,
    #[serde(default)]
    pub container: Option<ContainerSpec>,
    #[serde(default)]
    pub args: Vec<LaunchArg>,
    #[serde(default)]
    pub runtime: Option<RuntimeBinding>,
    #[serde(default = "default_start_timeout")]
    pub startup_timeout_seconds: u64,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub health: HealthCheck,
    #[serde(default)]
    pub services: Vec<ServiceSpec>,
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
        match self.provider {
            Provider::Native => {
                if !self.executable.is_absolute() || !self.executable.is_file() {
                    bail!("Executable must be an existing absolute file");
                }
                if self.container.is_some() {
                    bail!("Native projects cannot contain a container configuration");
                }
            }
            Provider::Container => self
                .container
                .as_ref()
                .context("Container configuration required")?
                .validate()?,
        }
        for arg in &self.args {
            arg.validate()?;
        }
        if self.startup_timeout_seconds == 0 || self.startup_timeout_seconds > 300 {
            bail!("Startup timeout must be between 1 and 300 seconds");
        }
        if self
            .runtime
            .as_ref()
            .is_some_and(|v| v.installation_id.is_empty() || v.installation_id.len() > 256)
        {
            bail!("Invalid runtime binding");
        }
        if let HealthCheck::Http { path } = &self.health {
            if !path.starts_with('/')
                || path.starts_with("//")
                || path.chars().any(char::is_control)
            {
                bail!("Invalid local healthcheck path");
            }
        }
        for service in &self.services {
            service.manifest(self)?;
        }
        self.service_order()?;
        if self.provider == Provider::Container {
            let engine = self
                .container
                .as_ref()
                .context("Container configuration required")?
                .engine;
            for service in &self.services {
                if service.provider != Provider::Container {
                    bail!("Container-to-native dependency routing is not implemented");
                }
                if service
                    .container
                    .as_ref()
                    .context("Service container configuration required")?
                    .engine
                    != engine
                {
                    bail!("Cross-engine container networking is not implemented");
                }
            }
        }
        Ok(())
    }
    pub fn service_order(&self) -> Result<Vec<&ServiceSpec>> {
        use std::collections::{BTreeMap, BTreeSet};
        if self.services.len() > 32 {
            bail!("A project may contain at most 32 auxiliary services");
        }
        let mut services = BTreeMap::new();
        for service in &self.services {
            if service.id.is_empty()
                || service.id.len() > 32
                || service.id == "app"
                || !service
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
            {
                bail!("Invalid or reserved service id");
            }
            if services.insert(service.id.as_str(), service).is_some() {
                bail!("Duplicate service id: {}", service.id);
            }
        }
        fn visit<'a>(
            id: &'a str,
            services: &BTreeMap<&'a str, &'a ServiceSpec>,
            active: &mut BTreeSet<&'a str>,
            done: &mut BTreeSet<&'a str>,
            out: &mut Vec<&'a ServiceSpec>,
        ) -> Result<()> {
            if done.contains(id) {
                return Ok(());
            }
            if !active.insert(id) {
                bail!("Dependency cycle at service {id}");
            }
            let service = services.get(id).context("Unresolved service dependency")?;
            for dependency in &service.depends_on {
                visit(dependency, services, active, done, out)?;
            }
            active.remove(id);
            done.insert(id);
            out.push(service);
            Ok(())
        }
        let mut result = vec![];
        let mut active = BTreeSet::new();
        let mut done = BTreeSet::new();
        for id in services.keys() {
            visit(id, &services, &mut active, &mut done, &mut result)?;
        }
        Ok(result)
    }
    pub fn read(path: &Path) -> Result<Self> {
        use std::io::Read;
        if !path.is_file() {
            bail!("Manifest must be a regular file");
        }
        let file = std::fs::File::open(path).context("Read manifest")?;
        let mut raw = Vec::new();
        file.take(65537).read_to_end(&mut raw)?;
        if raw.len() > 65536 {
            bail!("Manifest exceeds 64 KiB");
        }
        let content = raw.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&raw);
        let mut value: Self = serde_json::from_slice(content)?;
        value.validate()?;
        value.root = value.root.canonicalize()?;
        if value.provider == Provider::Native {
            value.executable = value.executable.canonicalize()?;
        }
        Ok(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum State {
    Stopped,
    Starting,
    Running,
    Degraded,
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
    pub container_id: Option<String>,
    pub services: Vec<ServiceStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub id: String,
    pub state: State,
    pub pid: Option<u32>,
    pub port: u16,
    pub container_id: Option<String>,
    pub smtp_port: Option<u16>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeInstallation {
    pub id: String,
    pub runtime: String,
    pub version: String,
    pub platform: String,
    pub root: PathBuf,
    pub executable: PathBuf,
    pub source: String,
    pub archive_hash: Option<String>,
    pub executable_hash: String,
    pub signature_verified: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Command {
    SystemPlan {
        action: SystemAction,
    },
    SystemApply {
        token: String,
    },
    RouterStatus,
    RouterPlan {
        settings: RouterSettings,
    },
    RouterApply {
        token: String,
    },
    RouterStop,
    Health,
    Capabilities,
    Diagnostics,
    GuardianPreview,
    GuardianModels {
        api_key: String,
    },
    GuardianAnalyze {
        api_key: String,
        model: String,
        context: serde_json::Value,
    },
    AdvisoryLookup {
        ecosystem: String,
        package: String,
        version: String,
    },
    RuntimeUpdates {
        runtime: String,
    },
    RuntimeCatalog {
        runtime: String,
    },
    RuntimeInstalled,
    RuntimeInstall {
        runtime: String,
        version: String,
    },
    RuntimeDiscover,
    RuntimeUseSystem {
        runtime: String,
        executable: PathBuf,
    },
    ProjectDetect {
        root: PathBuf,
    },
    ProjectCreate {
        id: String,
        name: String,
        root: PathBuf,
        runtime_installation: String,
    },
    ProjectImport {
        id: String,
        name: String,
        root: PathBuf,
        runtime_installation: String,
        profile: String,
        #[serde(default)]
        php_cgi_installation: Option<String>,
    },
    ProjectSwitchStaticServer {
        id: String,
        runtime_installation: String,
    },
    ProjectAddRedis {
        id: String,
    },
    ProjectAddMailpit {
        id: String,
    },
    ProjectAddPostgres {
        id: String,
    },
    ProjectAddDatabase {
        id: String,
        engine: String,
    },
    PostgresConnection {
        id: String,
    },
    DatabaseConnection {
        id: String,
        engine: String,
    },
    PostgresBackup {
        id: String,
    },
    PostgresBackups {
        id: String,
    },
    PostgresRestore {
        id: String,
        backup: String,
    },
    List,
    Register {
        manifest_path: PathBuf,
    },
    Start {
        id: String,
    },
    Stop {
        id: String,
    },
    Operation {
        id: String,
    },
    Events {
        after: u64,
    },
    Subscribe {
        after: u64,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterSettings {
    pub runtime_installation: String,
    pub http_port: u16,
    pub https_port: u16,
    pub routes: Vec<ProjectRoute>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemAction {
    HostsUpdate,
    HostsRemove,
    TrustCa,
    UntrustCa,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRoute {
    pub project_id: String,
    pub domain: String,
    pub https: bool,
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
    fn typed_templates_preserve_literal_arguments_and_reject_unknown_variables() {
        let data = std::path::Path::new("/private/project");
        assert_eq!(
            LaunchArg::from("${DOCKON_PORT}")
                .resolve(4321, data)
                .unwrap(),
            "${DOCKON_PORT}"
        );
        assert_eq!(
            LaunchArg::template("${DOCKON_HOST}:${DOCKON_PORT}")
                .resolve(4321, data)
                .unwrap(),
            "127.0.0.1:4321"
        );
        assert!(LaunchArg::template("${HOME}").validate().is_err());
        assert!(LaunchArg::template("${DOCKON_PORT").validate().is_err());
    }
    fn manifest(services: serde_json::Value) -> Manifest {
        serde_json::from_value(serde_json::json!({"schema_version":1,"id":"test","name":"test","root":std::env::current_dir().unwrap(),"executable":std::env::current_exe().unwrap(),"services":services})).unwrap()
    }
    #[test]
    fn dependency_graph_rejects_cycles_missing_nodes_and_path_escape() {
        let exe = std::env::current_exe().unwrap();
        let valid = manifest(
            serde_json::json!([{"id":"worker","executable":exe,"depends_on":["cache"]},{"id":"cache","executable":exe}]),
        );
        assert_eq!(
            valid
                .service_order()
                .unwrap()
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            vec!["cache", "worker"]
        );
        assert!(manifest(serde_json::json!([{"id":"a","executable":exe,"depends_on":["b"]},{"id":"b","executable":exe,"depends_on":["a"]}])).validate().is_err());
        assert!(manifest(
            serde_json::json!([{"id":"a","executable":exe,"depends_on":["missing"]}])
        )
        .validate()
        .is_err());
        assert!(
            manifest(serde_json::json!([{"id":"a","executable":exe,"working_dir":".."}]))
                .validate()
                .is_err()
        );
    }
    #[test]
    fn requires_digest_and_nonzero_internal_port() {
        let mut spec = ContainerSpec {
            smtp_port: None,
            engine: ContainerEngine::Docker,
            image: "redis:latest".into(),
            internal_port: 6379,
            command: vec![],
            entrypoint: None,
        };
        assert!(spec.validate().is_err());
        spec.image = format!("redis@sha256:{}", "a".repeat(64));
        assert!(spec.validate().is_ok());
        spec.internal_port = 0;
        assert!(spec.validate().is_err());
    }
    #[test]
    fn rejects_unsafe_ids_and_unknown_fields() {
        let m = Manifest {
            schema_version: 1,
            id: "../escape".into(),
            name: "test".into(),
            root: std::env::current_dir().unwrap(),
            executable: std::env::current_exe().unwrap(),
            provider: Provider::Native,
            container: None,
            health: HealthCheck::default(),
            services: vec![],
            runtime: None,
            startup_timeout_seconds: 30,
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
