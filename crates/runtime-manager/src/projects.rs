//! Detection, launch profiles, and minimal project creation.
use crate::RuntimeManager;
use anyhow::{bail, Context, Result};
use dockon_core::{HealthCheck, LaunchArg, Manifest, RuntimeInstallation, ServiceSpec};
use serde::Serialize;
use std::{
    io::{Read, Write},
    path::Path,
};

#[derive(Debug, Serialize)]
pub struct Detection {
    pub root: std::path::PathBuf,
    pub profiles: Vec<Profile>,
    pub versions: std::collections::BTreeMap<String, String>,
    pub dependency_files: Vec<String>,
    pub preset: Option<dockon_core::ProjectPreset>,
}
#[derive(Debug, Serialize)]
pub struct Profile {
    pub id: String,
    pub runtime: String,
    pub evidence: String,
}

fn read(path: &Path) -> Result<String> {
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_string(&mut text)?;
    if text.len() > 1024 * 1024 {
        bail!("Project metadata exceeds 1 MiB");
    }
    Ok(text)
}
fn child(root: &Path, name: &str) -> Result<std::path::PathBuf> {
    let value = dunce::canonicalize(root.join(name))?;
    if !value.starts_with(root) {
        bail!("Project entry point escapes the selected source directory");
    }
    Ok(value)
}
pub fn detect(root: &Path) -> Result<Detection> {
    if !root.is_absolute() {
        bail!("Select an absolute project directory");
    }
    let root = dunce::canonicalize(root)?;
    if !root.is_dir() {
        bail!("Project root is not a directory");
    }
    let mut profiles = vec![];
    let mut versions = std::collections::BTreeMap::new();
    let mut add = |id: &str, runtime: &str, evidence: &str| {
        profiles.push(Profile {
            id: id.into(),
            runtime: runtime.into(),
            evidence: evidence.into(),
        })
    };
    for entry in ["server.mjs", "server.js", "app.js", "index.js"] {
        if root.join(entry).is_file() {
            child(&root, entry)?;
            add(&format!("node:{entry}"), "node", entry);
        }
    }
    if let Ok(path) = child(&root, "package.json") {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&read(&path)?) {
            if let Some(version) = value["engines"]["node"].as_str().filter(|v| v.len() <= 100) {
                versions.insert("node".into(), version.into());
            }
            if value["devDependencies"]["vite"].is_string()
                || value["dependencies"]["vite"].is_string()
            {
                add("node:vite", "node", "package.json");
            }
        }
    }
    if let Ok(path) = child(&root, ".nvmrc") {
        let hint = read(&path)?;
        if !hint.trim().is_empty() && hint.trim().len() < 80 {
            versions.insert("node".into(), hint.trim().into());
        }
    }
    if let Ok(path) = child(&root, "composer.json") {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&read(&path)?) {
            if let Some(version) = value["require"]["php"].as_str().filter(|v| v.len() <= 100) {
                versions.insert("php".into(), version.into());
            }
        }
    }
    if let Ok(path) = child(&root, "pyproject.toml") {
        if let Some(hint) = read(&path)?
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once('=')?;
                (key.trim() == "requires-python")
                    .then(|| value.trim().trim_matches(['\'', '"']).to_owned())
            })
            .filter(|hint| !hint.is_empty() && hint.len() <= 100)
        {
            versions.insert("python".into(), hint);
        }
    }
    if let Ok(path) = child(&root, "manage.py") {
        if read(&path)?.contains("django") {
            add("python:django", "python", "manage.py");
        }
    } else if let Ok(path) = child(&root, "app.py") {
        if read(&path)?.contains("flask") {
            add("python:flask", "python", "app.py");
        }
    }
    if let Ok(path) = child(&root, "pubspec.yaml") {
        if read(&path)?.contains("sdk: flutter") && child(&root, "web/index.html").is_ok() {
            add("flutter:web", "flutter", "pubspec.yaml");
        }
    }
    if let Ok(path) = child(&root, "pom.xml") {
        if read(&path)?.contains("spring-boot")
            && child(&root, if cfg!(windows) { "mvnw.cmd" } else { "mvnw" }).is_ok()
        {
            add("java:spring", "java", "pom.xml");
        }
    }
    if root.join("artisan").is_file() {
        child(&root, "artisan")?;
        add("php:laravel", "php", "artisan");
    }
    if root.join("public/index.php").is_file() {
        child(&root, "public/index.php")?;
        add("php:public", "php", "public/index.php");
        if cfg!(windows) {
            add("apache:php", "apache", "public/index.php");
            add("nginx:php", "nginx", "public/index.php");
        }
    } else if root.join("webroot/index.php").is_file() {
        child(&root, "webroot/index.php")?;
        add("php:cakephp", "php", "webroot/index.php");
        if cfg!(windows) {
            add("apache:php", "apache", "webroot/index.php");
            add("nginx:php", "nginx", "webroot/index.php");
        }
    } else if root.join("index.php").is_file() {
        child(&root, "index.php")?;
        add("php:root", "php", "index.php");
    }
    let projects = std::fs::read_dir(&root)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|v| v == "csproj"))
        .collect::<Vec<_>>();
    if projects.len() == 1 {
        child(&root, &projects[0].file_name().to_string_lossy())?;
        add(
            "dotnet:web",
            "dotnet",
            &projects[0].file_name().to_string_lossy(),
        );
    }
    if root.join("go.mod").is_file() {
        child(&root, "go.mod")?;
        add("go:run", "go", "go.mod");
    }
    if root.join("Cargo.toml").is_file() {
        child(&root, "Cargo.toml")?;
        add("rust:run", "rust", "Cargo.toml");
    }
    if root.join("index.html").is_file() {
        child(&root, "index.html")?;
        add("python:static", "python", "index.html");
    }
    if root.join("public/index.html").is_file() {
        child(&root, "public/index.html")?;
        if cfg!(windows) {
            add("apache:static", "apache", "public/index.html");
        }
        add("nginx:static", "nginx", "public/index.html");
    }
    let dependency_files = [
        "composer.lock",
        "composer.json",
        "package-lock.json",
        "package.json",
        "requirements.txt",
        "pyproject.toml",
        "pubspec.lock",
        "pubspec.yaml",
    ]
    .into_iter()
    .filter(|name| child(&root, name).is_ok())
    .map(str::to_owned)
    .collect();
    let preset = child(&root, "dockon.json")
        .ok()
        .and_then(|path| read(&path).ok())
        .and_then(|source| serde_json::from_str::<dockon_core::ProjectPreset>(&source).ok())
        .filter(|preset| preset.validate().is_ok());
    Ok(Detection {
        root,
        profiles,
        versions,
        dependency_files,
        preset,
    })
}
pub(crate) fn php_extensions(root: &Path) -> Vec<&'static str> {
    [
        "curl",
        "fileinfo",
        "intl",
        "mbstring",
        "openssl",
        "pdo_mysql",
        "pdo_pgsql",
        "pdo_sqlite",
        "sqlite3",
        "mysqli",
        "zip",
    ]
    .into_iter()
    .filter(|name| root.join("ext").join(format!("php_{name}.dll")).is_file())
    .collect()
}

