# DEVFORGE — MASTER PRODUCT & ENGINEERING SPECIFICATION

> **Audience:** Codex / AI coding agents and human engineers  
> **Document type:** Single source of truth for implementation  
> **Status:** Proposed engineering specification, not a claim that features are already built  
> **Spec version:** 1.0  
> **Date:** 2026-11-13  
> **Working product name:** DevForge (renameable)

---

## 0. INSTRUCTIONS FOR CODEX — READ FIRST

You are implementing **DevForge**, a professional, modern, cross-platform local development environment manager intended to go substantially beyond Laragon. Implement working code, not just mockups or placeholder files.

**Hard product decisions (do not silently change):**

1. **Desktop platforms:** Windows, macOS, Linux.
2. **Desktop stack:** Tauri 2 + React + TypeScript + Vite; Tailwind CSS + shadcn/ui; Zustand and TanStack Query as needed.
3. **Core:** Rust stable + Tokio; modular Rust workspace; separate long-running daemon; local authenticated IPC. SQLite for local application state.
4. **Execution model:** **HYBRID** — native processes and opt-in Docker/Podman-style containers through one abstraction. Docker is **not required** for the whole application; when a specific service cannot run natively on a given OS, show a clear compatibility/provider requirement instead of faking native support.
5. **Service resource model:** **ISOLATED PER PROJECT** — separate runtime processes/instances, ports, mutable data, credentials, networks when containerized, logs, and lifecycle. Immutable downloaded binary archives may be cached/shared; mutable application state must not be shared between projects. A global reverse proxy and the daemon may be shared infrastructure.
6. **AI:** **CLOUD-BASED**, provider-neutral adapter; locally collected context with explicit consent, secret redaction, strict tool allowlists, approval gates, and rollback. The platform must remain functional offline except for cloud-only AI and online updates/scans.
7. **Lifecycle:** Do not start all projects on app launch. A project starts only when explicitly started (optionally user-configured per-project auto-start), and only its own dependencies start. Stopping one project must not stop another project's instances.
8. **Project configuration:** Per-project editable web server, runtime/version, database engine/version, Redis/cache, Mailpit, ports, environment, provider, HTTPS, lifecycle, resource policy, backups, and update policy.
9. **Updates:** Detect updates for each managed tool; show compatibility/risk; backup and stage before application; approval for potentially disruptive changes. Never promise universal downgrade safety for databases.
10. **Security:** Least privilege, no blanket admin execution, never transmit secrets to LLM, no automatic destructive/code-changing AI actions without approval.

### Execution discipline

- First inspect the repository. Preserve existing code and conventions when they are sound.
- Use this document as the feature/architecture target; implement incrementally in the ordered phases below.
- Start with the **Sprint 01 vertical slice** in §18; do **not** attempt to implement every feature in one gigantic code change.
- Before each implementation batch, state what will be done, then implement, run available checks, and summarize results and blockers.
- Prefer cohesive code with meaningful tests; do not add nonfunctional UI buttons or simulated statuses presented as real.
- When a service or runtime is not implemented, expose an explicit `unsupported` / `not_implemented` capability state rather than claiming support.
- Keep ABI/IPC/manifest schema versioned. Do not silently break previously saved projects.
- For OS-specific APIs and library dependencies, verify compatibility against up-to-date official documentation when network access is available.
- If a decision is underspecified, use a safe default, record it in `docs/adr/`, and proceed without blocking ordinary implementation.
- **Definition of done:** code compiles, tests pass, target OS behavior is verified or clearly marked unverified, relevant docs updated, and no fake success messages.

---

## 1. PRODUCT VISION

DevForge is a modern, extensible **local development orchestration desktop application** for Windows, macOS, and Linux. Users can create/import projects in PHP, Node.js, .NET, Go, Python, Rust, and future ecosystems, assign isolated supporting services (databases, caches, SMTP/mail testing, queues, object storage), choose Apache/Nginx/Caddy/built-in servers where compatible, manage versions, domains and HTTPS, observe logs/health, scan for vulnerabilities, and request safe cloud-AI diagnostics/repair plans.

### Product pillars

- **One project, one reproducible environment.** Each project has independent configuration and mutable service instances.
- **On-demand execution.** Only active projects' workloads consume resources; the daemon/router are common infrastructure exceptions.
- **Cross-platform interoperability.** An explicit capability matrix and native/container fallback where supported.
- **Observability and safety by default.** Clear logs, status, backups, approvals, secret hygiene.
- **Agent-assisted but not agent-controlled.** Cloud AI may analyze and propose; local validated tools execute only authorized actions.
- **Extensibility.** Runtime/service adapters and a future signed plugin system.

### Not in initial scope

- Remote production hosting, cloud orchestration, remote multi-user server management.
- Kubernetes orchestration.
- A public disposable email provider (Mailpit is a **local SMTP catcher**, not an internet temp-mail inbox).
- Guaranteed native availability of every service/version on every OS/CPU architecture.
- Automatic direct AI code edits, unrestricted terminal commands, or unattended database migrations.

---

## 2. TARGET PLATFORMS AND DISTRIBUTION

- Windows: modern supported Windows desktop releases, x64 initially; ARM64 later according to dependency availability.
- macOS: supported Intel/Apple Silicon targets; app signing/notarization for distribution.
- Linux: start with selected current Debian/Ubuntu and Fedora-compatible targets; test desktop prerequisites (including webview) and packaging. Expand via compatibility matrix.
- Installer/updater: signed installers and signed updates, verified artifact hashes, safe migration of persisted state.
- OS integration: Windows Named Pipes, Unix Domain Sockets for macOS/Linux; app system tray, OS keychain/credential store, platform paths and permission flow.
- Background daemon: independently managed lifecycle, reconnect after UI restart. Service manager integration (Windows Service, launchd, systemd) only after user permission and tested OS-specific implementation; support per-user startup where feasible.
- App uninstall **must not delete project directories, service data or backups without explicit opt-in**.

---

## 3. TECH STACK (BASELINE)

| Layer | Technology |
|---|---|
| Desktop shell | Tauri 2 |
| Frontend | React, TypeScript, Vite |
| Styling / controls | Tailwind CSS, shadcn/ui, Lucide |
| UI state & async data | Zustand, TanStack Query |
| Core / daemon | Rust stable, Tokio |
| Serialization / config | serde, serde_json, serde_yaml (or maintained equivalent) |
| Persistence | SQLite with SQLx migrations |
| HTTP / downloads | reqwest; streaming downloads |
| Logging & tracing | tracing, structured JSON logs |
| IPC | Windows Named Pipe / macOS/Linux Unix Domain Socket; typed versioned requests/events |
| Reverse proxy | Caddy as global local-domain/HTTPS routing service |
| Per-project web engines | Nginx, Apache, built-in app servers, Caddy where appropriate |
| Native runtime supervision | Rust process supervisor + OS-specific child process group handling |
| Optional containers | Docker/Podman compatible backend adapter |
| Mail testing | Mailpit |
| Security intelligence | OSV API, official advisories, optional Trivy integration |
| Cloud AI | Provider-agnostic HTTPS API adapter, explicit user configuration |
| Automated builds | GitHub Actions multi-OS matrix |
| Tests | Rust unit/integration, Vitest, Playwright where suitable |

