use crate::{
    archive,
    projects::{attach_php_cgi, manifest, php_extensions, validate_identity},
    RuntimeManager,
};
use anyhow::{bail, Context, Result};
use dockon_core::{Manifest, ProjectTools, RuntimeInstallation};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::process::Command;

pub async fn create(
    manager: &RuntimeManager,
    id: &str,
    name: &str,
    root: &Path,
    installation_id: &str,
    cgi: Option<&str>,
    template: &str,
    tools: ProjectTools,
) -> Result<Manifest> {
    validate_identity(id, name)?;
    if !root.is_absolute() || root.exists() {
        bail!("New project directory must be absolute and must not already exist");
    }
    let parent = dunce::canonicalize(root.parent().context("Project parent missing")?)?;
    let installation = manager.selected(installation_id).await?;
    let stage = parent.join(format!(".dockon-create-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&stage)?;
    let source = stage.join("source");
    let result = async {
        let profile = match template {
            "laravel" | "codeigniter" | "cakephp" | "symfony" => {
                let package = match template {
                    "laravel" => "laravel/laravel",
                    "codeigniter" => "codeigniter4/appstarter",
                    "cakephp" => "cakephp/app",
                    _ => "symfony/skeleton",
                };
                let metadata: serde_json::Value = manager.http.json(&format!("https://repo.packagist.org/p2/{package}.json")).await?;
                let version = metadata["packages"][package].as_array().context("Framework package metadata missing")?.iter()
                    .filter_map(|release| {
                        let label = release["version"].as_str()?;
                        let version = semver::Version::parse(label.trim_start_matches('v')).ok()?;
                        version.pre.is_empty().then_some((version, label))
                    })
                    .max_by(|a,b| a.0.cmp_precedence(&b.0)).context("No stable framework template found")?.1;
                let composer = manager.selected(tools.composer.as_deref().context("Select an installed Composer version")?).await?;
                if composer.runtime != "composer" { bail!("Select an installed Composer version"); }
                let php = match cgi {
                    Some(id) if matches!(installation.runtime.as_str(), "apache" | "nginx") => manager.selected(id).await?,
                    None if installation.runtime == "php" => installation.clone(),
                    _ => bail!("Select PHP or a PHP CGI web profile"),
                };
                let executable = if php.runtime == "php-cgi" { php.executable.with_file_name(if cfg!(windows) { "php.exe" } else { "php" }) } else { php.executable.clone() };
                let mut command = command(&executable, &stage);
                let php_ini = stage.join("php.ini");
                if php.root.join("ext").is_dir() {
                    let ext = dunce::simplified(&php.root).join("ext").to_string_lossy().replace('\\', "/");
                    if ext.contains(['"', '\r', '\n']) { bail!("Invalid PHP extension path"); }
                    let mut config = format!("extension_dir=\"{ext}\"\n");
                    for extension in php_extensions(&php.root) { config.push_str(&format!("extension={extension}\n")); }
                    std::fs::write(&php_ini, config)?;
                    command.env("PHPRC", &php_ini);
                }
                if template == "laravel" {
                    command.env("DB_CONNECTION", "sqlite").env("DB_DATABASE", source.join("database/database.sqlite"))
                        .env_remove("DB_URL").env_remove("DATABASE_URL");
                }
                command.arg(&composer.executable).args(["create-project", "--no-interaction", "--prefer-dist", package])
                    .arg(&source).arg(version)
                    .env("COMPOSER_HOME", manager.root.join("tool-state").join(id).join("composer"))
                    .env("COMPOSER_CACHE_DIR", manager.root.join("packages/composer-cache"));
                run(command).await?;
                if template == "laravel" {
                    let database = source.join("database/database.sqlite");
                    std::fs::OpenOptions::new().create(true).append(true).open(&database)?;
                    let mut migrate = self::command(&executable, &source);
                    if php_ini.is_file() { migrate.env("PHPRC", &php_ini); }
                    migrate.args(["artisan", "migrate", "--force", "--no-interaction"])
                        .env("DB_CONNECTION", "sqlite").env("DB_DATABASE", database)
                        .env_remove("DB_URL").env_remove("DATABASE_URL");
                    run(migrate).await?;
                }
                if template == "cakephp" {
                    let config = source.join("config");
                    std::fs::rename(config.join("app_local.php"), config.join("app_local.starter.php"))?;
                    std::fs::write(config.join("app_local.php"), r"<?php
$config = require __DIR__ . '/app_local.starter.php';
$config['Datasources']['default']['driver'] = \Cake\Database\Driver\Sqlite::class;
$config['Datasources']['default']['database'] = dirname(__DIR__) . '/tmp/dockon.sqlite';
unset($config['Datasources']['default']['url']);
return $config;
")?;
                }
                if template == "symfony" {
                    let controllers = source.join("src/Controller");
                    std::fs::create_dir_all(&controllers)?;
                    std::fs::write(controllers.join("DockonController.php"), "<?php
namespace App\\Controller;
use Symfony\\Component\\HttpFoundation\\Response;
use Symfony\\Component\\Routing\\Attribute\\Route;
final class DockonController { #[Route('/')] public function index(): Response { return new Response('Dockon - Symfony'); } }
")?;
                    let routes = source.join("config/routes");
                    std::fs::create_dir_all(&routes)?;
                    std::fs::write(routes.join("dockon.yaml"), "dockon_home:\n  path: /\n  controller: App\\Controller\\DockonController::index\n")?;
                }
                if cgi.is_some() {
                    if installation.runtime == "apache" { "apache:php" } else { "nginx:php" }
                } else if template == "cakephp" { "php:cakephp" } else { "php:public" }
            }
            "vite-react" | "vite-vue" | "vite-svelte" if installation.runtime == "node" => {
                let npm = npm_cli(&installation)?;
                std::fs::create_dir(&source)?;
                let cache = manager.root.join("packages/npm-cache");
                let mut create = command(&installation.executable, &source);
                create.arg(&npm).args(["exec", "--yes", "--package=create-vite@latest", "--", "create-vite", ".", "--template", template.trim_start_matches("vite-"), "--no-interactive"])
                    .env("npm_config_cache", &cache);
                run(create).await?;
                let mut install = command(&installation.executable, &source);
                install.arg(npm).args(["install", "--include=dev", "--no-audit", "--no-fund"]).env("npm_config_cache", cache);
                run(install).await?;
                "node:vite"
            }
            "django" | "flask" if installation.runtime == "python" => {
                let package = if template == "django" { "Django" } else { "Flask" };
                let metadata: serde_json::Value = manager.http.json(&format!("https://pypi.org/pypi/{package}/json")).await?;
                let version = metadata["info"]["version"].as_str().context("Python template package version missing")?;
                if !version.bytes().all(|value| value.is_ascii_digit() || value == b'.') { bail!("No final Python template version found"); }
                let requirements = format!("{package}=={version}\n");
                std::fs::create_dir(&source)?;
                std::fs::write(source.join(".dockon-python-requirements.txt"), &requirements)?;
                std::fs::write(source.join("requirements.txt"), &requirements)?;
                if template == "django" {
                    let dependencies = stage.join("python-dependencies");
                    let mut install = command(&installation.executable, &stage);
                    install.args(["-m", "pip", "install", "--disable-pip-version-check", "--target"]).arg(&dependencies).arg(requirements.trim());
                    run(install).await?;
                    let mut django = command(&installation.executable, &source);
                    django.args(["-m", "django", "startproject", "dockon_app", "."]).env("PYTHONPATH", dependencies);
                    run(django).await?;
                    "python:django"
                } else {
                    std::fs::write(source.join("app.py"), "from flask import Flask\n\napp = Flask(__name__)\n\n@app.get('/')\ndef home():\n    return 'Dockon · Flask'\n")?;
                    "python:flask"
                }
            }
            "flutter" if installation.runtime == "flutter" => {
                let mut flutter = command(&installation.executable, &stage);
                flutter.args(["create", "--empty", "--platforms=android,web", "--project-name=dockon_app"]).arg(&source);
                flutter.env("FLUTTER_SUPPRESS_ANALYTICS", "true").env("PUB_CACHE", manager.root.join("packages/pub-cache"));
                run(flutter).await?;
                if !source.join("web/index.html").is_file() { bail!("Flutter did not generate a web target"); }
                "flutter:web"
            }
            "spring-boot" if installation.runtime == "java" => {
                let java = installation.version.split('.').next().context("JDK feature version missing")?;
                let url = format!("https://start.spring.io/starter.zip?type=maven-project&language=java&javaVersion={java}&dependencies=web&groupId=local.dockon&artifactId=app&packageName=local.dockon.app");
                let body = manager.http.bytes(&url, 8 * 1024 * 1024).await?;
                let zip = stage.join("spring.zip");
                std::fs::write(&zip, body)?;
                archive::extract(&zip, "zip", "", &source)?;
                std::fs::write(source.join("src/main/java/local/dockon/app/DockonHome.java"), "package local.dockon.app;\nimport org.springframework.web.bind.annotation.*;\n@RestController class DockonHome { @GetMapping(\"/\") String home() { return \"Dockon · Spring Boot\"; } }\n")?;
                "java:spring"
            }
            _ => bail!("Template and selected runtime do not match"),
        };
        let mut project = manifest(id, name, &dunce::canonicalize(&source)?, &installation, profile)?;
        attach_php_cgi(manager, &mut project, profile, cgi).await?;
        project.tools = tools;
        project.template = Some(template.into());
        manager.tool_environment(&project).await?;
        project.root = dunce::simplified(&parent).join(root.file_name().context("Missing project directory name")?);
        std::fs::write(source.join("dockon.json"), serde_json::to_vec_pretty(&project)?)?;
        if root.exists() { bail!("Project directory was created by another operation"); }
        std::fs::rename(&source, root)?;
        Ok(project)
    }.await;
    let _ = std::fs::remove_dir_all(&stage);
    result
}

fn npm_cli(installation: &RuntimeInstallation) -> Result<std::path::PathBuf> {
    let bin = installation
        .executable
        .parent()
        .context("Node binary directory unavailable")?;
    [
        bin.join("node_modules/npm/bin/npm-cli.js"),
        installation
            .root
            .join("lib/node_modules/npm/bin/npm-cli.js"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .context("Select a Node.js installation that includes npm")
}

fn command(executable: &Path, directory: &Path) -> Command {
    let mut command = Command::new(dunce::simplified(executable));
    if let Some(bin) = executable.parent() {
        let original = std::env::var_os("PATH").unwrap_or_default();
        if let Ok(path) = std::env::join_paths(
            std::iter::once(dunce::simplified(bin).to_path_buf())
                .chain(std::env::split_paths(&original)),
        ) {
            command.env("PATH", path);
        }
    }
    command
        .current_dir(directory)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .env_remove("NODE_OPTIONS")
        .env_remove("NODE_PATH")
        .env_remove("PYTHONPATH")
        .env_remove("PYTHONHOME")
        .env("CI", "1");
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
}

async fn run(mut command: Command) -> Result<()> {
    let output = tokio::time::timeout(Duration::from_secs(600), command.output())
        .await
        .context("Template setup timed out")??;
    if !output.status.success() {
        let details = if output.stderr.is_empty() {
            &output.stdout
        } else {
            &output.stderr
        };
        let start = details.len().saturating_sub(6000);
        bail!(
            "Template setup failed: {}",
            String::from_utf8_lossy(&details[start..])
        );
    }
    Ok(())
}
