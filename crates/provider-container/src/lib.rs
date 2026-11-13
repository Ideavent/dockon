use anyhow::{bail, Context, Result};
use dockon_core::{project_key, ContainerEngine, ContainerSpec};
use serde::Serialize;
use serde_json::Value;
use std::{process::Stdio, time::Duration};
use tokio::process::Command;

const INSTALLATION: &str = "dev.dockon.installation";
const PROJECT: &str = "dev.dockon.project";
const INSTANCE: &str = "dev.dockon.instance";

#[derive(Debug, Serialize)]
pub struct Capability {
    pub engine: ContainerEngine,
    pub available: bool,
    pub detail: String,
    pub experimental: bool,
}

pub async fn capability(engine: ContainerEngine) -> Capability {
    let result = execute(
        engine,
        &[
            "info".into(),
            "--format".into(),
            if engine == ContainerEngine::Docker {
                "{{json .ServerVersion}}".into()
            } else {
                "json".into()
            },
        ],
        8,
    )
    .await;
    match result {
        Ok(detail) => Capability {
            engine,
            available: true,
            detail: if engine == ContainerEngine::Docker {
                detail.trim().to_owned()
            } else {
                "Podman engine responded; execution profile is experimental".into()
            },
            experimental: engine == ContainerEngine::Podman,
        },
        Err(error) => Capability {
            engine,
            available: false,
            detail: format!("{error:#}"),
            experimental: engine == ContainerEngine::Podman,
        },
    }
}