**Libraries are recommendations, not permission to accept abandoned/insecure packages.** Select current maintained versions after checking compatibility. Keep external dependencies minimal.

---

## 4. HIGH-LEVEL ARCHITECTURE

```text
┌─────────────────────────────────────────────────────────────┐
│ DevForge Desktop UI (React / TypeScript / Tauri 2)         │
│ Dashboard · Projects · Runtimes · Services · Mail · AI      │
└───────────────────────────┬─────────────────────────────────┘
                            │ authenticated local IPC
┌───────────────────────────▼─────────────────────────────────┐
│ Rust Core Daemon                                            │
│  IPC Gateway / Permission Policy / Event Bus                │
│  Project Manager / Orchestrator / Dependency Graph          │
│  Runtime Registry / Service & Container Providers           │
│  Web Routing / Domain / TLS / Update / Backup / Diagnosis   │
│  Security Scanner / Cloud AI Gateway / Plugin Registry      │
│  SQLite State / Secret Store Integration / Operation Queue  │
└──────────────┬────────────────┬────────────────┬─────────────┘
               │                │                │
       Native OS Adapter   Container Adapter   Cloud AI API
       (processes, FS,    (Docker/Podman)     (redacted context)
        ports, privileges)
               │                │
        Per-project runtimes, web servers, databases,
        caches, mail catchers and background workers
```

### Core architectural rules

- UI does not directly launch services, edit system hosts or access privileged processes. UI calls the daemon through typed IPC.
- Daemon process supervision is **independent** from the Tauri window. UI state is a projection of daemon events/state, not the source of truth.
- Persistent desired state is recorded separately from observed runtime state. On daemon startup, reconcile safely.
- Long-running actions are asynchronous operations with operation IDs, state transitions, logs, cancellation when possible, and event streams.
- No hidden system-wide changes. Privileged operations pass through a narrow helper or platform-specific elevated flow.
- A shared proxy/router may route multiple projects, but application and data service instances remain project-isolated.
- Cross-platform service support is a **tested capability**, not a marketing assumption.

---

## 5. MONOREPO STRUCTURE

```text
devforge/
├── apps/
│   └── desktop/
│       ├── src/
│       │   ├── app/
│       │   ├── components/
│       │   ├── features/
│       │   │   ├── dashboard/
│       │   │   ├── projects/
│       │   │   ├── runtimes/
│       │   │   ├── services/
│       │   │   ├── databases/
│       │   │   ├── mail/
│       │   │   ├── updates/
│       │   │   ├── diagnostics/
│       │   │   ├── ai/
│       │   │   └── settings/
│       │   ├── ipc/
│       │   └── state/
│       └── src-tauri/
├── crates/
│   ├── core/                  # Domain entities, state machine, policy
│   ├── daemon/                # Independent background app
│   ├── ipc/                   # Versioned protocol and transport
│   ├── persistence/           # SQLite repository and migrations
│   ├── project-manager/
│   ├── orchestrator/
│   ├── provider-native/
│   ├── provider-container/
│   ├── runtime-manager/
│   ├── service-catalog/
│   ├── web-manager/
│   ├── domain-tls-manager/
│   ├── database-manager/
│   ├── mail-manager/
│   ├── update-manager/
│   ├── backup-manager/
│   ├── diagnostics/
│   ├── security-scanner/
│   ├── ai-gateway/
│   ├── secrets/
│   └── platform/
├── schemas/
│   ├── project-manifest.schema.json
│   ├── ipc-v1.schema.json
│   └── ai-repair-plan.schema.json
├── manifests/                # Signed/verifiable artifact registry metadata
├── examples/
│   ├── hello-http/
│   ├── php-laravel/
│   ├── node-next/
│   ├── dotnet-api/
│   └── go-api/
├── tests/
│   ├── integration/
│   ├── lifecycle/
│   └── platform/
├── docs/
│   ├── adr/
│   ├── architecture/
│   ├── security/
│   ├── compatibility-matrix.md
│   └── developer-guide.md
├── scripts/
├── .github/workflows/
├── Cargo.toml
├── pnpm-workspace.yaml
└── README.md
```

Do not create empty crates for all future modules on day one. Establish shared contracts, then add real module implementations phase by phase.

---

## 6. FUNCTIONAL MODULES AND REQUIRED FEATURES

### PRJ — Projects

- Create, import, list, rename, archive/remove project registrations without silently deleting source files.
- Detect project type from `composer.json`, `artisan`, `package.json`, `.csproj`, `.sln`, `go.mod`, `pyproject.toml`, `requirements.txt`, `Cargo.toml`, etc.
- Support Laravel, WordPress, Symfony, CodeIgniter; Express, NestJS, Next.js, Nuxt, Vite; ASP.NET Core/Blazor; Gin/Fiber/Echo; Django/FastAPI/Flask; Rust web frameworks as launch profiles, expandable to others.
- Support custom command-based runtimes and generic executables.
- Per-project settings UI: web server, provider (native/container/auto), runtime/version, DB/cache/mail/queue, domain, HTTPS, env, ports, working dir, log policy, update policy.
- Project template system, project open-in-editor, local terminal integration, browser launch.
- Environment export/import with sanitized credentials and explicit secret recreation.
- Project root symlink/path traversal safety; never trust manifest paths blindly.

### RUN — Runtime manager

- Multi-version PHP, Node.js, .NET SDK/runtime, Go, Python, Rust; architecture-aware distribution metadata.
- Extensible adapters for Java/Kotlin, Ruby, Bun, Deno, Elixir, etc.
- Project-level pinned versions. Shared immutable binary installations allowed; project-level settings/processes/venvs and mutable state isolated.
- Discovery of installed system runtimes; explicit opt-in to use system installations.
- Runtime downloads: publisher origin, artifact hash, metadata signature when available, architecture compatibility, quarantine/staging, atomic installation.
- Update discovery, incompatibility warnings, patch/minor/major classification, rollback path when supported.
- Do not rewrite global PATH to change one project's runtime; inject per-process environment or private tool shims.

### WEB — Web engine manager

