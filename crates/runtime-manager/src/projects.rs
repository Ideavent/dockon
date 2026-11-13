//! Read-only detection and explicit source creation/import. No dependency installer is run.
use crate::RuntimeManager;
use anyhow::{bail, Context, Result};
use dockon_core::{LaunchArg, Manifest, RuntimeInstallation};
use serde::Serialize;
use std::{
    io::{Read, Write},
    path::Path,
};

#[derive(Debug, Serialize)]
pub struct Detection {
    pub root: std::path::PathBuf,
    pub profiles: Vec<Profile>,
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
    if root.join("artisan").is_file() {
        child(&root, "artisan")?;
        add("php:laravel", "php", "artisan");
    }
    if root.join("public/index.php").is_file() {
        child(&root, "public/index.php")?;
        add("php:public", "php", "public/index.php");
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
    Ok(Detection { root, profiles })
}
fn validate_identity(id: &str, name: &str) -> Result<()> {
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
fn manifest(
    id: &str,
    name: &str,
    root: &Path,
    installation: &RuntimeInstallation,
    profile: &str,
) -> Result<Manifest> {
    validate_identity(id, name)?;
    let mut args: Vec<LaunchArg> = vec![];
    match profile {
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
        "php:root" | "php:public" => {
            args.extend([
                "-S".into(),
                LaunchArg::template("127.0.0.1:${DOCKON_PORT}"),
                "-t".into(),
                if profile == "php:public" {
                    "public".into()
                } else {
                    ".".into()
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
        serde_json::json!({"schema_version":1,"id":id,"name":name,"root":root,"executable":installation.executable,"args":args,"runtime":{"installation_id":installation.id,"isolated_python":installation.runtime=="python"},"health":{"type":"http","path":"/"},"startup_timeout_seconds":120}),
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
    manifest(id, name, &detection.root, &installation, profile)
}
pub async fn create(
    manager: &RuntimeManager,
    id: &str,
    name: &str,
    root: &Path,
    installation_id: &str,
) -> Result<Manifest> {
    validate_identity(id, name)?;
    if !root.is_absolute() || root.exists() {
        bail!("New project path must be absolute and must not already exist");
    }
    let installation = manager.selected(installation_id).await?;
    let mut files: Vec<(&str, String)> = vec![];
    let profile = match installation.runtime.as_str() {
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
        "rust" => {
            files.push((
                "Cargo.toml",
                "[package]\nname = \"dockon-app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"
                    .into(),
            ));
            files.push(("src/main.rs","use std::{io::Write,net::TcpListener};\nfn main(){let listener=TcpListener::bind(format!(\"127.0.0.1:{}\",std::env::var(\"PORT\").unwrap())).unwrap();for stream in listener.incoming(){if let Ok(mut stream)=stream{let body=\"dockon - Rust\";let _=write!(stream,\"HTTP/1.1 200 OK\\r\\nContent-Length: {}\\r\\nConnection: close\\r\\n\\r\\n{}\",body.len(),body);}}}\n".into()));
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
    let manifest = manifest(id, name, &root, &installation, profile)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("dockon.json"))?;
    file.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    Ok(manifest)
}