#[derive(Clone)]
pub struct ContainerProvider {
    installation: String,
}
impl ContainerProvider {
    pub fn new(installation: String) -> Result<Self> {
        uuid::Uuid::parse_str(&installation)?;
        Ok(Self { installation })
    }
    pub fn resource_name(&self, owner: &str) -> String {
        format!(
            "dockon-{}-{}",
            self.installation.replace('-', ""),
            project_key(owner)
        )
    }
    fn labels(&self, project: &str, owner: &str) -> Vec<String> {
        vec![
            "--label".into(),
            format!("{INSTALLATION}={}", self.installation),
            "--label".into(),
            format!("{PROJECT}={project}"),
            "--label".into(),
            format!("{INSTANCE}={owner}"),
        ]
    }
    async fn inspect(&self, engine: ContainerEngine, kind: &str, id: &str) -> Result<Value> {
        let output = execute(engine, &[kind.into(), "inspect".into(), id.into()], 15).await?;
        let value: Value = serde_json::from_str(&output)?;
        value
            .as_array()
            .and_then(|v| v.first())
            .cloned()
            .context("Invalid provider inspect response")
    }
    fn check_labels(&self, labels: &Value, project: &str, owner: &str) -> Result<()> {
        if labels[INSTALLATION].as_str() != Some(&self.installation)
            || labels[PROJECT].as_str() != Some(project)
            || labels[INSTANCE].as_str() != Some(owner)
        {
            bail!("Provider resource ownership mismatch");
        }
        Ok(())
    }
    async fn ensure_resource(
        &self,
        engine: ContainerEngine,
        kind: &str,
        name: &str,
        project: &str,
        owner: &str,
    ) -> Result<()> {
        match self.inspect(engine, kind, name).await {
            Ok(value) => self.check_labels(&value["Labels"], project, owner),
            Err(_) => {
                let mut args = vec![kind.into(), "create".into()];
                args.extend(self.labels(project, owner));
                args.push(name.into());
                execute(engine, &args, 30).await?;
                self.check_labels(
                    &self.inspect(engine, kind, name).await?["Labels"],
                    project,
                    owner,
                )
            }
        }
    }
    pub async fn start(
        &self,
        project: &str,
        owner: &str,
        alias: &str,
        spec: &ContainerSpec,
        port: u16,
        environment: &std::collections::BTreeMap<String, String>,
    ) -> Result<ContainerInstance> {
        spec.validate()?;
        let resource = self.resource_name(owner);
        let network = self.resource_name(project);
        self.ensure_resource(spec.engine, "network", &network, project, project)
            .await?;
        self.ensure_resource(spec.engine, "volume", &resource, project, owner)
            .await?;
        // Pull by immutable digest. No tags, host mounts, engine socket, host
        // networking or arbitrary engine options are exposed to manifests.
        if self
            .inspect(spec.engine, "image", &spec.image)
            .await
            .is_err()
        {
            execute(spec.engine, &["pull".into(), spec.image.clone()], 180).await?;
        }
        let mut args = vec![
            "container".into(),
            "create".into(),
            "--name".into(),
            format!("{resource}-{}", uuid::Uuid::new_v4().simple()),
            "--network".into(),
            network,
            "--network-alias".into(),
            alias.into(),
            "--mount".into(),
            format!("type=volume,source={resource},target=/dockon-data"),
            "--publish".into(),
            format!("127.0.0.1:{port}:{}", spec.internal_port),
            "--cap-drop".into(),
            "ALL".into(),
            "--security-opt".into(),
            if spec.engine == ContainerEngine::Docker {
                "no-new-privileges:true".into()
            } else {
                "no-new-privileges".into()
            },
            "--read-only".into(),
            "--pids-limit".into(),
            "256".into(),
            "--log-opt".into(),
            "max-size=5m".into(),
        ];
        if spec.engine == ContainerEngine::Docker {
            args.extend(["--log-opt".into(), "max-file=2".into()]);
        }
        args.extend([
            "--env".into(),
            format!("PORT={}", spec.internal_port),
            "--env".into(),
            "HOST=0.0.0.0".into(),
        ]);
        for (key, value) in environment {
            args.push("--env".into());
            args.push(format!("{key}={value}"));
        }
        if let Some(entrypoint) = &spec.entrypoint {
            args.extend(["--entrypoint".into(), entrypoint.clone()]);
        }
        args.extend(self.labels(project, owner));
        args.push(spec.image.clone());
        args.extend(spec.command.clone());
        let id = execute(spec.engine, &args, 30).await?.trim().to_owned();
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("Invalid container ID from provider");
        }
        let mut instance = ContainerInstance {
            id,
            owner: owner.into(),
            project: project.into(),
            engine: spec.engine,
            provider: self.clone(),
        };
        let value = self.inspect(spec.engine, "container", &instance.id).await?;
        self.check_labels(&value["Config"]["Labels"], project, owner)?;
        if let Err(error) = execute(
            spec.engine,
            &["container".into(), "start".into(), instance.id.clone()],
            30,
        )
        .await
        {
            let _ = instance.stop(owner).await;
            return Err(error);
        }
        Ok(instance)
    }
    pub async fn reconcile(&self, engine: ContainerEngine, project: &str) -> Result<usize> {
        let ids = execute(
            engine,
            &[
                "container".into(),
                "ls".into(),
                "--all".into(),
                "--quiet".into(),
                "--no-trunc".into(),
                "--filter".into(),
                format!("label={INSTALLATION}={}", self.installation),
                "--filter".into(),
                format!("label={PROJECT}={project}"),
            ],
            15,
        )
        .await?;
        let mut count = 0;
        for id in ids.lines().filter(|s| !s.is_empty()) {
            let value = self.inspect(engine, "container", id).await?;
            let owner = value["Config"]["Labels"][INSTANCE]
                .as_str()
                .context("Missing instance owner label")?
                .to_owned();
            let mut instance = ContainerInstance {
                id: id.into(),
                owner: owner.clone(),
                project: project.into(),
                engine,
                provider: self.clone(),
            };
            instance.stop(&owner).await?;
            count += 1;
        }
        Ok(count)
    }
}

pub struct ContainerInstance {
    pub id: String,
    pub owner: String,
    pub project: String,
    pub engine: ContainerEngine,
    provider: ContainerProvider,
}
impl ContainerInstance {
    pub async fn running(&self) -> Result<bool> {
        let value = self
            .provider
            .inspect(self.engine, "container", &self.id)
            .await?;
        self.provider
            .check_labels(&value["Config"]["Labels"], &self.project, &self.owner)?;
        Ok(value["State"]["Running"].as_bool().unwrap_or(false))
    }
    pub async fn stop(&mut self, owner: &str) -> Result<()> {
        if owner != self.owner {
            bail!("Container ownership mismatch");
        }
        let value = self
            .provider
            .inspect(self.engine, "container", &self.id)
            .await?;
        self.provider
            .check_labels(&value["Config"]["Labels"], &self.project, owner)?;
        if value["State"]["Running"].as_bool() == Some(true) {
            execute(
                self.engine,
                &[
                    "container".into(),
                    "stop".into(),
                    "--time".into(),
                    "15".into(),
                    self.id.clone(),
                ],
                25,
            )
            .await?;
        }
        execute(
            self.engine,
            &["container".into(), "rm".into(), self.id.clone()],
            15,
        )
        .await?;
        // Persistent volumes are deliberately never removed by lifecycle stop.
        Ok(())
    }
    pub async fn logs(&self) -> Result<String> {
        execute(
            self.engine,
            &[
                "container".into(),
                "logs".into(),
                "--tail".into(),
                "100".into(),
                self.id.clone(),
            ],
            10,
        )
        .await
    }
}