- **Caddy:** central local domain routing, HTTPS and reverse proxy; can also be used as per-project engine where relevant.
- **Nginx:** per-project web server with generated config, reverse proxy and PHP-FPM support.
- **Apache:** per-project web server, `mod_php` or PHP-FPM where supported, `.htaccess` compatibility considerations.
- **Built-in app servers:** Node.js, ASP.NET Core/Kestrel, Go, Python, Rust launch commands; proxy fronts application when domain/TLS needed.
- Server switching must generate a preview/diff, check feature compatibility and validate config before replacing running server.
- Clearly communicate unsupported semantics when switching Apache `.htaccess` rules to Nginx (not automatic 1:1 conversion).
- Automatic available-port allocation with runtime confirmation; detect binding conflicts and surface owner/diagnostics when allowed.
- Local domain defaults such as `<project>.test`; hosts/DNS changes only with permission; HTTPS trust through OS adapters.
- Do not open development servers to public interfaces by default; bind loopback unless explicitly enabled.

### DB — Database manager

- MySQL, MariaDB, PostgreSQL, MongoDB, SQLite; expand through adapters (e.g. additional search/data services).
- Separate instance and data directory/volume per project, including when versions match.
- Independent version, credentials, listening port/socket, storage policy, start/stop, readiness healthchecks.
- Provide database connection strings as secret references, not literal secrets in committed project manifests.
- Database creation/drop UI with explicit confirmation, export/import, backups, restoration and compatibility validation.
- No assumption that MongoDB/MySQL/etc. have native builds on all OS/arch combinations; use optional container provider when that is the supported route.
- Database engine **downgrades and storage format rollbacks are not assumed safe**. Use verified backups/migrations and engine-specific rules.
- Review redistribution and license obligations for embedded third-party binaries.

### SVC — Supporting services

- Redis and optionally Valkey; RabbitMQ, Memcached, MinIO, Meilisearch, OpenSearch/Elasticsearch where supportable.
- **Mailpit** SMTP catcher: project-specific instance/inbox, SMTP connection settings, embedded message list/read/HTML preview/attachments via its API, test automation hooks.
- Mailpit is **not** a public disposable email account service; public temp-mail would be a separate later feature with abuse/privacy controls.
- Queue worker manager, scheduler/cron-like project jobs, optional background runners, debug processes.
- Supporting services always obey project-scoped lifecycle and ownership.

### ORC — Smart lifecycle orchestration

- Desired project states: STOPPED, STARTING, RUNNING, DEGRADED, STOPPING, FAILED; include recovery/migration states if necessary.
- Start a project only by explicit command or project-level approved autostart policy. Do not start all projects on UI/daemon launch.
- Construct dependency graph; validate acyclic dependencies; start dependencies in topological order, wait for readiness, start app, register proxy route.
- Stop in reverse order, gracefully terminate workers first and data services last; timeouts, forced termination with warning.
- Projects isolated: stopping project A must not touch project B's service process/volume/credentials even if same engine and version.
- Shared daemon, immutable download cache, and global router are permitted infrastructure exceptions; project instances are **not** shared.
- Multiple simultaneous start/stop requests are serialized per project and idempotent.
- Crash recovery, backoff policy, restart ceilings; never infinite crash loop.
- Persist desired state and journal operations; reconcile orphaned processes safely on daemon restart.
- Closing a project tab is not necessarily stopping its services; explicit stop button and configurable idle policy.
- Distinguish service `process exists`, `ready`, and `healthy` states.

### ISO — Hybrid execution and isolation

- `native`, `container`, `auto` backend selections.
- Native: per-project process group, cwd, env, ports, data paths, resource monitoring; native alone does **not** guarantee sandbox-grade security isolation.
- Container: separate per-project networks (where feasible), volumes, labels, env, port bindings, healthchecks, image pull verification, cleanup without data loss.
- Backend switching must include a user-visible migration plan (storage compatibility and address changes), never silently convert existing persistent databases.
- Capability matrix by OS, arch, tool/version, backend. No fake portability promises.
- Support optional Docker and Podman; detect availability. Use an explicit integration implementation and tested compatibilities rather than assume APIs are identical.
- Local database ports should not be globally published unless requested; favor loopback or private network.

### NET — Domain, TLS and networks

- Default `.test` domain strategy; name collision detection; hosts or local DNS proxy implementation.
- Separate certificate generation, certificate trust, certificate renewal/removal policies; trust store changes require explicit elevation.
- Caddy local HTTPS where compatible, but domain resolution configured separately.
- Global routing table is managed atomically; when project stops remove/disable its route safely without affecting others.
- Explicit local LAN share feature: off by default, warns about secrets/public exposure. Optional tunnel integration only with user consent.

### VER / UPD — Updates

- Runtime and service version discovery, EOL advisories, vulnerability associations, platform compatibility and engine-specific migration rules.
- Policy per project: notify only default; separate patch/minor/major approvals.
- App self-updater is distinct from managed-runtime update engine.
- Staged download, verification, compatibility dry run, backup/snapshot where relevant, apply, healthcheck, audit and rollback when technically possible.
- Downloaded binary version artifacts may coexist and should not replace actively used versions unexpectedly.
- Database major upgrades require explicit backup and restore/migration plan; some rollback operations are restore-from-backup only.
- No automatic destructive upgrades without explicit user policy and verified safeguards.

### SEC — Security center

- Local inventory / software bill of materials (SBOM) where possible.
- Dependency vulnerability mapping (OSV API, lockfiles, official advisories); optional Trivy invocation; offline cache labeling and freshness.
- Findings include source, advisory ID, severity, affected version, recommended safe version and uncertainty.
- Vulnerability listed for a dependency does not automatically imply exploitability in a specific project.
- Detect secret leaks in logs/configs, open ports, permissive bindings and unsafe configuration; avoid printing detected secret values.
- Authenticated IPC, least-privilege elevation, secure secret storage, audit trail.

### AI — Cloud AI DevOps Guardian

- Provider-neutral cloud API adapter, configurable provider/model/endpoint within allowlisted protocol; API keys from OS secret store.
- AI capabilities: analyze runtime/service versions; correlate security advisories; read sanitized logs; identify startup/runtime/config/network faults; recommend fixes; propose test commands and updates; explain compatibility failures; provide AI-guided troubleshooting.
- Cloud AI is optional **at runtime**, but the chosen AI architecture is cloud. The rest of DevForge works without AI/internet.
- Context minimization: send only user-approved relevant snippets/metadata. Default `includeSourceCode=false` and redaction enabled. Never send secrets, credential files, private keys or raw `.env` values.
- Use deterministic security scanners for vulnerability findings; LLM interprets findings and proposes plans, does not authoritatively invent CVEs.
- No raw arbitrary tool execution suggested by model is executed automatically. Validate typed actions against local allowlist, resource scope, current user approval and expected preconditions.
- AI actions use dry run/diff, backup when applicable, explicit approval, execute, verify, audit, rollback plan. DB migration/data deletion always require special confirmation.
- Treat logs, web pages, repository files and model outputs as untrusted (prompt-injection-resistant tool boundary).
- Cost/rate limit, request cancellation, data-retention disclosure, model errors, timeouts, offline fallback messaging.

### OBS — Diagnostics and UI observability

