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
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream},
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
    pub certificate_trusted: bool,
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

pub fn loopback_port_available(port: u16) -> bool {
    let addresses = [
        SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        SocketAddr::from((Ipv6Addr::LOCALHOST, port)),
    ];
    addresses
        .iter()
        .all(|addr| TcpStream::connect_timeout(addr, Duration::from_millis(100)).is_err())
        && TcpListener::bind(addresses[0]).is_ok()
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
        let supported = |port, standard| port >= 1024 || (cfg!(windows) && port == standard);
        if !supported(settings.http_port, 80)
            || !supported(settings.https_port, 443)
            || settings.http_port == settings.https_port
        {
            bail!("Choose distinct web ports. Windows supports HTTP 80 and HTTPS 443, or ports from 1024 through 65535.");
        }
        if settings.routes.is_empty() || settings.routes.len() > 100 {
            bail!("Configure between 1 and 100 routes");
        }
        let mut domains: BTreeSet<String> = settings
            .routes
            .iter()
            .map(|route| route.domain.clone())
            .collect();
        if domains.len() != settings.routes.len() {
            bail!("Domains and projects must be unique in router settings");
        }
        let mut ids = BTreeSet::new();
        let bind = if TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).is_ok() {
            "127.0.0.1 ::1"
        } else {
            "127.0.0.1"
        };
        let storage = dunce::simplified(&self.directory().join("storage"))
            .to_string_lossy()
            .replace('\\', "/");
        if storage.contains(['"', '\n', '\r', '{', '}', '$']) {
            bail!("Router storage path contains unsupported configuration characters");
        }
        let mut result=format!("{{\n admin off\n persist_config off\n auto_https disable_redirects\n skip_install_trust\n storage file_system {{\n  root \"{storage}\"\n }}\n}}\n");
        for route in &settings.routes {
            validate_domain(&route.domain)?;
            if !ids.insert(&route.project_id) {
                bail!("Domains and projects must be unique in router settings");
            }
            let project = projects
                .iter()
                .find(|p| p.manifest.id == route.project_id)
                .context("Route refers to an unregistered project")?;
            if !matches!(project.manifest.health, HealthCheck::Http { .. })
                && !project.has_mailpit()
            {
                bail!("Only HTTP projects can use web routing");
            }
            let (scheme, port) = if route.https {
                ("https", settings.https_port)
            } else {
                ("http", settings.http_port)
            };
            let mut addresses = format!("{scheme}://{}:{port}", route.domain);
            if let Some(alias) = settings.workspace_alias(route, project) {
                addresses.push_str(&format!(", {scheme}://{alias}:{port}"));
            }
            let upstream = if matches!(project.manifest.health, HealthCheck::Http { .. })
                && matches!(project.state, State::Running | State::Degraded)
            {
                let upstream = project.port.context("Running project has no bound port")?;
                if [settings.http_port, settings.https_port].contains(&upstream) {
                    bail!("Router ports conflict with a project upstream");
                }
                if project
                    .manifest
                    .template
                    .as_deref()
                    .is_some_and(|template| {
                        template.starts_with("vite-") || template.starts_with("django")
                    })
                {
                    format!(" reverse_proxy 127.0.0.1:{upstream} {{\n  header_up Host 127.0.0.1:{upstream}\n }}\n")
                } else {
                    format!(" reverse_proxy 127.0.0.1:{upstream}\n")
                }
            } else {
                " respond \"Project is stopped in Dockon\" 503\n".into()
            };
            let tls = if route.https { " tls internal\n" } else { "" };
            let dedicated = cfg!(windows) && !route.domain.ends_with(".localhost");
            let standard = if route.https { 443 } else { 80 };
            let project_bind = if dedicated && port == standard {
                format!("{bind} {}", dockon_core::WINDOWS_WEBMAIL_IP)
            } else {
                bind.to_owned()
            };
            result.push_str(&format!(
                "{addresses} {{\n bind {project_bind}\n header X-Dockon-Router {}\n{tls}{upstream}}}\n",
                self.marker
            ));
            if dedicated && port != standard {
                result.push_str(&format!(
                    "{scheme}://{}:{standard} {{\n bind {}\n header X-Dockon-Router {}\n{tls}{upstream}}}\n",
                    route.domain, dockon_core::WINDOWS_WEBMAIL_IP, self.marker
                ));
            }
            if project.has_mailpit() {
                let smtp_domain = format!("mail.{}", route.domain);
                let domain = format!("webmail.{}", route.domain);
                for name in [&smtp_domain, &domain] {
                    validate_domain(name)?;
                    if !domains.insert(name.clone()) {
                        bail!("Mail address {name} conflicts with another route");
                    }
                }
                let upstream = if let Some(mail) = project
                    .services
                    .iter()
                    .find(|service| service.id == "mailpit" && service.state == State::Running)
                {
                    if [settings.http_port, settings.https_port].contains(&mail.port) {
                        bail!("Router ports conflict with the project mail server");
                    }
                    format!(
                        " reverse_proxy 127.0.0.1:{} {{\n  header_up Host 127.0.0.1\n }}\n",
                        mail.port
                    )
                } else {
                    " respond \"Project mail is stopped in Dockon\" 503\n".into()
                };
                if cfg!(windows) && !smtp_domain.ends_with(".localhost") {
                    let ip = dockon_core::project_smtp_ip(&route.project_id);
                    let project_origin = route.domain.replace('.', "[.]");
                    result.push_str(&format!(
                        r#"http://{smtp_domain}:80 {{
 bind {ip}
 header X-Dockon-Router {marker}
 @app_origin header_regexp Origin ^https?://{project_origin}$
 header @app_origin Access-Control-Allow-Origin {{http.request.header.Origin}}
 header @app_origin +Vary Origin
 @preflight {{
  method OPTIONS
  header_regexp Origin ^https?://{project_origin}$
 }}
 header @preflight Access-Control-Allow-Methods "GET, POST, PUT, DELETE, OPTIONS"
 header @preflight Access-Control-Allow-Headers "Content-Type, Authorization"
 respond @preflight 204
{upstream}}}
"#,
                        marker = self.marker
                    ));
                }
                let alias = settings.webmail_alias(route, project);
                if cfg!(windows) && !domain.ends_with(".localhost") {
                    result.push_str(&format!(
                        "http://{domain}:80 {{\n bind {}\n header X-Dockon-Router {}\n{upstream}}}\n",
                        dockon_core::WINDOWS_WEBMAIL_IP, self.marker
                    ));
                }
                let mut addresses = Vec::new();
                if !cfg!(windows) || domain.ends_with(".localhost") {
                    addresses.push(format!("http://{domain}:{}", settings.http_port));
                }
                if validate_domain(&alias).is_ok() && domains.insert(alias.clone()) {
                    addresses.push(format!("http://{alias}:{}", settings.http_port));
                }
                if !addresses.is_empty() {
                    let addresses = addresses.join(", ");
                    result.push_str(&format!(
                        "{addresses} {{\n bind {bind}\n header X-Dockon-Router {}\n{upstream}}}\n",
                        self.marker
                    ));
                }
            }
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
        let certificate_path = self.certificate();
        Ok(Status {
            settings: state.settings.clone(),
            running,
            pid: if running {
                state.instance.as_ref().and_then(|v| v.child.id())
            } else {
                None
            },
            error: state.error.clone(),
            certificate_trusted: self.certificate_trusted(),
            root_certificate: certificate_path.is_file().then_some(certificate_path),
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
        let hosts_entries = hosts_entries(&settings, projects);
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
    async fn spawn(
        &self,
        manifest: &Manifest,
        settings: &RouterSettings,
        config: &str,
    ) -> Result<Instance> {
        for (scheme, port) in [
            ("http://", settings.http_port),
            ("https://", settings.https_port),
        ] {
            if config.lines().any(|line| line.starts_with(scheme)) && !loopback_port_available(port)
            {
                bail!("Local router port {port} is in use");
            }
        }
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
        match self.spawn(&manifest, settings, config).await {
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
                    match self.spawn(previous, previous_settings, &state.config).await {
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
        if let Err(error) = self.persist_settings(&pending.settings) {
            state.error = Some(format!(
                "Router is running with the new settings, but saving them failed: {error:#}"
            ));
            return Err(error);
        }
        Ok(())
    }
    fn persist_settings(&self, settings: &RouterSettings) -> Result<()> {
        let temporary = self.directory().join("settings.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(settings)?)?;
        std::fs::rename(temporary, self.directory().join("settings.json"))?;
        Ok(())
    }

    pub async fn save_preferences(
        &self,
        settings: RouterSettings,
        projects: &[Project],
    ) -> Result<()> {
        {
            let mut state = self.inner.lock().await;
            let same_routes = state
                .settings
                .as_ref()
                .is_some_and(|previous| previous.domain_suffix == settings.domain_suffix);
            if same_routes || settings.routes.is_empty() {
                self.persist_settings(&settings)?;
                state.settings = Some(settings);
                return Ok(());
            }
        }
        let plan = self.plan(settings, projects).await?;
        self.apply(&plan.token, projects).await
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

fn hosts_entries(settings: &RouterSettings, projects: &[Project]) -> Vec<String> {
    let mut entries = Vec::new();
    for route in settings
        .routes
        .iter()
        .filter(|route| !route.domain.ends_with(".localhost"))
    {
        let web_ip = if cfg!(windows) {
            dockon_core::WINDOWS_WEBMAIL_IP
        } else {
            Ipv4Addr::LOCALHOST
        };
        entries.push(format!("{web_ip} {}", route.domain));
        if projects
            .iter()
            .any(|project| project.manifest.id == route.project_id && project.has_mailpit())
        {
            let webmail_ip = if cfg!(windows) {
                dockon_core::WINDOWS_WEBMAIL_IP
            } else {
                Ipv4Addr::LOCALHOST
            };
            entries.push(format!("{webmail_ip} webmail.{}", route.domain));
            if cfg!(windows) {
                let smtp_ip = dockon_core::project_smtp_ip(&route.project_id);
                entries.push(format!("{smtp_ip} mail.{}", route.domain));
            }
        }
    }
    entries
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