fn local_docker_endpoint(endpoint: &str) -> bool {
    endpoint.starts_with("unix:///") || endpoint.starts_with("npipe:////./pipe/")
}
async fn execute(engine: ContainerEngine, args: &[String], timeout: u64) -> Result<String> {
    if engine == ContainerEngine::Docker {
        let endpoint = if std::env::var_os("DOCKER_CONTEXT").is_none() {
            match std::env::var("DOCKER_HOST") {
                Ok(value) => value,
                Err(_) => execute_raw(
                    engine,
                    &[
                        "context".into(),
                        "inspect".into(),
                        "--format".into(),
                        "{{.Endpoints.docker.Host}}".into(),
                    ],
                    8,
                    false,
                )
                .await?
                .trim()
                .to_owned(),
            }
        } else {
            execute_raw(
                engine,
                &[
                    "context".into(),
                    "inspect".into(),
                    "--format".into(),
                    "{{.Endpoints.docker.Host}}".into(),
                ],
                8,
                false,
            )
            .await?
            .trim()
            .to_owned()
        };
        if !local_docker_endpoint(&endpoint) {
            bail!("Remote/TCP Docker endpoints are not supported by this local provider");
        }
        let mut scoped = vec!["--host".into(), endpoint];
        scoped.extend_from_slice(args);
        return execute_raw(engine, &scoped, timeout, true).await;
    }
    if std::env::var_os("CONTAINER_CONNECTION").is_some()
        || std::env::var("CONTAINER_HOST").is_ok_and(|host| !host.starts_with("unix:///"))
    {
        bail!("Remote Podman connections are not supported by this local provider");
    }
    execute_raw(engine, args, timeout, false).await
}
async fn execute_raw(
    engine: ContainerEngine,
    args: &[String],
    timeout: u64,
    scoped: bool,
) -> Result<String> {
    let mut command = Command::new(engine.executable());
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if scoped {
        command
            .env_remove("DOCKER_CONTEXT")
            .env_remove("DOCKER_HOST")
            .env_remove("DOCKER_TLS_VERIFY")
            .env_remove("DOCKER_CERT_PATH");
    }
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let (stdout, stderr, status) = tokio::time::timeout(Duration::from_secs(timeout), async {
        let mut child = command.spawn().context("Container provider unavailable")?;
        let stdout = child.stdout.take().context("Provider stdout unavailable")?;
        let stderr = child.stderr.take().context("Provider stderr unavailable")?;
        let (stdout, stderr, status) =
            tokio::try_join!(read_limited(stdout), read_limited(stderr), async {
                Ok::<_, anyhow::Error>(child.wait().await?)
            })?;
        Ok::<_, anyhow::Error>((stdout, stderr, status))
    })
    .await
    .context("Container provider command timed out")??;
    if !status.success() {
        bail!(
            "{}: {}",
            engine.executable(),
            String::from_utf8_lossy(&stderr)
                .chars()
                .take(2000)
                .collect::<String>()
        );
    }
    Ok(String::from_utf8(stdout)?)
}

async fn read_limited(stream: impl tokio::io::AsyncRead + Unpin) -> Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut bytes = vec![];
    stream
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > 8 * 1024 * 1024 {
        bail!("Provider output exceeds 8 MiB");
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_remote_engines_and_remote_named_pipes() {
        assert!(local_docker_endpoint("unix:///var/run/docker.sock"));
        assert!(local_docker_endpoint("npipe:////./pipe/docker_engine"));
        assert!(!local_docker_endpoint("ssh://server"));
        assert!(!local_docker_endpoint("tcp://127.0.0.1:2375"));
        assert!(!local_docker_endpoint(
            "npipe:////remote/pipe/docker_engine"
        ));
    }
    #[test]
    fn ownership_and_case_sensitive_resource_names() {
        let provider = ContainerProvider::new(uuid::Uuid::new_v4().to_string()).unwrap();
        assert_ne!(
            provider.resource_name("Project"),
            provider.resource_name("project")
        );
        assert!(provider
            .check_labels(&serde_json::json!({}), "project", "project")
            .is_err());
    }
}