- Project dashboard with actual state and resource usage; no simulated live metrics.
- Per-service start/stop/restart, log streaming with time and severity filters, download/redact logs.
- CPU/RAM/disk/ports, resource budget and idle policy per project where enforceable by provider/OS.
- Healthchecks and failure reason with suggested next action.
- Operations activity feed with correlation ID, timestamps, result, errors and retry action.

### PLG — Future extension system

- Start with built-in adapters and signed/versioned declarative package manifests.
- Design stable adapter interfaces and capabilities registry; do not enable untrusted arbitrary code plugins in MVP.
- Later signed plugin SDK, permissions/capabilities, compatibility checks and update/revocation policy.

---

## 7. PROJECT MANIFEST CONTRACT

A project can have `devforge.yaml`; alternatively store the same validated manifest in app state for projects without a file. The schema is versioned and must be validated before using any path/command.

**Example (illustrative; implement formal JSON Schema in `schemas/project-manifest.schema.json`):**

```yaml
schema_version: 1
project:
  id: ecommerce-api
  name: Ecommerce API
  root: .
  domain: ecommerce-api.test

execution:
  provider: auto              # auto | native | container
  isolation: project          # required
  auto_start: false

runtime:
  id: dotnet
  version: "10"
  working_dir: .
  command: ["dotnet", "run", "--no-launch-profile"]
  environment:
    ASPNETCORE_ENVIRONMENT: Development

web:
  engine: builtin             # builtin | nginx | apache | caddy
  proxy: caddy
  https: true
  application_port: auto
  bind_address: 127.0.0.1

services:
  - id: document-db
    type: database
    engine: mongodb
    version: "8.0"
    provider: container
    storage: persistent
    expose_to_host: false
  - id: project-cache
    type: cache
    engine: redis
    version: "7.4"
    provider: auto
    storage: persistent
  - id: project-mail
    type: mail
    engine: mailpit
    provider: auto
    storage: persistent

lifecycle:
  start_dependencies: true
  stop_on_project_stop: true
  graceful_timeout_seconds: 20
  restart_policy: on_failure
  idle_timeout_seconds: null

updates:
  check: daily
  runtime_policy: notify
  service_policy: notify
  backup_before_update: true
  allow_major_updates: false

security:
  vulnerability_scanning: true
  allow_external_access: false

ai:
  diagnostics: true
  cloud_enabled: true
  send_source_code_by_default: false
  auto_apply_changes: false
  require_approval: true
```

**Validation/security requirements:**

- Unique project ID and unique per-project service IDs.
- Paths constrained to authorized root/data paths; normalize/canonicalize and check symlink/path traversal.
- Commands parsed as argument arrays, **not** interpolated into a shell by default; optional shell profile requires explicit risk warning.
- Arbitrary manifest files cannot grant daemon privileged API access.
- Version requests resolved through registry with explicit unsupported errors.
- No passwords, API keys, or secret values in checked-in manifests. Use secret references to secure store.
- Project name/domain sanitized; reject unsafe/invalid domains; detect collisions.
- Manifest changes generate preview and restart/reconfiguration plan when running.
- Schema migrations are explicit, reversible where possible, and covered by tests.

---

## 8. RUST MODULE INTERFACES (CONCEPTUAL CONTRACTS)

These are representative signatures; move shared types/errors into common crates and adapt async traits idiomatically. Do not blindly copy snippets if dependency versions require other patterns.

### 8.1 ExecutionProvider

```rust
#[async_trait::async_trait]
pub trait ExecutionProvider: Send + Sync {
    async fn prepare(&self, spec: &ServiceSpec)
        -> Result<PreparedService, EngineError>;
    async fn start(&self, prepared: &PreparedService)
        -> Result<InstanceHandle, EngineError>;
    async fn inspect(&self, instance: &InstanceHandle)
        -> Result<ServiceStatus, EngineError>;
    async fn stop(&self, instance: &InstanceHandle, grace_seconds: u32)
        -> Result<(), EngineError>;
    async fn remove(&self, instance: &InstanceHandle)
        -> Result<(), EngineError>;
}
```

Implement `NativeExecutionProvider` and `ContainerExecutionProvider`. All handles must carry immutable project owner identity. Provider checks ownership before action.

### 8.2 RuntimeAdapter

```rust
#[async_trait::async_trait]
pub trait RuntimeAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    async fn detect(&self, project_path: &std::path::Path)
        -> Result<Detection, RuntimeError>;
    async fn list_versions(&self)
        -> Result<Vec<RuntimeVersion>, RuntimeError>;
    async fn install(&self, version: &str, platform: &Platform)
        -> Result<RuntimeInstallation, RuntimeError>;
    async fn build_launch_spec(&self, project: &ProjectConfig)
        -> Result<ServiceSpec, RuntimeError>;
    async fn check_compatibility(&self, project: &ProjectConfig, target_version: &str)
        -> Result<CompatibilityReport, RuntimeError>;
}
```

### 8.3 ServiceAdapter

```rust
#[async_trait::async_trait]
pub trait ServiceAdapter: Send + Sync {
    fn engine_id(&self) -> &'static str;
    fn capabilities(&self, platform: &Platform) -> ServiceCapabilities;
    async fn validate(&self, spec: &ServiceConfig) -> Result<(), ServiceError>;
    async fn generate_config(&self, ctx: &InstanceContext)
        -> Result<GeneratedConfig, ServiceError>;
    async fn healthcheck(&self, handle: &InstanceHandle)
        -> Result<HealthStatus, ServiceError>;
    async fn plan_update(&self, request: &UpdateRequest)
        -> Result<UpdatePlan, ServiceError>;
}
```

### 8.4 ProjectOrchestrator

```rust
#[async_trait::async_trait]
pub trait ProjectOrchestrator: Send + Sync {
    async fn validate(&self, project_id: &ProjectId)
        -> Result<ValidationReport, OrchestrationError>;
    async fn start(&self, project_id: &ProjectId, key: &IdempotencyKey)
        -> Result<OperationId, OrchestrationError>;
    async fn stop(&self, project_id: &ProjectId, key: &IdempotencyKey)
        -> Result<OperationId, OrchestrationError>;
    async fn status(&self, project_id: &ProjectId)
        -> Result<ProjectStatus, OrchestrationError>;
}
```

### 8.5 UpdateManager

```rust
#[async_trait::async_trait]
pub trait UpdateManager: Send + Sync {
    async fn check(&self, project_id: &ProjectId) -> Result<UpdateInventory, UpdateError>;
    async fn plan(&self, request: UpdateRequest) -> Result<UpdatePlan, UpdateError>;
    async fn apply(&self, plan_id: &PlanId, approval: &ApprovalToken)
        -> Result<OperationId, UpdateError>;
}
```

### 8.6 AIGateway

```rust
#[async_trait::async_trait]
pub trait AIGateway: Send + Sync {
    async fn analyze(&self, req: AIAnalysisRequest)
        -> Result<AIAnalysis, AIError>;
    async fn propose_repair(&self, analysis_id: &AnalysisId)
        -> Result<AIRepairPlan, AIError>;
}
```

