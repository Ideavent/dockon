//! Local-only Caddy supervision. DNS and trust-store changes are separate approvals.
pub mod permissions;
use anyhow::{bail, Context, Result};
use dockon_core::{HealthCheck, Manifest, Project, RouterSettings, State};
use dockon_provider_native::Instance;
use dockon_runtime_manager::RuntimeManager;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader},
    sync::Mutex,
};

#[derive(Serialize)]
pub struct Plan {
    pub token: String,
    pub before: String,
    pub after: String,
    pub warnings: Vec<String>,
    pub hosts_entries: Vec<String>,
    pub trust_store_changed: bool,
}
#[derive(Serialize)]
pub struct Status {
    pub settings: Option<RouterSettings>,
    pub running: bool,
    pub pid: Option<u32>,
    pub error: Option<String>,
    pub root_certificate: Option<PathBuf>,
    pub logs: Vec<String>,
}
struct Pending {
    settings: RouterSettings,
    config: String,
    expires: Instant,
}
struct Inner {
    settings: Option<RouterSettings>,
    instance: Option<Instance>,
    manifest: Option<Manifest>,
    config: String,
    pending: BTreeMap<String, Pending>,
    error: Option<String>,
    failed_config: Option<String>,
}
pub struct Router {
    root: PathBuf,
    runtimes: Arc<RuntimeManager>,
    inner: Mutex<Inner>,
    logs: Arc<Mutex<VecDeque<String>>>,
    marker: String,
    permissions: Mutex<BTreeMap<String, permissions::Prepared>>,
}

