# dockon

Cross-platform local development environment manager. Working foundation under development; see [progress](docs/progress.md) and [limitations](docs/adr/0001-foundation.md).

## Windows development

From C:\dockon_build in PowerShell:

```powershell
. .\scripts\Enter-Dev.ps1
pnpm install --frozen-lockfile
cargo build --workspace
pnpm desktop
```

Rust toolchain is installed locally in .tools on this development machine. On other machines install stable Rust, Node 24+, pnpm 11.25.0 and official Tauri 2 prerequisites. The application does not change global PATH.

The Windows setup creates `C:\dockon\www`. Each immediate directory appears in the Projects page. Use Configure project to detect its files and assign its local runtime. The installer preserves `www` during uninstall. Native project start and stop work without Docker. The shared native web, database, cache and mail stack is under development.

Create/import a sample after building:

```powershell
.\scripts\New-SampleManifest.ps1
```

Open **Menu → Import manifest** and paste the returned path. The **+** button creates a project or configures an existing source folder. Registered projects have their own start and stop controls.

Open **Settings** to download Apache, Nginx, PHP, PHP FastCGI, Node.js and other runtimes. When creating a PHP project, choose Apache, Nginx or the PHP development server and the installed version. The wizard also lets you attach a project database or continue without one. The PostgreSQL and MySQL presets run bundled Windows binaries with project-specific data directories and credentials. Open **Project settings → Versions** to change the assigned runtime later.

The Redis Cache and Memcached tabs download verified portable Windows x64 builds for native Windows projects. Dockon starts them with the assigned project. The Windows cache presets run with the native project.

Dockon prepares each project's local address when it starts. If the requested address is occupied, Dockon picks an available hostname under the configured suffix and saves it for that project. On Windows, approve the system prompt when Dockon needs to update hosts. If a Laragon-generated loopback record conflicts with a Dockon address, the hosts update comments out that record while Dockon manages the address. Enable project mail in the project wizard or project settings to use `mail.<name>.test` for SMTP on port 25, `http://mail.<name>.test/api/v1/` for the Mailpit HTTP API, and `http://webmail.<name>.test` for the inbox. Mailpit captures test messages locally.

The **www** shortcut opens the workspace in Explorer. A terminal shortcut beside each project opens its folder with that project's selected native runtimes on the shell's PATH. Closing the Dockon window hides it in the Windows notification area. Choose **Stop services and quit** in the tray or run `dockonctl shutdown` to shut down Dockon and its workloads.

On Windows x64, select Apache on the Runtimes page to install the pinned Apache Lounge package or enter the absolute path of an existing `httpd.exe`. The project wizard supports Apache static sites with `public/index.html`.

## Project toolchains and templates

Install Composer, portable Python, Flutter SDK, Temurin Java JDK, and Android SDK command-line tools from **Versions and services**. Installed versions can coexist. Set the runtime and tools separately for each project. A PHP project with Composer selected gets a `composer` command in its Dockon PowerShell terminal. Flutter projects can use an installed JDK and either a managed Android SDK directory or an existing SDK directory.

Choose a language in **New project** to see its templates. Dockon resolves the published stable template at creation time. PHP supports Laravel, CodeIgniter, CakePHP, and Symfony. Node.js supports React, Vue, and Svelte with Vite. Python supports Django and Flask. Java supports Spring Boot. Flutter creates Android and web targets and the Dockon **Start** action runs the web target. Flutter on Windows requires Git. Use `sdkmanager --list` and `sdkmanager --licenses` in the Flutter project's SDK terminal to install the Android platform packages and accept their licenses.

## Project setup and maintenance

Use **New project → Import from Git** to clone an HTTPS repository into the workspace. Dockon detects launch profiles, common lockfiles and runtime version hints. Explicitly opt into running repository scripts to install dependencies, or start the imported project after setup.

Each project's **Tasks** page can install dependencies, run tests, start a Laravel queue worker or start a Vite server. Tasks are scoped to the project and stop with it. Flutter projects can start the web preview, run on connected Android devices and boot existing Android virtual devices. **Health checks** reports missing toolchains, PHP extensions and dependencies with links to the related project settings.

In **Database**, take a MySQL or PostgreSQL snapshot and restore it as a new database in the same project or a different running project using the same database engine. The original databases are preserved. Snapshots currently support SQL dumps up to 8 MiB.

In **Presets and sharing**, save a reusable project configuration or export a portable `dockon.json`. Use the preset selector in a new project's wizard, import a shared preset file, or apply a portable `dockon.json` detected in a repository. Presets carry runtime and SDK version labels, task shortcuts, mail, database and SSL configuration. Passwords, executable paths and machine-specific SDK directories are excluded.

## Checks

```powershell
pnpm build
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
node scripts/smoke.mjs
node scripts/project-smoke.mjs
node scripts/runtime-smoke.mjs
node scripts/router-smoke.mjs
node scripts/ui-smoke.mjs
node scripts/desktop-smoke.mjs
```

The smoke test uses an isolated temporary data directory. It verifies real IPC/auth, persistence, two-project isolation, HTTP readiness, idempotence, failure and cleanup.

The browser/native UI checks expect the Vite dev server to be running (`pnpm dev`). `desktop-smoke.mjs` is a Windows WebView2 test; its temporary debugging port is loopback-only and enabled exclusively on the isolated test process, never in normal app configuration.


`runtime-smoke.mjs` downloads the current official Node LTS into an isolated test data directory, verifies its digest/version, runs a project and checks offline catalog recovery. Set `DOCKON_EXTENDED_RUNTIMES=1` to also download and verify supported PHP/.NET/Go/Caddy artifacts and launch project profiles. These are network integration checks, not publisher signature verification.

## Windows x64 installer

Increment the workspace, desktop package and Tauri versions for each installer release. Regenerate `Cargo.lock` after changing the workspace version. The setup filename, Windows installed-app version and Dockon footer show the release version.

Run `./scripts/Build-WindowsInstaller.ps1` from PowerShell after installing the development dependencies. The setup executable and its SHA-256 file appear in `target/x86_64-pc-windows-msvc/release/bundle/nsis`. The installer uses administrator elevation to put application binaries in `C:\dockon`. Dockon runs under the signed-in user's account and stores project metadata, runtimes and backups in their user data directory. Choose **Stop services and quit** in the tray before updating or uninstalling. Uninstall preserves user data.

The current installer is an unsigned development build. Windows/macOS/Linux foundation, Node runtime, Node/Python/Rust project profiles and local routing checks passed in the configured GitHub repository.

**Local domains** lists configured addresses. Change the shared suffix (initially `.test`) and the default SSL setting for new routes there. Existing projects keep their SSL choice when the default changes. Each project can toggle SSL under **Project settings → Mail and address**. Windows asks for administrator approval when local DNS records need to be registered. Trusting the Dockon CA for HTTPS is a separate action in project settings and targets the current Windows user’s ROOT store.
