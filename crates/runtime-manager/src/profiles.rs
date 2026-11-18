use crate::{hash_file, RuntimeManager};
use anyhow::{bail, Context, Result};
use dockon_core::{LaunchArg, Manifest};
use std::{collections::BTreeMap, path::Path};

fn set(env: &mut BTreeMap<String, String>, key: &str, path: &Path) {
    env.insert(key.into(), path.to_string_lossy().into());
}

impl RuntimeManager {
    pub async fn tool_environment(
        &self,
        manifest: &Manifest,
    ) -> Result<(Vec<std::path::PathBuf>, BTreeMap<String, String>)> {
        let mut paths = Vec::new();
        let mut env = BTreeMap::new();
        let tools = &manifest.tools;
        let android = if let Some(id) = &tools.android_tools {
            let selected = self.selected(id).await?;
            if selected.runtime != "android-tools" {
                bail!("Select installed Android command-line tools");
            }
            set(
                &mut env,
                "DOCKON_SDKMANAGER",
                dunce::simplified(&selected.executable),
            );
            let managed = self.root.join("tool-state/android-sdk");
            let path = tools.android_sdk.clone().unwrap_or(managed);
            if tools.android_sdk.is_none() {
                std::fs::create_dir_all(&path)?;
            }
            Some(path)
        } else {
            tools.android_sdk.clone()
        };
        if let Some(path) = &android {
            if !path.is_absolute()
                || !path.is_dir()
                || (tools.android_tools.is_none()
                    && !(path.join("platform-tools").is_dir()
                        || path.join("cmdline-tools").is_dir()))
            {
                bail!("Select a valid Android SDK directory");
            }
            set(&mut env, "ANDROID_HOME", path);
            set(&mut env, "ANDROID_SDK_ROOT", path);
            for dir in ["platform-tools", "cmdline-tools/latest/bin", "emulator"] {
                let dir = path.join(dir);
                if dir.is_dir() {
                    paths.push(dir);
                }
            }
        }
        let primary = match &manifest.runtime {
            Some(binding) => Some(self.selected(&binding.installation_id).await?),
            None => None,
        };
        let java = if let Some(id) = &tools.java {
            let java = self.selected(id).await?;
            if java.runtime != "java" {
                bail!("Select an installed Java JDK");
            }
            Some(java)
        } else {
            primary
                .as_ref()
                .filter(|record| record.runtime == "java")
                .cloned()
        };
        if let Some(java) = java {
            let bin = java
                .executable
                .parent()
                .context("Java binary path unavailable")?;
            let home = bin.parent().context("Java home unavailable")?;
            paths.push(bin.to_path_buf());
            set(&mut env, "JAVA_HOME", home);
        }
        if let Some(id) = &tools.composer {
            let composer = self.selected(id).await?;
            if composer.runtime != "composer" {
                bail!("Select an installed Composer version");
            }
            let cli = if primary
                .as_ref()
                .is_some_and(|record| record.runtime == "php")
            {
                primary.unwrap().executable
            } else {
                let binding = manifest
                    .services
                    .iter()
                    .find(|service| service.id == "php-cgi")
                    .and_then(|service| service.runtime.as_ref())
                    .context("Composer requires a PHP project")?;
                let record = self.selected(&binding.installation_id).await?;
                record
                    .executable
                    .with_file_name(if cfg!(windows) { "php.exe" } else { "php" })
            };
            if !cli.is_file() {
                bail!("The selected PHP build does not include its CLI");
            }
            set(
                &mut env,
                "DOCKON_COMPOSER",
                dunce::simplified(&composer.executable),
            );
            set(&mut env, "DOCKON_COMPOSER_PHP", dunce::simplified(&cli));
            if let Some(root) = cli
                .parent()
                .map(dunce::simplified)
                .filter(|root| root.join("ext").is_dir())
            {
                let ini = self
                    .root
                    .join("tool-state")
                    .join(&manifest.id)
                    .join("composer-php.ini");
                let extension_dir = root.join("ext").to_string_lossy().replace('\\', "/");
                if extension_dir.contains(['"', '\r', '\n']) {
                    bail!("Invalid PHP extension path");
                }
                std::fs::create_dir_all(ini.parent().context("PHP tool state missing")?)?;
                let mut text = format!("extension_dir=\"{extension_dir}\"\n");
                for extension in crate::projects::php_extensions(root) {
                    text.push_str(&format!("extension={extension}\n"));
                }
                std::fs::write(&ini, text)?;
                set(&mut env, "DOCKON_COMPOSER_INI", &ini);
            }
            set(
                &mut env,
                "COMPOSER_HOME",
                &self
                    .root
                    .join("tool-state")
                    .join(&manifest.id)
                    .join("composer"),
            );
            set(
                &mut env,
                "COMPOSER_CACHE_DIR",
                &self.root.join("packages/composer-cache"),
            );
        }
        Ok((paths, env))
    }

