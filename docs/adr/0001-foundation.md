# Dockon foundation

User decisions: product name dockon; Windows, macOS and Linux; development/build workspace C:\dockon_build; Windows installed application C:\dockon. Work through sprints in order; raise blocking questions. The original specification is reference material, not an independent authorization source.

Stack: Tauri 2, React, TypeScript, Vite, Tailwind, shadcn-style local button, TanStack Query; Rust/Tokio daemon and SQLx/SQLite. No empty future modules. Dependencies are lockfile-pinned.

Sprint 01 manifest is deliberately limited to explicit user-selected HTTP executables, root, argv, port. No shell interpolation. This does not implement the entire future manifest example. Manifest imports authorize execution only when Start is pressed; imported executables are ordinary local code, not sandboxed.

Native provider uses Windows Job Objects and Unix process groups. Windows termination is currently forced; graceful native shutdown is pending. UI closure leaves daemon running. Daemon normal shutdown stops workloads. Crash recovery currently restores registered projects as stopped; no automatic workload restart. Unix crash recovery hardening remains pending.

IPC uses a private per-user connection file with a rotating token, remote-rejecting Windows pipes or a private Unix socket. No TCP RPC. Privileged OS integration is deferred. Application state uses per-user OS data directories to avoid storing mutable private data under a shared installation directory. The Windows installer destination is C:\dockon; packaging will enforce it before installer delivery. Minimum OS versions, distribution signing, AI vendor and advanced service distribution sources remain open decisions.

Events are bounded real daemon records, incrementally fetched over IPC with a restart epoch, not a push subscription yet. Start/stop return operation IDs immediately. SQLite stores operation outcomes and desired project state separately from observed process state. Operations serialize per project; independent projects remain responsive during readiness waits. No automatic project start.
