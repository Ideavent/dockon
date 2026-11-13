# Delivery status

Work is in Sprint 01. Later sprints must not be called complete before their implementations and acceptance checks pass.

| Sprint | Status |
|---|---|
| 01 foundation and lifecycle | Windows host checks pass; current cross-platform CI verification in progress |
| 02 hybrid provider and isolation | Pending |
| 03 runtimes and web | Pending |
| 04 databases, cache, mail | Pending |
| 05 updates, backup, diagnostics | Pending |
| 06 cloud AI | Pending |

Unimplemented capabilities return not_implemented from health. No production installer or signed release exists yet.

Local checks passed: workspace build; Rust tests including malformed/oversized IPC frames, coalesced stream frames and SQLite recovery; warning-free Clippy; frontend production build; native lifecycle smoke; event subscription; browser localization/offline checks; real Windows Tauri/WebView2 import/start/HTTP/stop flow. Remote baseline macOS checks passed; Windows and Linux jobs still running at this update.