    pub async fn prepare(
        &self,
        manifest: &Manifest,
        data: &Path,
        environment: &BTreeMap<String, String>,
        port: u16,
    ) -> Result<(Manifest, BTreeMap<String, String>)> {
        let mut manifest = manifest.clone();
        let mut environment = environment.clone();
        let Some(binding) = &manifest.runtime else {
            return Ok((manifest, environment));
        };
        let record = self.selected(&binding.installation_id).await?;
        if dunce::canonicalize(&manifest.executable)? != dunce::canonicalize(&record.executable)? {
            bail!("Manifest executable differs from the selected runtime");
        }
        let bin = record
            .executable
            .parent()
            .context("Runtime binary directory unavailable")?;
        let (tool_paths, tools) = self.tool_environment(&manifest).await?;
        environment.extend(tools);
        let paths = std::iter::once(bin.to_path_buf())
            .chain(tool_paths)
            .chain(std::env::split_paths(
                &std::env::var_os("PATH").unwrap_or_default(),
            ))
            .collect::<Vec<_>>();
        environment.insert(
            "PATH".into(),
            std::env::join_paths(paths)?.to_string_lossy().into(),
        );
        let cache = data.join("cache");
        std::fs::create_dir_all(&cache)?;
        match record.runtime.as_str() {
            "nginx" => {
                if !manifest.args.is_empty() {
                    bail!("Nginx static projects use generated configuration");
                }
                let public = web_public(&manifest.root)?;
                let backend = php_backend(&environment)?;
                let public = web_path(&public)?;
                let config = data.join("nginx.conf");
                let prefix = format!("{}/", web_path(data)?);
                let php = match backend {
                    Some(backend) => format!(
                        r#"
        location / {{ rewrite ^ /index.php last; }}
        location = /index.php {{
            fastcgi_pass 127.0.0.1:{backend};
            fastcgi_param SCRIPT_FILENAME "{public}/index.php";
            fastcgi_param DOCUMENT_ROOT "{public}";
            fastcgi_param SCRIPT_NAME /index.php;
            fastcgi_param REQUEST_URI $request_uri;
            fastcgi_param REQUEST_METHOD $request_method;
            fastcgi_param QUERY_STRING $query_string;
            fastcgi_param CONTENT_TYPE $content_type;
            fastcgi_param CONTENT_LENGTH $content_length;
            fastcgi_param SERVER_PROTOCOL $server_protocol;
            fastcgi_param SERVER_NAME $server_name;
            fastcgi_param SERVER_PORT $server_port;
        }}
"#
                    ),
                    None => "location = / { rewrite ^ /index.html last; }".into(),
                };
                std::fs::write(
                    &config,
                    format!(
                        r#"daemon off;
worker_processes 1;
pid nginx.pid;
error_log stderr;
events {{ worker_connections 128; }}
http {{
    access_log off;
    types {{
        text/html html;
        text/css css;
        application/javascript js;
        image/png png;
        image/jpeg jpeg jpg;
        image/gif gif;
        image/svg+xml svg;
        image/x-icon ico;
        image/webp webp;
    }}
    client_body_temp_path client_body;
    proxy_temp_path proxy;
    fastcgi_temp_path fastcgi;
    uwsgi_temp_path uwsgi;
    scgi_temp_path scgi;
    server {{
        listen 127.0.0.1:{port};
        root "{public}";
        location ~ "(?i)^/(?:[a-z0-9_-][a-z0-9_.-]*/)*[a-z0-9_-][a-z0-9_.-]*\.(html|css|js|png|jpe?g|gif|svg|ico|webp)$" {{
            {symlink_policy}
        }}
        {php}
        location / {{ return 403; }}
    }}
}}
"#,
                        symlink_policy = if cfg!(windows) {
                            ""
                        } else {
                            "disable_symlinks on;"
                        }
                    ),
                )?;
                let config = web_path(&config)?;
                let args = ["-p", &prefix, "-c", &config, "-e", "stderr"];
                validate_web_config(&record.executable, data, &args, &["-t"]).await?;
                manifest.args = args.into_iter().map(LaunchArg::from).collect();
            }
            "apache" if cfg!(windows) => {
                if !manifest.args.is_empty() {
                    bail!("Apache static projects use generated configuration");
                }
                let server_root = bin
                    .parent()
                    .context("Apache installation root unavailable")?;
                let public = web_public(&manifest.root)?;
                let backend = php_backend(&environment)?;
                let quote = |path: &Path| -> Result<String> {
                    let path = path.to_str().context("Apache requires UTF-8 paths")?;
                    if path.contains(['"', '\n', '\r', '$']) {
                        bail!("Unsupported character in Apache configuration path");
                    }
                    Ok(format!("\"{}\"", path.replace('\\', "/")))
                };
                let server_root = quote(server_root)?;
                let public = quote(&public)?;
                let pid = quote(&data.join("httpd.pid"))?;
                let error_log = quote(&data.join("httpd-error.log"))?;
                let config = data.join("httpd.conf");
                let (proxy_modules, index, handler) = match backend {
                    Some(backend) => (
                        "LoadModule proxy_module modules/mod_proxy.so\nLoadModule proxy_fcgi_module modules/mod_proxy_fcgi.so\nProxyRequests Off\nProxyFCGIBackendType GENERIC",
                        "index.php",
                        format!("<LocationMatch \"^/index[.]php$\">\n    Require all granted\n    SetHandler \"proxy:fcgi://127.0.0.1:{backend}/\"\n    ProxyFCGISetEnvIf \"true\" SCRIPT_FILENAME \"{}/index.php\"\n</LocationMatch>", public.trim_matches('"')),
                    ),
                    None => ("", "index.html", String::new()),
                };
                let fallback = if backend.is_some() {
                    "FallbackResource /index.php"
                } else {
                    ""
                };
                std::fs::write(
                    &config,
                    format!(
                        r#"ServerRoot {server_root}
ServerName localhost
LoadModule authz_core_module modules/mod_authz_core.so
LoadModule dir_module modules/mod_dir.so
LoadModule mime_module modules/mod_mime.so
TypesConfig conf/mime.types
{proxy_modules}
PidFile {pid}
ErrorLog {error_log}
DocumentRoot {public}
<Directory />
    Require all denied
</Directory>
<Directory {public}>
    Options None
    AllowOverride None
    Require all granted
    DirectoryIndex {index}
    {fallback}
    <FilesMatch "(?i)^(?![a-z0-9_-][a-z0-9_.-]*\.(?:html|css|js|png|jpe?g|gif|svg|ico|webp)$).+">
        Require all denied
    </FilesMatch>
</Directory>
{handler}
"#
                    ),
                )?;
                let config = config
                    .to_str()
                    .context("Invalid Apache config path")?
                    .replace('\\', "/");
                let listen = format!("Listen 127.0.0.1:{port}");
                let args = ["-f", &config, "-C", &listen];
                validate_web_config(&record.executable, data, &args, &["-t"]).await?;
                manifest.args = args
                    .into_iter()
                    .chain(["-X"])
                    .map(LaunchArg::from)
                    .collect();
            }
            "node" => {
                set(&mut environment, "npm_config_cache", &cache.join("npm"));
            }
            "php" | "php-cgi" => {
                let ini = data.join("php.ini");
                let extension = record.root.join("ext").to_string_lossy().replace('\\', "/");
                let private = data.to_string_lossy().replace('\\', "/");
                if extension.contains(['\n', '\r', '"']) || private.contains(['\n', '\r', '"']) {
                    bail!("PHP configuration path cannot contain quotes or line breaks");
                }
                std::fs::write(&ini,format!("; Generated isolated Dockon PHP configuration\n[PHP]\nextension_dir=\"{extension}\"\nsession.save_path=\"{private}\"\nupload_tmp_dir=\"{private}\"\nsys_temp_dir=\"{private}\"\ndisplay_errors=On\nlog_errors=On\n"))?;
                let mut config = std::fs::OpenOptions::new().append(true).open(&ini)?;
                use std::io::Write;
                for extension in crate::projects::php_extensions(&record.root) {
                    writeln!(config, "extension={extension}")?;
                }
                manifest.args.splice(
                    0..0,
                    [
                        LaunchArg::from("-c"),
                        LaunchArg::from(ini.to_string_lossy().to_string()),
                    ],
                );
                set(
                    &mut environment,
                    "PHP_INI_SCAN_DIR",
                    &data.join("php-ini.d"),
                );
            }
            "dotnet" => {
                set(&mut environment, "DOTNET_ROOT", &record.root);
                set(&mut environment, "DOTNET_CLI_HOME", data);
                set(&mut environment, "NUGET_PACKAGES", &cache.join("nuget"));
                for (key, value) in [
                    ("DOTNET_CLI_TELEMETRY_OPTOUT", "1"),
                    ("DOTNET_NOLOGO", "1"),
                    ("DOTNET_MULTILEVEL_LOOKUP", "0"),
                    ("DOTNET_GENERATE_ASPNET_CERTIFICATE", "false"),
                ] {
                    environment.insert(key.into(), value.into());
                }
            }
            "go" => {
                set(&mut environment, "GOROOT", &record.root);
                set(&mut environment, "GOPATH", &data.join("go"));
                set(&mut environment, "GOCACHE", &cache.join("go-build"));
                set(&mut environment, "GOMODCACHE", &cache.join("go-mod"));
                environment.insert("GOTOOLCHAIN".into(), "local".into());
                environment.insert("GOPROXY".into(), "off".into());
                environment.insert("GOENV".into(), "off".into());
            }
            "rust" => {
                let rustc = bin.join(if cfg!(windows) { "rustc.exe" } else { "rustc" });
                if !rustc.is_file() || hash_file(&rustc, "sha256")? == record.executable_hash {
                    bail!("Select Cargo from a concrete installed toolchain, not a rustup shim");
                }
                set(&mut environment, "RUSTC", &rustc);
                set(&mut environment, "CARGO_HOME", &data.join("cargo-home"));
                set(
                    &mut environment,
                    "CARGO_TARGET_DIR",
                    &data.join("cargo-target"),
                );
            }
            "python" => {
                set(&mut environment, "PIP_CACHE_DIR", &cache.join("pip"));
                set(
                    &mut environment,
                    "PYTHONPYCACHEPREFIX",
                    &cache.join("python"),
                );
                environment.insert("PYTHONNOUSERSITE".into(), "1".into());
                if binding.isolated_python {
                    let legacy = data.join("venv");
                    let venv = if std::fs::read_to_string(legacy.join("dockon-base.sha256"))
                        .ok()
                        .as_deref()
                        == Some(&record.executable_hash)
                    {
                        legacy
                    } else {
                        data.join("venvs").join(&record.executable_hash)
                    };
                    std::fs::create_dir_all(
                        venv.parent()
                            .context("Python environment directory unavailable")?,
                    )?;
                    let marker = venv.join("dockon-base.sha256");
                    if venv.exists() {
                        if std::fs::read_to_string(&marker).ok().as_deref()
                            != Some(&record.executable_hash)
                        {
                            bail!("Existing Python environment uses a different base interpreter; explicit environment migration is required");
                        }
                    } else {
                        let mut command = tokio::process::Command::new(&record.executable);
                        command
                            .args(["-m", "venv"])
                            .arg(&venv)
                            .env("PYTHONNOUSERSITE", "1")
                            .envs(&environment)
                            .env_remove("PYTHONHOME")
                            .env_remove("PYTHONPATH")
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .kill_on_drop(true);
                        #[cfg(windows)]
                        command.creation_flags(0x08000000);
                        if !tokio::time::timeout(
                            std::time::Duration::from_secs(60),
                            command.status(),
                        )
                        .await
                        .context("Python environment creation timed out")??
                        .success()
                        {
                            bail!("Python venv creation failed; check that the selected interpreter includes venv support");
                        }
                        std::fs::write(&marker, &record.executable_hash)?;
                    }
                    manifest.executable = venv.join(if cfg!(windows) {
                        "Scripts/python.exe"
                    } else {
                        "bin/python"
                    });
                    if matches!(manifest.template.as_deref(), Some("django" | "flask")) {
                        let requirements = manifest.root.join(".dockon-python-requirements.txt");
                        let marker = venv.join("dockon-requirements.sha256");
                        let digest = hash_file(&requirements, "sha256")?;
                        if std::fs::read_to_string(&marker).ok().as_deref() != Some(&digest) {
                            let mut pip = tokio::process::Command::new(&manifest.executable);
                            pip.current_dir(&manifest.root)
                                .envs(&environment)
                                .env_remove("PYTHONHOME")
                                .env_remove("PYTHONPATH")
                                .args([
                                    "-m",
                                    "pip",
                                    "install",
                                    "--disable-pip-version-check",
                                    "--no-input",
                                    "-r",
                                ])
                                .arg(&requirements)
                                .stdin(std::process::Stdio::null())
                                .kill_on_drop(true);
                            #[cfg(windows)]
                            pip.creation_flags(0x08000000);
                            let output = tokio::time::timeout(
                                std::time::Duration::from_secs(240),
                                pip.output(),
                            )
                            .await
                            .context("Installing Python template dependencies timed out")??;
                            if !output.status.success() {
                                let details =
                                    &output.stderr[output.stderr.len().saturating_sub(4000)..];
                                bail!(
                                    "Installing Python template dependencies failed: {}",
                                    String::from_utf8_lossy(details)
                                );
                            }
                            std::fs::write(marker, digest)?;
                        }
                    }
                    set(&mut environment, "VIRTUAL_ENV", &venv);
                    let paths = std::iter::once(
                        manifest
                            .executable
                            .parent()
                            .context("Python venv binary directory unavailable")?
                            .to_path_buf(),
                    )
                    .chain(std::env::split_paths(std::ffi::OsStr::new(
                        environment
                            .get("PATH")
                            .map(String::as_str)
                            .unwrap_or_default(),
                    )))
                    .collect::<Vec<_>>();
                    environment.insert(
                        "PATH".into(),
                        std::env::join_paths(paths)?.to_string_lossy().into(),
                    );
                }
            }
            "flutter" => {
                manifest.executable = dunce::canonicalize(&record.executable)?;
                set(
                    &mut environment,
                    "PUB_CACHE",
                    &self.root.join("packages/pub-cache"),
                );
                set(&mut environment, "FLUTTER_ROOT", &record.root);
                environment.insert("FLUTTER_SUPPRESS_ANALYTICS".into(), "true".into());
            }
            "java" => {
                if manifest.template.as_deref() == Some("spring-boot") {
                    let wrapper = dunce::canonicalize(manifest.root.join(if cfg!(windows) {
                        "mvnw.cmd"
                    } else {
                        "mvnw"
                    }))?;
                    if !wrapper.starts_with(dunce::canonicalize(&manifest.root)?)
                        || !wrapper.is_file()
                    {
                        bail!("Maven wrapper must be inside the project");
                    }
                    manifest.executable = wrapper;
                    let cache = self.root.join("packages/maven-cache");
                    set(&mut environment, "MAVEN_USER_HOME", &cache);
                    manifest.args.insert(
                        0,
                        format!("-Dmaven.repo.local={}", cache.join("repository").display()).into(),
                    );
                }
            }
            "redis" | "memcached" => {}
            _ => bail!("This runtime cannot launch a native project profile"),
        }
        Ok((manifest, environment))
    }
}