pub(crate) fn validate_identity(id: &str, name: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        bail!("Invalid project id");
    }
    if name.trim().is_empty() || name.len() > 200 {
        bail!("Invalid project name");
    }
    Ok(())
}
pub(crate) fn manifest(
    id: &str,
    name: &str,
    root: &Path,
    installation: &RuntimeInstallation,
    profile: &str,
) -> Result<Manifest> {
    validate_identity(id, name)?;
    let mut args: Vec<LaunchArg> = vec![];
    match profile {
        "apache:static" | "nginx:static" | "apache:php" | "nginx:php" => {}
        "node:vite" => args.extend(["node_modules/vite/bin/vite.js".into(), "--host".into(), "127.0.0.1".into(), "--strictPort".into(), LaunchArg::template("--port=${DOCKON_PORT}")]),
        "python:django" => args.extend(["manage.py".into(), "runserver".into(), "--noreload".into(), LaunchArg::template("127.0.0.1:${DOCKON_PORT}")]),
        "python:flask" => args.extend(["-m".into(), "flask".into(), "--app".into(), "app".into(), "run".into(), "--host=127.0.0.1".into(), LaunchArg::template("--port=${DOCKON_PORT}")]),
        "flutter:web" => args.extend(["run".into(), "--release".into(), "-d".into(), "web-server".into(), "--web-hostname=127.0.0.1".into(), LaunchArg::template("--web-port=${DOCKON_PORT}")]),
        "java:spring" => args.extend(["spring-boot:run".into(), LaunchArg::template("-Dspring-boot.run.arguments=--server.address=127.0.0.1 --server.port=${DOCKON_PORT}")]),
        "java:basic" => args.push("Main.java".into()),
        p if p.starts_with("node:") => {
            child(root, &p[5..])?;
            args.push(p[5..].into());
        }
        "php:laravel" => {
            args.extend([
                "artisan".into(),
                "serve".into(),
                "--host=127.0.0.1".into(),
                LaunchArg::template("--port=${DOCKON_PORT}"),
            ]);
        }
        "php:root" | "php:public" | "php:cakephp" => {
            args.extend([
                "-S".into(),
                LaunchArg::template("127.0.0.1:${DOCKON_PORT}"),
                "-t".into(),
                match profile {
                    "php:public" => "public".into(),
                    "php:cakephp" => "webroot".into(),
                    _ => ".".into(),
                },
            ]);
        }
        "dotnet:web" => args.extend([
            "run".into(),
            "--no-launch-profile".into(),
            "--".into(),
            "--urls".into(),
            LaunchArg::template("http://127.0.0.1:${DOCKON_PORT}"),
        ]),
        "go:run" => args.extend(["run".into(), ".".into()]),
        "rust:run" => args.extend(["run".into(), "--offline".into()]),
        "python:static" => args.extend([
            "-m".into(),
            "http.server".into(),
            LaunchArg::template("${DOCKON_PORT}"),
            "--bind".into(),
            "127.0.0.1".into(),
        ]),
        _ => bail!("Unsupported launch profile"),
    }
    let manifest: Manifest = serde_json::from_value(
        serde_json::json!({"schema_version":1,"id":id,"name":name,"root":root,"executable":installation.executable,"args":args,"runtime":{"installation_id":installation.id,"isolated_python":installation.runtime=="python"},"health":{"type":"http","path":"/"},"startup_timeout_seconds":if matches!(installation.runtime.as_str(), "flutter" | "java" | "python") {300} else {120}}),
    )?;
    manifest.validate()?;
    Ok(manifest)
}
pub async fn import(
    manager: &RuntimeManager,
    id: &str,
    name: &str,
    root: &Path,
    installation_id: &str,
    profile: &str,
) -> Result<Manifest> {
    let installation = manager.selected(installation_id).await?;
    let detection = detect(root)?;
    if !detection
        .profiles
        .iter()
        .any(|p| p.id == profile && p.runtime == installation.runtime)
    {
        bail!("Launch profile and selected runtime do not match detected project files");
    }
    // Existing source and dependency files are never rewritten during import.
    if installation.runtime == "dotnet" && root.join("global.json").is_file() {
        let config: serde_json::Value =
            serde_json::from_str(&read(&child(&detection.root, "global.json")?)?)?;
        if config["sdk"]["version"].as_str() != Some(&installation.version) {
            bail!("global.json SDK differs from the selected runtime; select the pinned SDK or edit the project explicitly");
        }
    }
    let mut result = manifest(id, name, &detection.root, &installation, profile)?;
    result.template = match profile {
        "java:spring" => Some("spring-boot".into()),
        "node:vite" => Some("vite-import".into()),
        "python:django" => Some("django-import".into()),
        "flutter:web" => Some("flutter".into()),
        _ => None,
    };
    Ok(result)
}
pub async fn attach_php_cgi(
    manager: &RuntimeManager,
    manifest: &mut Manifest,
    profile: &str,
    installation_id: Option<&str>,
) -> Result<()> {
    if !matches!(profile, "apache:php" | "nginx:php") {
        if installation_id.is_some() {
            bail!("The selected profile does not use PHP CGI");
        }
        return Ok(());
    }
    let id = installation_id.context("Select an installed PHP CGI executable")?;
    let cgi = manager.selected(id).await?;
    if cgi.runtime != "php-cgi" {
        bail!("The selected runtime must be PHP CGI");
    }
    let service: ServiceSpec = serde_json::from_value(serde_json::json!({
        "id": "php-cgi",
        "executable": cgi.executable,
        "args": [
            "-d", "cgi.force_redirect=0",
            "-d", "cgi.fix_pathinfo=0",
            "-b", {"template": "127.0.0.1:${DOCKON_PORT}"}
        ],
        "runtime": {"installation_id": cgi.id},
        "health": HealthCheck::Tcp,
        "working_dir": if manifest.root.join("webroot/index.php").is_file() && !manifest.root.join("public").exists() { "webroot" } else { "public" }
    }))?;
    manifest.services.push(service);
    manifest.validate()
}