The AIGateway **never** executes repairs directly. Repair plan execution belongs to a separate strict `ApprovedActionExecutor` with allowlisted typed actions.

---

## 9. IPC CONTRACT (`devforge.ipc.v1`)

**Transport:** OS-local authenticated socket/pipe. Permit only the logged-in user/session by default, protect file permissions/ACL and use challenge/authorization if necessary. Avoid public TCP listener.

**Envelope:** JSON-RPC 2.0-like typed message, with schema version and structured errors. All potentially long-running commands return an `operation_id`; completion comes via events/status polling.

| Method | Params | Result |
|---|---|---|
| `system.health` | none | daemon health/version |
| `system.capabilities` | none | OS, architecture, providers, service support |
| `projects.list` | none | project summaries |
| `projects.create` | manifest | project ID |
| `projects.validate` | manifest | validation report |
| `projects.start` | project ID, idempotency key | operation ID |
| `projects.stop` | project ID, idempotency key | operation ID |
| `projects.status` | project ID | desired/observed status |
| `services.list` | project ID | instances/status |
| `services.restart` | project ID + service ID | operation ID |
| `runtimes.list` | runtime ID | supported/installed versions |
| `runtimes.install` | runtime ID + version | operation ID |
| `updates.plan` | project ID | plan |
| `updates.apply` | plan ID + approval token | operation ID |
| `diagnostics.run` | project ID | report/operation ID |
| `ai.analyze` | project ID + scope | analysis ID/operation ID |
| `ai.apply_plan` | plan ID + approval token | operation ID |
| `operations.get` | operation ID | status/progress/errors |
| `events.subscribe` | filter + last sequence | replay/live events |

**Example request:**

```json
{
  "jsonrpc": "2.0",
  "id": "req-001",
  "method": "projects.start",
  "params": {
    "project_id": "ecommerce-api",
    "idempotency_key": "start-ecommerce-001"
  }
}
```

**Example accepted response (not the same as service ready):**

```json
{
  "jsonrpc": "2.0",
  "id": "req-001",
  "result": {"operation_id": "op-1001", "accepted": true}
}
```

**Example event:**

```json
{
  "event": "project.status_changed",
  "schema_version": 1,
  "sequence": 1029,
  "timestamp": "2026-11-13T20:00:00Z",
  "project_id": "ecommerce-api",
  "operation_id": "op-1001",
  "payload": {
    "previous": "STARTING",
    "current": "RUNNING",
    "healthy_services": 4,
    "total_services": 4
  }
}
```

**Errors:** `PROJECT_NOT_FOUND`, `MANIFEST_INVALID`, `PORT_CONFLICT`, `RUNTIME_UNAVAILABLE`, `PROVIDER_UNAVAILABLE`, `PERMISSION_REQUIRED`, `SERVICE_START_FAILED`, `HEALTHCHECK_TIMEOUT`, `UPDATE_INCOMPATIBLE`, `BACKUP_FAILED`, `AI_APPROVAL_REQUIRED`, `CLOUD_UNAVAILABLE`, `OPERATION_CONFLICT`, `UNSUPPORTED_PLATFORM`, `UNAUTHORIZED`.

**Requirements:** event sequence/replay or full-state reconciliation after UI reconnect; cancellation where supported; operation timeout; per-project locking; ownership enforcement; event schema versioning.

---

## 10. STATE MACHINE AND START/STOP SEMANTICS

```text
STOPPED -> STARTING -> RUNNING
                  \-> DEGRADED
                  \-> FAILED
RUNNING/DEGRADED -> STOPPING -> STOPPED
FAILED -> STOPPING/STOPPED or explicit RETRY -> STARTING
```

### Project startup algorithm

1. Obtain exclusive per-project operation lock; reject/return existing matching idempotent request.
2. Load + validate manifest; capability and permission checks.
3. Build dependency DAG; reject cycles and unresolved dependencies.
4. Assign isolated instance paths, ports, identities, env and provider resources.
5. Start DB/cache/mail/queue dependencies and wait for readiness/healthchecks.
6. Start runtime application/web engine; validate readiness.
7. Add route to shared Caddy routing table atomically; configure HTTPS as allowed.
8. Persist observed RUNNING/DEGRADED status, broadcast event and log operation result.
9. On failure, stop newly created resources safely, preserve data, report concrete failure and remediation.

### Project stopping algorithm

1. Lock project operation and mark STOPPING.
2. Remove/disable routing entry.
3. Stop foreground app and workers gracefully, then auxiliary services, databases last.
4. Enforce configured timeout; report failed stop and allow explicit force-stop.
5. Preserve persistent storage/credentials/backups. Mark STOPPED only when verified.
6. Never stop any process/volume not owned by this project.

### Crash/restart recovery

- Persist intended state and operation journal.
- Detect orphaned/owned processes using more than PID alone (PID can be reused), including owned metadata/launch token when feasible.
- Reconcile after daemon restart; do not automatically restart all STOPPED projects.
- Add restart backoff, limit, and user-visible reasons.
- Process group and child process termination must be OS-specific and tested.

---

## 11. DATA MODEL / PERSISTENCE

Use SQLite with explicit migrations. Suggested entities:

| Entity/table | Purpose |
|---|---|
| `projects` | project registration + manifest/version |
| `service_instances` | project-scoped desired service specs |
| `runtime_installations` | installed immutable runtime distributions |
| `package_artifacts` | signed/hash-verified artifact metadata |
| `port_reservations` | project-specific port allocation, lease state |
| `operations` | accepted/running/succeeded/failed/cancelled work |
| `project_events` | sequencing and event replay |
| `observed_processes` | owned process/container reconciliation handles |
| `backup_records` | snapshot/backup metadata, integrity, retention |
| `security_findings` | findings + provenance/freshness |
| `ai_analyses` | minimal audit metadata, privacy-managed |
| `ai_repair_plans` | proposed actions, approvals and execution results |
| `settings` | app config, capability/feature flags |

**Important:** Never put plaintext credentials/tokens in SQLite, logs or manifests. Secret references point to platform credential storage or an encrypted secrets implementation. Retention, backup encryption and export policies are separate.

**Data layout (logical):**

```text
<OS application data directory>/devforge/
  config/
  state/devforge.sqlite
  runtimes/<runtime>/<version>/<arch>/
  packages/manifests/
  packages/downloads/
  instances/<project-id>/
    services/
    data/
    generated-config/
    logs/
    runtime-state/
  backups/
  certificates/
  diagnostics/
```

Projects themselves may live anywhere the user authorizes. App uninstall must offer preserve-data default.

---

## 12. DOMAIN, TLS AND WEB-SERVER SWITCHING

