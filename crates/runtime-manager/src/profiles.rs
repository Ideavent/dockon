use crate::{hash_file, RuntimeManager};
use anyhow::{bail, Context, Result};
use dockon_core::{LaunchArg, Manifest};
use std::{collections::BTreeMap, path::Path};

impl RuntimeManager {
    pub async fn prepare(
        &self,
        manifest: &Manifest,
        data: &Path,
        environment: &BTreeMap<String, String>,
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
        let paths = std::iter::once(bin.to_path_buf())
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
        let set = |env: &mut BTreeMap<String, String>, key: &str, path: &Path| {
            env.insert(key.into(), path.to_string_lossy().into());
        };
        match record.runtime.as_str() {
            "apache" if cfg!(windows) => {
                if !manifest.args.is_empty() {
                    bail!("Apache static projects use generated configuration");
                }
                let server_root = bin
                    .parent()
                    .context("Apache installation root unavailable")?;
                let public = dunce::canonicalize(manifest.root.join("public"))?;
                if !public.starts_with(dunce::canonicalize(&manifest.root)?) {
                    bail!("Apache public directory escapes the project root");
                }
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
                std::fs::write(
                    &config,
                    format!(
                        r#"ServerRoot {server_root}
ServerName localhost
LoadModule authz_core_module modules/mod_authz_core.so
LoadModule dir_module modules/mod_dir.so
LoadModule mime_module modules/mod_mime.so
TypesConfig conf/mime.types
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
    DirectoryIndex index.html
    <FilesMatch "(?i)^(?![a-z0-9_-][a-z0-9_.-]*\.(?:html|css|js|png|jpe?g|gif|svg|ico|webp)$).+">
        Require all denied
    </FilesMatch>
</Directory>
"#
                    ),
                )?;
                let config = config
                    .to_str()
                    .context("Invalid Apache config path")?
                    .replace('\\', "/");
                manifest.args = vec![
                    "-X".into(),
                    "-f".into(),
                    config.into(),
                    "-C".into(),
                    LaunchArg::template("Listen 127.0.0.1:${DOCKON_PORT}"),
                ];
            }
            "node" => {
                set(&mut environment, "npm_config_cache", &cache.join("npm"));
            }
            "php" => {
                let ini = data.join("php.ini");
                let extension = record.root.join("ext").to_string_lossy().replace('\\', "/");
                let private = data.to_string_lossy().replace('\\', "/");
                if extension.contains(['\n', '\r', '"']) || private.contains(['\n', '\r', '"']) {
                    bail!("PHP configuration path cannot contain quotes or line breaks");
                }
                std::fs::write(&ini,format!("; Generated isolated Dockon PHP configuration\n[PHP]\nextension_dir=\"{extension}\"\nsession.save_path=\"{private}\"\nupload_tmp_dir=\"{private}\"\nsys_temp_dir=\"{private}\"\ndisplay_errors=On\nlog_errors=On\n"))?;
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
                    let venv = data.join("venv");
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
            _ => bail!("This runtime cannot launch a native project profile"),
        }
        Ok((manifest, environment))
    }
}