pub fn validate_domain(domain: &str) -> Result<()> {
    if domain.len() > 253
        || domain != domain.to_ascii_lowercase()
        || domain.parse::<std::net::IpAddr>().is_ok()
        || domain.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
    {
        bail!("Enter a lowercase DNS hostname without scheme, port, wildcard or path (for example project.test)");
    }
    Ok(())
}
impl Router {
    pub fn new(root: PathBuf, runtimes: Arc<RuntimeManager>) -> Result<Self> {
        std::fs::create_dir_all(root.join("router"))?;
        let path = root.join("router/settings.json");
        let settings = if path.is_file() {
            let bytes = std::fs::read(path)?;
            if bytes.len() > 65536 {
                bail!("Router settings exceed size limit");
            }
            Some(serde_json::from_slice(&bytes)?)
        } else {
            None
        };
        Ok(Self {
            root,
            runtimes,
            inner: Mutex::new(Inner {
                settings,
                instance: None,
                manifest: None,
                config: String::new(),
                pending: BTreeMap::new(),
                error: None,
                failed_config: None,
            }),
            logs: Arc::default(),
            marker: uuid::Uuid::new_v4().to_string(),
            permissions: Mutex::new(BTreeMap::new()),
        })
    }
    fn directory(&self) -> PathBuf {
        self.root.join("router")
    }
    fn certificate(&self) -> PathBuf {
        self.directory()
            .join("storage/pki/authorities/local/root.crt")
    }
    fn render(&self, settings: &RouterSettings, projects: &[Project]) -> Result<String> {
        if settings.http_port < 1024
            || settings.https_port < 1024
            || settings.http_port == settings.https_port
        {
            bail!("Choose distinct unprivileged HTTP/HTTPS ports between 1024 and 65535");
        }
        if settings.routes.is_empty() || settings.routes.len() > 100 {
            bail!("Configure between 1 and 100 routes");
        }
        let mut domains = BTreeSet::new();
        let mut ids = BTreeSet::new();
        let storage = dunce::simplified(&self.directory().join("storage"))
            .to_string_lossy()
            .replace('\\', "/");
        if storage.contains(['"', '\n', '\r', '{', '}', '$']) {
            bail!("Router storage path contains unsupported configuration characters");
        }
        let mut result=format!("{{\n admin off\n persist_config off\n auto_https disable_redirects\n skip_install_trust\n storage file_system {{\n  root \"{storage}\"\n }}\n}}\n");
        for route in &settings.routes {
            validate_domain(&route.domain)?;
            if !domains.insert(&route.domain) || !ids.insert(&route.project_id) {
                bail!("Domains and projects must be unique in router settings");
            }
            let project = projects
                .iter()
                .find(|p| p.manifest.id == route.project_id)
                .context("Route refers to an unregistered project")?;
            if !matches!(project.manifest.health, HealthCheck::Http { .. }) {
                bail!("Only HTTP projects can use web routing");
            }
            let (scheme, port) = if route.https {
                ("https", settings.https_port)
            } else {
                ("http", settings.http_port)
            };
            result.push_str(&format!(
                "{scheme}://{}:{port} {{\n bind 127.0.0.1\n header X-Dockon-Router {}\n",
                route.domain, self.marker
            ));
            if route.https {
                result.push_str(" tls internal\n");
            }
            if matches!(project.state, State::Running | State::Degraded) {
                let upstream = project.port.context("Running project has no bound port")?;
                if [settings.http_port, settings.https_port].contains(&upstream) {
                    bail!("Router ports conflict with a project upstream");
                }
                result.push_str(&format!(" reverse_proxy 127.0.0.1:{upstream}\n"));
            } else {
                result.push_str(" respond \"Project is stopped in Dockon\" 503\n");
            }
            result.push_str("}\n");
        }
        Ok(result)
    }
    pub async fn status(&self) -> Result<Status> {
        let mut state = self.inner.lock().await;
        let running = if let Some(instance) = state.instance.as_mut() {
            instance.child.try_wait()?.is_none()
        } else {
            false
        };
        if !running && state.instance.is_some() {
            state.error = Some("Router process exited; review logs and apply a new plan".into());
        }
        Ok(Status {
            settings: state.settings.clone(),
            running,
            pid: if running {
                state.instance.as_ref().and_then(|v| v.child.id())
            } else {
                None
            },
            error: state.error.clone(),
            root_certificate: self.certificate().is_file().then(|| self.certificate()),
            logs: self.logs.lock().await.iter().cloned().collect(),
        })
    }
    async fn runtime_manifest(
        &self,
        settings: &RouterSettings,
        path: &Path,
        validate: bool,
    ) -> Result<Manifest> {
        let runtime = self
            .runtimes
            .selected(&settings.runtime_installation)
            .await?;
        if runtime.runtime != "caddy" {
            bail!("Select an installed Caddy runtime");
        }
        Ok(serde_json::from_value(
            serde_json::json!({"schema_version":1,"id":"dockon-router","name":"Dockon local router","root":dunce::canonicalize(self.directory())?,"executable":runtime.executable,"args":[if validate{"validate"}else{"run"},"--config",path,"--adapter","caddyfile"],"health":{"type":"tcp"}}),
        )?)
    }
    fn environment(&self) -> BTreeMap<String, String> {
        let mut values = BTreeMap::new();
        for key in ["XDG_DATA_HOME", "XDG_CONFIG_HOME"] {
            values.insert(key.into(), self.directory().to_string_lossy().into());
        }
        values
    }
    async fn validate(&self, settings: &RouterSettings, config: &str) -> Result<Manifest> {
        let path = self
            .directory()
            .join(format!("{}.Caddyfile", uuid::Uuid::new_v4()));
        std::fs::write(&path, config)?;
        let manifest = self.runtime_manifest(settings, &path, true).await?;
        let mut instance = Instance::spawn(&manifest, 0, &self.directory(), &self.environment())?;
        let stdout = instance
            .child
            .stdout
            .take()
            .context("Caddy stdout unavailable")?;
        let stderr = instance
            .child
            .stderr
            .take()
            .context("Caddy stderr unavailable")?;
        let mut out = vec![];
        let mut err = vec![];
        let result = tokio::time::timeout(Duration::from_secs(20), async {
            tokio::try_join!(
                async { stdout.take(131073).read_to_end(&mut out).await },
                async { stderr.take(131073).read_to_end(&mut err).await }
            )?;
            if out.len() > 131072 || err.len() > 131072 {
                bail!("Caddy validation output exceeds limit");
            }
            if !instance.child.wait().await?.success() {
                bail!(
                    "Caddy rejected generated configuration: {}",
                    String::from_utf8_lossy(&err)
                );
            }
            Ok::<_, anyhow::Error>(())
        })
        .await;
        match result {
            Ok(Ok(())) => {}
            other => {
                let _ = instance.stop("dockon-router").await;
                other.context("Caddy validation timed out")??;
            }
        }
        self.runtime_manifest(settings, &path, false).await
    }
    pub async fn plan(&self, settings: RouterSettings, projects: &[Project]) -> Result<Plan> {
        let config = self.render(&settings, projects)?;
        self.validate(&settings, &config).await?;
        let mut state = self.inner.lock().await;
        state.pending.retain(|_, v| v.expires > Instant::now());
        if state.pending.len() >= 16 {
            bail!("Too many outstanding router plans; close old previews and retry later");
        }
        let token = uuid::Uuid::new_v4().to_string();
        let hosts_entries = settings
            .routes
            .iter()
            .filter(|r| !r.domain.ends_with(".localhost"))
            .map(|r| format!("127.0.0.1 {}", r.domain))
            .collect();
        let mut warnings=vec!["Applying this plan restarts the shared router briefly; project processes remain running.".into(),"This plan does not change hosts or any certificate trust store. Custom public domain names can override normal browsing when mapped locally.".into()];
        if settings.routes.iter().any(|r| r.https) {
            warnings.push("HTTPS uses Dockon's private local CA. Browser trust requires a separate explicit approval.".into());
        }
        let result = Plan {
            token: token.clone(),
            before: state.config.clone(),
            after: config.clone(),
            warnings,
            hosts_entries,
            trust_store_changed: false,
        };
        state.pending.insert(
            token,
            Pending {
                settings,
                config,
                expires: Instant::now() + Duration::from_secs(300),
            },
        );
        Ok(result)
    }
    async fn spawn(&self, manifest: &Manifest, settings: &RouterSettings) -> Result<Instance> {
        let mut instance = Instance::spawn(manifest, 0, &self.directory(), &self.environment())?;
        if let Some(stdout) = instance.child.stdout.take() {
            capture(stdout, self.logs.clone());
        }
        if let Some(stderr) = instance.child.stderr.take() {
            capture(stderr, self.logs.clone());
        }
        let result = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if instance.child.try_wait()?.is_some() {
                    bail!("Caddy exited during startup; see router logs");
                }
                if self.probe(settings).await.is_ok() {
                    return Ok::<_, anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
        match result {
            Ok(Ok(())) => Ok(instance),
            other => {
                let _ = instance.stop("dockon-router").await;
                other.context("Router readiness timed out")??;
                unreachable!()
            }
        }
    }
    async fn probe(&self, settings: &RouterSettings) -> Result<()> {
        for route in &settings.routes {
            let port = if route.https {
                settings.https_port
            } else {
                settings.http_port
            };
            let mut client = reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_millis(500))
                .resolve(
                    &route.domain,
                    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port),
                );
            if route.https {
                client = client.add_root_certificate(reqwest::Certificate::from_pem(
                    &std::fs::read(self.certificate())?,
                )?);
            }
            let response = client
                .build()?
                .get(format!(
                    "{}://{}:{port}/",
                    if route.https { "https" } else { "http" },
                    route.domain
                ))
                .send()
                .await?;
            if response
                .headers()
                .get("X-Dockon-Router")
                .and_then(|v| v.to_str().ok())
                != Some(&self.marker)
            {
                bail!("Listener did not identify as this Dockon router");
            }
        }
        Ok(())
    }
    async fn replace(
        &self,
        state: &mut Inner,
        settings: &RouterSettings,
        config: &str,
    ) -> Result<()> {
        let manifest = self.validate(settings, config).await?;
        if let Some(mut instance) = state.instance.take() {
            instance.stop("dockon-router").await?;
        }
        match self.spawn(&manifest, settings).await {
            Ok(instance) => {
                state.instance = Some(instance);
                state.manifest = Some(manifest);
                state.config = config.into();
                state.error = None;
                state.failed_config = None;
                Ok(())
            }
            Err(error) => {
                if let (Some(previous), Some(previous_settings)) =
                    (&state.manifest, &state.settings)
                {
                    match self.spawn(previous, previous_settings).await {
                        Ok(instance) => state.instance = Some(instance),
                        Err(rollback) => {
                            state.error=Some(format!("Router startup failed: {error:#}; rollback also failed: {rollback:#}"));
                            bail!("{}", state.error.as_deref().unwrap_or_default());
                        }
                    }
                }
                state.error=Some(format!("New router configuration failed; previous configuration restored when available: {error:#}"));
                Err(error)
            }
        }
    }
    pub async fn apply(&self, token: &str, projects: &[Project]) -> Result<()> {
        let mut state = self.inner.lock().await;
        let pending = state
            .pending
            .remove(token)
            .context("Router plan token is unknown or already used")?;
        if pending.expires < Instant::now()
            || self.render(&pending.settings, projects)? != pending.config
        {
            bail!("Router plan expired or project state changed; preview it again");
        }
        self.replace(&mut state, &pending.settings, &pending.config)
            .await?;
        state.settings = Some(pending.settings.clone());
        let saved = (|| -> Result<()> {
            let temporary = self.directory().join("settings.tmp");
            std::fs::write(&temporary, serde_json::to_vec_pretty(&pending.settings)?)?;
            std::fs::rename(temporary, self.directory().join("settings.json"))?;
            Ok(())
        })();
        if let Err(error) = saved {
            state.error = Some(format!(
                "Router is running with the new settings, but saving them failed: {error:#}"
            ));
            return Err(error);
        }
        Ok(())
    }
    pub async fn refresh(&self, projects: &[Project]) -> Result<()> {
        let mut state = self.inner.lock().await;
        if state.instance.is_none() {
            return Ok(());
        }
        let settings = state
            .settings
            .clone()
            .context("Running router has no settings")?;
        let config = self.render(&settings, projects)?;
        if config != state.config && state.failed_config.as_deref() != Some(&config) {
            if let Err(error) = self.replace(&mut state, &settings, &config).await {
                state.failed_config = Some(config);
                return Err(error);
            }
        }
        Ok(())
    }
    pub async fn stop(&self) -> Result<()> {
        let mut state = self.inner.lock().await;
        if let Some(mut instance) = state.instance.take() {
            instance.stop("dockon-router").await?;
        }
        Ok(())
    }
    pub fn root_fingerprint(&self) -> Result<String> {
        Ok(format!(
            "{:x}",
            Sha256::digest(std::fs::read(self.certificate())?)
        ))
    }
}
fn capture(stream: impl AsyncRead + Unpin + Send + 'static, logs: Arc<Mutex<VecDeque<String>>>) {
    tokio::spawn(async move {
        let mut stream = BufReader::new(stream);
        let mut bytes = vec![];
        loop {
            bytes.clear();
            match (&mut stream).take(4097).read_until(b'\n', &mut bytes).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let mut logs = logs.lock().await;
                    logs.push_back(
                        String::from_utf8_lossy(&bytes[..bytes.len().min(4096)])
                            .trim()
                            .into(),
                    );
                    while logs.len() > 40 {
                        logs.pop_front();
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn domains_cannot_inject_configuration() {
        for domain in [
            "shop.test",
            "custom.example.com",
            "xn--bcher-kva.test",
            "custom",
        ] {
            validate_domain(domain).unwrap();
        }
        for domain in [
            "https://shop.test",
            "*.test",
            "shop.test:443",
            "shop.test\n}",
            "shop..test",
            "127.0.0.1",
            "Shop.test",
            "-shop.test",
        ] {
            assert!(validate_domain(domain).is_err());
        }
    }
}