fn web_public(root: &Path) -> Result<std::path::PathBuf> {
    let root = dunce::canonicalize(root)?;
    let folder = if root.join("webroot/index.php").is_file() && !root.join("public").exists() {
        "webroot"
    } else {
        "public"
    };
    let public = dunce::canonicalize(root.join(folder))?;
    if !public.is_dir() || !public.starts_with(&root) || public == root {
        bail!("The public directory must be inside the project root");
    }
    #[cfg(windows)]
    reject_windows_links(&public)?;
    Ok(public)
}

fn php_backend(environment: &BTreeMap<String, String>) -> Result<Option<u16>> {
    let Some(services) = environment.get("DOCKON_SERVICES") else {
        return Ok(None);
    };
    let services: serde_json::Value = serde_json::from_str(services)?;
    let Some(endpoint) = services.get("php-cgi") else {
        return Ok(None);
    };
    let port = endpoint["port"]
        .as_u64()
        .and_then(|port| u16::try_from(port).ok());
    match (endpoint["host"].as_str(), port) {
        (Some("127.0.0.1"), Some(port)) if port != 0 => Ok(Some(port)),
        _ => bail!("PHP CGI must be a loopback project dependency"),
    }
}

fn web_path(path: &Path) -> Result<String> {
    let path = path.to_str().context("Web server requires UTF-8 paths")?;
    if path.contains(['"', '\n', '\r', '$']) {
        bail!("Unsupported character in web server configuration path");
    }
    Ok(path.replace('\\', "/"))
}