- Use `*.test`-style names with exact project route mappings, not silently promise wildcard DNS on every platform.
- Manage hosts entries or a local DNS resolver with elevation when necessary. Validate restoration/removal and conflicts with VPN/corporate DNS.
- Use Caddy as central routing/HTTPS gateway, with *per-project* Nginx or Apache (or framework's built-in server) behind it as appropriate.
- Ensure route generation and switch are atomic and revocable. Cert trust must be explicit and OS-aware.
- Web-server switching: validate differences (`.htaccess`, rewrite rules, proxy headers, PHP handling), generate config diff, preview plan, stage, healthcheck, switch, rollback config on failure.
- Multiple projects run concurrently with independent hosts/ports and names.
- Default loopback access; external LAN/tunnel access only after user opt-in and warnings.

---

## 13. PACKAGES, UPDATES, BACKUPS AND ROLLBACKS

### Package registry model

- Engine, version, supported OS/arch, minimum dependencies, origin/source, checksum, optional publisher signature, license, support/EOL status, install recipe and healthcheck definition.
- Version catalog and install artifacts are independent. No downloading arbitrary shell scripts to execute without verification/approval.
- Download to staging, verify checksum/signature, extract safely (protect traversal/archive bombs), atomically move into versioned cache.
- Keep old installed versions until unused and explicitly cleaned.

### Managed update plan

1. Discover available version and advisory data.
2. Compare per-project pinned current version.
3. Check framework, ABI, extensions, dependency and storage compatibility.
4. Produce plan with expected changes, downtime, risk, backup requirement, tests and rollback feasibility.
5. Ask explicit approval before disruptive update or config change.
6. Back up consistent data/state, test checksum, apply staged version.
7. Run healthcheck/smoke tests and log result.
8. Roll back if supported; if data migrated irreversibly, use validated restore process or halt with recovery instructions.

Automatic background **check** is acceptable with user policy; automatic **apply** is not the default, especially for database major upgrades.

---

## 14. CLOUD AI SECURITY, CONTEXT AND REPAIR PROTOCOL

AI must be useful, transparent and safely constrained.

### Architecture

```text
User request / detected issue
  -> Local deterministic collectors (versions, logs, errors, scanner findings)
  -> Policy + secret redaction + consent + max-context limits
  -> Cloud AI provider adapter (HTTPS, timeout, rate limit)
  -> Structured analysis + proposed RepairPlan
  -> Schema validation + local policy checks + diff preview
  -> Explicit user approval for each risky action
  -> Typed allowlisted executor + backup/test/verification/audit
  -> Success or safe rollback/clear human recovery path
```

### AI scopes

- `logs`
- `security`
- `compatibility`
- `configuration`
- `performance`
- `update_planning`

### Representative request type

```typescript
type AIAnalysisRequest = {
  projectId: string;
  scope: "logs" | "security" | "compatibility" | "configuration" | "performance" | "update_planning";
  contextPolicy: {
    includeSourceCode: boolean; // false by default
    includeLogSamples: boolean;
    redactSecrets: true;
    maxContextBytes: number;
  };
  userInstruction?: string;
};
```

### Repair plan type

```typescript
type AIRepairPlan = {
  id: string;
  projectId: string;
  findings: Array<{
    severity: "info" | "low" | "medium" | "high" | "critical";
    description: string;
    evidence: string[];
    source?: string;
  }>;
  actions: Array<{
    type: "edit_config" | "change_runtime" | "restart_service" | "update_dependency" | "apply_patch";
    target: string;
    preview: string;
    requiresApproval: true;
  }>;
  requiresBackup: boolean;
  verificationSteps: string[];
  rollbackPlan: string;
};
```

### Nonnegotiable AI controls

- No untrusted cloud/model output may request arbitrary code execution with implicit approval.
- Never expose API keys, private keys, passwords, raw `.env`, full database dumps, or undisclosed source code to the AI provider.
- Model does not decide the level of its own permissions. Policies are enforced entirely in local trusted code.
- Logs/website snippets/files are untrusted inputs, not instructions to the local tool executor.
- AI repair must show exact target resource and diff. File edits require backup and project-root path checks; DB operations require special approval.
- Allow users to disable AI, revoke API credentials, clear stored analysis, and inspect approximate request size/cost where available.
- Security scanning is deterministic and evidence-based. AI may explain/adapt findings but may not label invented CVEs as verified.
- If cloud is unavailable, show error and preserve native orchestration functionality.

---

## 15. SECURITY BASELINE

**Threat model:** malicious project manifest, dependency scripts, compromised downloaded binary, untrusted container image, other local user, network attacker, malicious log contents/prompt injection, compromised cloud AI output, accidental destructive upgrade.

Controls:

- Least privilege: desktop and daemon run as normal user; narrowly scoped elevated helper for hosts/trust/service installation only.
- OS-local IPC with strict filesystem permissions/ACL; never unauthenticated public RPC endpoint.
- Explicit separation between user-selected project execution and privileged host mutations.
- Per-project secrets stored securely, redacted from logs/AI and protected in backup/export.
- Verify artifact hashes and signatures where possible; manifest provenance and pinned sources.
- Process argv arrays and safe config rendering; avoid implicit shell interpolation.
- Restricted file path handling, symlink/hardlink attack consideration, safe archive extraction.
- Network services bind to loopback/private container network by default.
- User approval and backup before destructive changes; clear audit log of applied operations.
- Respect third-party licensing/redistribution rights, including database images and binaries.
- Signed desktop releases and updates; no false claim of tamper resistance without completed pipeline.

---

## 16. USER INTERFACE / UX SPECIFICATION

**Design:** premium, modern developer tooling; fast, accessible, dark/light themes, compact but readable; avoid clutter. Real measured status; no fake data in production mode.

### Navigation

1. **Dashboard:** running projects, system overview, CPU/RAM/disk/ports, recent operations, alerts.
2. **Projects:** create/import, start/stop, open URL/editor, live logs, individual configuration.
3. **Runtimes:** installed/available PHP, Node, .NET, Go, Python, Rust versions, update actions.
4. **Services:** Nginx, Apache, Caddy, MongoDB, MySQL, MariaDB, PostgreSQL, Redis, Mailpit, etc.; per-project associations and state.
5. **Database Center:** connection details, backups, diagnostics and basic explorer (phase-based).
6. **Mail Center:** project-specific Mailpit inbox and message preview.
7. **Security & Updates:** dependency inventory, findings, supported upgrades, approval plans.
8. **AI Guardian:** chat-style diagnostics, evidence, repair plan preview, approval/run outcomes.
9. **Settings:** OS integration, ports/domain/TLS, proxy, AI provider/credentials, updates, backups, logs, platform capabilities.

### Project card

- Name, framework icon/label, domain, runtime & version, provider, running/starting/failed, key services, start/stop and open-in-browser.
- Clear loading/progress feedback while operation is underway; disable conflicting actions.
- UI should receive true daemon state via IPC events and reconnect reconciliation.

### Project detail tabs

Overview, Services, Runtimes, Web & Domain, Databases, Cache & Queue, Mail, Environment, Logs, Updates & Security, AI Diagnosis, Backup & History.

### Accessibility/localization

- Keyboard-operable controls, contrast, screen-reader labels, notification nondependence on color.
- Prepare i18n infrastructure; English/Turkish first. Avoid hardcoded UI strings where feasible.

---

## 17. VERIFICATION / TEST STRATEGY

- **Unit:** manifest parsing/validation, DAG validation, state transition rules, port allocation, data model migration, error mapping.
- **Integration:** native provider process launch/stop, ownership and timeout, daemon IPC, event replay, SQLite re-open/reconcile.
- **Container integration:** opt-in CI environment with Docker/Podman available; test volume/network isolation and cleanup.
- **E2E UI:** create/import example project, start/stop, status, logs, error visibility.
- **Cross-platform:** Windows/macOS/Linux CI compilation and smoke tests; real-host manual validation for elevation, hosts/DNS, cert trust and uninstall.
- **Security:** unauthorized IPC attempt, path traversal, malicious manifest, injection in logs/AI output, archive traversal, secret redaction, plugin/package integrity.
- **Recovery:** daemon crash, UI close/restart, computer restart, process orphan, failed healthcheck, port conflict, interrupted update, corrupted package.
- **Database:** consistent backup and tested restore for supported engines, no accidental data deletion or unsafe downgrade assumptions.
- **Performance:** define measured startup/memory budgets after first working build; do not invent performance figures.

**Definition of done for any feature:** unit/integration tests at appropriate level, useful error states, actual data wiring, docs and compatibility flags. Do not mark feature complete on mock UI alone.

---

## 18. SPRINT 01 — 10 WORKDAYS / FOUNDATION & LIFECYCLE

**Goal:** deliver a working cross-platform desktop app talking to a separate Rust daemon that can validate, start, observe, and stop one isolated sample HTTP project.

**Not required this sprint:** full PHP/MySQL/MongoDB/Mailpit/cloud AI, production installer, privileged system integration, full container engine. These are future phases.

| Task | Estimate | Acceptance criteria |
|---|---:|---|
| DF-001 Monorepo + Rust workspace + Tauri/React scaffold | 8h | Cargo/pnpm workspace builds; lint/format commands documented |
| DF-002 Windows/macOS/Linux CI build matrix | 8h | Builds and base tests run on all three OS targets |
| DF-003 Independent daemon | 12h | health, single-instance lock, clean shutdown and structured logs |
| DF-004 Authenticated local IPC | 14h | UI can call health/capabilities, unauthorized access rejected |
| DF-005 SQLite migrations | 8h | Projects/operations persist, migration idempotent |
| DF-006 Manifest schema + parser | 10h | Valid example passes; malformed/unsafe manifest errors well defined |
| DF-007 Native provider PoC | 14h | Start small HTTP child, inspect health/PID safely, stop child/process group |
| DF-008 Lifecycle engine PoC | 12h | STOPPED/STARTING/RUNNING/STOPPING; idempotence; failed start cleanup |
| DF-009 Desktop project cards | 10h | Real daemon-backed project list, start/stop and states |
| DF-010 Events/log streaming | 6h | Live events/logs; reconnect resynchronizes |
| DF-011 Unit/integration tests | 6h | Start/stop/conflict/failure test cases pass |
| DF-012 Demo/docs | 4h | Local run instructions and demonstrated E2E flow |
| **TOTAL** | **112h** | Estimated person-hours, **not** guaranteed calendar delivery |

### Sprint 01 file-level action plan

1. Initialize workspace and `apps/desktop` Tauri 2 app.
2. Create `crates/core` domain types and status enums; `crates/ipc` request/response/event schemas.
3. Create `crates/daemon` with boot/health/shutdown, local endpoint and SQLite state.
4. Create `crates/persistence` migrations and repositories for projects/operations.
5. Implement `schemas/project-manifest.schema.json` with at least sample runtime and lifecycle fields.
6. Implement `crates/provider-native` to spawn a **safe sample HTTP executable** with current OS process controls; never use shell interpolation.
7. Add `crates/orchestrator` start/stop with per-project lock, event feed and real healthcheck.
8. Add React dashboard and project card that calls IPC (no fake success, all statuses real).
9. Build `examples/hello-http`, ideally a small Rust or equivalent portable deterministic sample with local bind.
10. Add CI matrix, docs and tests.

### Sprint 01 E2E acceptance scenario

1. Launch desktop app; daemon starts or is discovered and IPC handshake succeeds.
2. `hello-http` project is registered/listed.
3. Press Start: manifest validates and only this example project's child process starts.
4. UI shows STARTING then RUNNING after actual healthcheck.
5. HTTP endpoint responds on its local assigned port.
6. Press Start again: no duplicate instance.
7. Press Stop: process group is terminated and status becomes STOPPED.
8. Press Stop again: idempotent result.
9. Quit/reopen UI: daemon state is accurately reflected.
10. Simulate failed executable/port conflict: meaningful failure shown, no zombie processes, unrelated projects untouched.

### Suggested calendar ordering (parallelizable team)

- Days 1–2: DF-001/002/003
- Days 3–4: DF-003/004/005
- Days 5–6: DF-006/007
- Days 7–8: DF-008/009
- Day 9: DF-010/011
- Day 10: DF-012 plus cross-platform verification

---

## 19. FORWARD ROADMAP (PHASED, NOT ALL AT ONCE)

### Sprint 02 — Hybrid provider and isolation

- Container provider with capability detection; independent project networks/volumes and cleanup guards.
- Native/container ownership and port allocator hardening.
- Service dependency DAG; explicit provider fallback/unsupported UX.
- Provider integration tests and daemon recovery reconciliation.

### Sprint 03 — Runtime + web foundation

- PHP + Node.js + .NET adapters and project templates first; Go and Python next, then Rust and more via adapters.
- Caddy router, local project domains/HTTPS permissions flow.
- Nginx/Apache switch plans, config validation, staged restart; meaningful `.htaccess` compatibility warnings.
- Version catalog/download/cache.

### Sprint 04 — Isolated databases, cache, Mailpit

- MySQL/MariaDB/PostgreSQL, MongoDB as platform-dependent native/container providers.
- Redis, Mailpit per-project instances, persistent volumes, SMTP environment integration and basic UI mail inbox.
- Connection detail secret handling, backup/restore for initial supported engines.

### Sprint 05 — Updates, safety and diagnostics

- Artifact registry verification, update detection/compatibility plans.
- Service/runtimes version switching; backups/snapshots, health verification and engine-aware rollback.
- OSV/official advisory lookup and optional Trivy scanner.
- Logs/metrics/diagnostics dashboards, export sanitized reports.

### Sprint 06 — Cloud AI Guardian

- Configurable provider & secret storage, consent UI, redaction and context budget.
- Security/error/compatibility analysis and schema-validated repair plans.
- Diff previews, approval tokens, allowlisted local action executor, audit trail and verified outcomes.
- Prompt injection, permission abuse, cost and secret-leak tests.

### Later releases

- More languages, runtimes and service adapters; templates and extension SDK.
- Environment export/import and reproducing environments.
- Queue/scheduler manager, API testing, DB explorer, network share/tunnel, performance optimizer.
- Signed plugin marketplace after threat model and permission system mature.

**Planning note:** The entire requested system is a multi-month/multi-release undertaking. Prioritize a reliable cross-platform lifecycle engine before broad integrations. Rough full-beta planning assumption for an experienced 4–6-person team: many months (approximately 8–12+), depending on coverage/OS variance; treat this as an estimate, not a promise.

---

## 20. ACCEPTANCE MATRIX FOR THE FINISHED PRODUCT

| Requirement | Acceptance |
|---|---|
| Cross-platform | Supported OS/arch matrix published with tested installers |
| Project isolation | Two projects using the same database version have separate instances/data/ports |
| On-demand lifecycle | App launch does not start all projects; start A affects only A and shared proxy route |
| Mixed providers | Project can run native runtime and container DB if supported |
| Apache/Nginx | Switch is validated/previewed; incompatible rewrite semantics clearly warned |
| Language support | PHP, Node, .NET, Go, Python and Rust have documented, functional sample launches |
| DB catalog | MySQL/MariaDB/PostgreSQL/MongoDB/SQLite supported according to capability matrix |
| Mailpit | Project-specific SMTP catch and UI message inspection works |
| Version management | Version check, side-by-side install, compatibility report, staged switch |
| AI Guardian | Cloud-only AI analysis is consented/redacted; changes require local validation/approval |
| Security checks | Findings cite real advisory data, not hallucinated claims |
| Reliability | Crash, failed healthcheck, port collision and restart recovery tested |
| Data safety | App uninstall and failed update do not silently delete user data |
| Extensibility | New runtime adapter integrates without rewriting orchestrator |

---

## 21. NONFUNCTIONAL REQUIREMENTS AND OPEN DECISIONS

### Requirements

- User data must be preserved through normal upgrades/uninstalls unless explicitly deleted.
- Operations must be observable, cancellable where feasible and safe to retry.
- Respect local CPU/RAM/disk limits with enforceable provider/OS capabilities; show unsupported controls transparently.
- App usable without constant internet, except AI/cloud data and updates.
- No silent global environment PATH changes or uncontrolled privilege escalation.
- Provide English and Turkish localization infrastructure.
- UI must handle daemon disconnected/offline/permission-required states without crashes.

### Engineering decisions to record as ADRs before broad rollout

- Supported minimum OS versions and initial CPU architectures.
- Exact signed artifact distribution sources and license obligations.
- Packaging and startup mechanism for independent daemon on each platform.
- Caddy + hosts vs dedicated DNS proxy evolution.
- How to implement project-scoped secret storage and backup encryption.
- Which container provider capabilities are guaranteed per OS.
- Cloud AI vendors/models, data retention policy, user-consent UX and budget limits.
- Runtime ABI/extension compatibility and persistent database upgrades per engine.
- Feature flags for partial support and experimental engines.

---

## 22. CODEX FIRST COMMAND / STARTING PROMPT

After reading this document, Codex should proceed as follows:

> You are the lead engineer for DevForge. Read `DEVFORGE_CODEX_MASTER_SPEC.md` as the product and architecture specification. Inspect the existing repository before making changes. Implement **Sprint 01 only** as a working vertical slice, preserving all final architectural decisions: Tauri 2 + React/TypeScript UI, Rust daemon and authenticated IPC, SQLite state, project manifest schema, native process provider, lifecycle state machine, real project start/stop and log/status events, cross-platform build matrix and tests. Do not fake functionality. Do not begin full database, container, cloud AI or Mailpit implementations until the core vertical slice is functioning. Keep source code modular, record decisions, run available tests, and report completed work, test results and blockers. If the repository is empty, create the monorepo structure and scaffold the minimal implementation.

### Requested Codex response format after each batch

1. **Implemented:** concrete source files and features.
2. **Verified:** exact commands/tests, pass/fail and host platforms covered.
3. **Known limitations:** not-built/unverified features, not hidden.
4. **Next tasks:** smallest coherent next implementation batch.

---

## 23. FEATURE CHECKLIST (TRACK AS ISSUES)

### Core and platform
- [ ] Three OS installers/builds and capability matrix
- [ ] Independent Rust daemon
- [ ] Secure local IPC and event stream
- [ ] SQLite migrations and operation journal
- [ ] Native process provider
- [ ] Docker provider
- [ ] Podman provider (tested capability subset)
- [ ] Project-specific process/data/network isolation
- [ ] Dependency DAG + healthchecks
- [ ] Safe daemon recovery and orphan process handling
- [ ] System tray and updater

### Developer tooling
- [ ] PHP + Laravel/WordPress/Symfony support
- [ ] Node.js + Next/Express/Nest support
- [ ] .NET + ASP.NET Core support
- [ ] Go application support
- [ ] Python + Django/FastAPI support
- [ ] Rust web app support
- [ ] Custom launch profiles
- [ ] Multiple installed versions per runtime
- [ ] Per-project pinned versions
- [ ] Project template/import/export

### Web, data and messaging
- [ ] Caddy global reverse proxy
- [ ] Per-project Nginx
- [ ] Per-project Apache
- [ ] Server switching preview and validation
- [ ] Local `.test` domain management
- [ ] Local HTTPS and certificate trust flow
- [ ] MySQL and MariaDB
- [ ] PostgreSQL
- [ ] MongoDB
- [ ] SQLite project tools
- [ ] Redis/Valkey
- [ ] Mailpit isolated per project and mail UI
- [ ] Queue worker and scheduler tools
- [ ] Optional RabbitMQ/MinIO/search services

### Updates, safety and AI
- [ ] Version/advisory discovery
- [ ] Download/hash/signature verification
- [ ] Compatibility checks and update plans
- [ ] Consistent backup + tested restore
- [ ] Security scanning and findings provenance
- [ ] Cloud AI provider settings and credential storage
- [ ] Secret redaction, consent and cost guardrails
- [ ] AI analysis and structured repair proposals
- [ ] Explicit approval/diff and allowlisted action executor
- [ ] AI action auditing, verification and recovery

### UX
- [ ] Real live dashboard and service controls
- [ ] Project details and isolated service settings
- [ ] Runtime version manager
- [ ] DB center and Mailpit inbox
- [ ] Logs, diagnostics, alerts
- [ ] Security/update dashboard
- [ ] AI Guardian panel
- [ ] Turkish/English localization
- [ ] Dark/light modes and accessibility checks

---

**END OF MASTER SPECIFICATION.**