pub async fn create(
    manager: &RuntimeManager,
    id: &str,
    name: &str,
    root: &Path,
    installation_id: &str,
) -> Result<Manifest> {
    create_with_php(manager, id, name, root, installation_id, None).await
}

pub async fn create_with_php(
    manager: &RuntimeManager,
    id: &str,
    name: &str,
    root: &Path,
    installation_id: &str,
    php_cgi: Option<&str>,
) -> Result<Manifest> {
    validate_identity(id, name)?;
    if !root.is_absolute() || root.exists() {
        bail!("New project path must be absolute and must not already exist");
    }
    let installation = manager.selected(installation_id).await?;
    if php_cgi.is_some()
        && !(installation.runtime == "nginx" || (installation.runtime == "apache" && cfg!(windows)))
    {
        bail!("Choose Apache or Nginx for PHP FastCGI");
    }
    if let Some(id) = php_cgi {
        if manager.selected(id).await?.runtime != "php-cgi" {
            bail!("The selected runtime must be PHP CGI");
        }
    }
    let mut files: Vec<(&str, String)> = vec![];
    let profile = match installation.runtime.as_str() {
        "apache" | "nginx" if php_cgi.is_some() => {
            files.push((
                "public/index.php",
                "<?php header('Content-Type: text/plain'); echo 'dockon · PHP '.PHP_VERSION;\n"
                    .into(),
            ));
            if installation.runtime == "apache" {
                "apache:php"
            } else {
                "nginx:php"
            }
        }
        "nginx" | "apache" if installation.runtime == "nginx" || cfg!(windows) => {
            let server = if installation.runtime == "nginx" {
                "Nginx"
            } else {
                "Apache"
            };
            files.push((
                "public/index.html",
                format!("<!doctype html><meta charset=\"utf-8\"><title>Dockon</title><h1>Dockon · {server}</h1>\n"),
            ));
            if installation.runtime == "nginx" {
                "nginx:static"
            } else {
                "apache:static"
            }
        }
        "node" => {
            files.push(("server.mjs", "import http from 'node:http';\nhttp.createServer((req,res)=>{res.setHeader('Content-Type','text/plain');res.end('dockon · Node.js '+process.version);}).listen(Number(process.env.PORT),'127.0.0.1');\n".into()));
            files.push((
                "package.json",
                "{\"private\":true,\"type\":\"module\"}\n".into(),
            ));
            "node:server.mjs"
        }
        "php" => {
            files.push((
                "index.php",
                "<?php header('Content-Type: text/plain'); echo 'dockon · PHP '.PHP_VERSION;\n"
                    .into(),
            ));
            "php:root"
        }
        "dotnet" => {
            let major = installation
                .version
                .split('.')
                .next()
                .context("Invalid SDK version")?;
            files.push(("DockonApp.csproj",format!("<Project Sdk=\"Microsoft.NET.Sdk.Web\"><PropertyGroup><TargetFramework>net{major}.0</TargetFramework><ImplicitUsings>enable</ImplicitUsings></PropertyGroup></Project>\n")));
            files.push(("Program.cs","var builder = WebApplication.CreateBuilder(args);\nvar app = builder.Build();\napp.MapGet(\"/\", () => \"dockon · .NET\");\napp.Run();\n".into()));
            files.push(("global.json",serde_json::to_string_pretty(&serde_json::json!({"sdk":{"version":installation.version,"rollForward":"disable","allowPrerelease":false}}))?));
            "dotnet:web"
        }
        "go" => {
            files.push(("go.mod", "module dockon.local/app\n\ngo 1.22\n".into()));
            files.push(("main.go","package main\nimport (\"fmt\";\"net/http\";\"os\")\nfunc main(){http.HandleFunc(\"/\",func(w http.ResponseWriter,r *http.Request){fmt.Fprint(w,\"dockon · Go\")});if err:=http.ListenAndServe(\"127.0.0.1:\"+os.Getenv(\"PORT\"),nil);err!=nil{panic(err)}}\n".into()));
            "go:run"
        }
        "python" => {
            files.push(("index.html","<!doctype html><meta charset=\"utf-8\"><title>dockon</title><h1>dockon · Python</h1>\n".into()));
            "python:static"
        }
        "java" => {
            files.push(("Main.java", "import com.sun.net.httpserver.HttpServer;\nimport java.net.InetSocketAddress;\npublic class Main { public static void main(String[] args) throws Exception { var s=HttpServer.create(new InetSocketAddress(\"127.0.0.1\", Integer.parseInt(System.getenv(\"PORT\"))),0); s.createContext(\"/\", e->{ byte[] body=\"Dockon - Java\".getBytes(); e.sendResponseHeaders(200,body.length); try (var out=e.getResponseBody()) { out.write(body); } }); s.start(); } }\n".into()));
            "java:basic"
        }
        "rust" => {
            files.push((
                "Cargo.toml",
                "[workspace]\n\n[package]\nname = \"dockon-app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"
                    .into(),
            ));
            files.push(("src/main.rs","use std::{io::{Read,Write},net::TcpListener};\nfn main(){let listener=TcpListener::bind(format!(\"127.0.0.1:{}\",std::env::var(\"PORT\").unwrap())).unwrap();for stream in listener.incoming(){if let Ok(mut stream)=stream{let _=stream.read(&mut [0u8;4096]);let body=\"dockon - Rust\";let _=write!(stream,\"HTTP/1.1 200 OK\\r\\nContent-Length: {}\\r\\nConnection: close\\r\\n\\r\\n{}\",body.len(),body);}}}\n".into()));
            "rust:run"
        }
        _ => bail!("Selected runtime has no project creation profile"),
    };
    let parent = root
        .parent()
        .context("Project parent directory is missing")?
        .canonicalize()?;
    if !parent.is_dir() {
        bail!("Select an existing parent directory");
    }
    // Exclusive directory creation protects existing source, including racing requests.
    std::fs::create_dir(root)?;
    for (name, contents) in files {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().context("File parent unavailable")?)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(contents.as_bytes())?;
    }
    let root = dunce::canonicalize(root)?;
    let mut manifest = manifest(id, name, &root, &installation, profile)?;
    attach_php_cgi(manager, &mut manifest, profile, php_cgi).await?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("dockon.json"))?;
    file.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    Ok(manifest)
}