#[cfg(windows)]
fn reject_windows_links(public: &Path) -> Result<()> {
    use std::os::windows::fs::MetadataExt;
    let mut directories = vec![public.to_path_buf()];
    let mut entries = 0;
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory)? {
            entries += 1;
            if entries > 100_000 {
                bail!("Nginx public directory exceeds the Windows file inspection limit");
            }
            let entry = entry?;
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if metadata.file_attributes() & 0x400 != 0 {
                bail!("Nginx public directories on Windows cannot contain filesystem links");
            }
            if metadata.is_dir() {
                directories.push(entry.path());
            }
        }
    }
    Ok(())
}

async fn validate_web_config(
    executable: &Path,
    directory: &Path,
    args: &[&str],
    validation_args: &[&str],
) -> Result<()> {
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;
    let mut command = tokio::process::Command::new(executable);
    command
        .args(args)
        .args(validation_args)
        .current_dir(directory)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let (status, errors) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut child = command.spawn()?;
        let stderr = child
            .stderr
            .take()
            .context("Web server stderr unavailable")?;
        let mut errors = Vec::new();
        stderr.take(8193).read_to_end(&mut errors).await?;
        if errors.len() > 8192 {
            bail!("Web server configuration output exceeds the size limit");
        }
        Ok::<_, anyhow::Error>((child.wait().await?, errors))
    })
    .await
    .context("Web server configuration validation timed out")??;
    if !status.success() {
        let diagnostic = String::from_utf8_lossy(&errors);
        let diagnostic: String = diagnostic.chars().take(1000).collect();
        bail!("Web server configuration rejected: {diagnostic}");
    }
    Ok(())
}
