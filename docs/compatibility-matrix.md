# Compatibility

| Capability | Windows x64 | macOS | Linux |
|---|---|---|---|
| Rust daemon / desktop | Local native UI E2E and CI pass | Foundation CI passed | Foundation CI passed |
| IPC | Named Pipe, authenticated and streamed | Unix socket streaming CI passed | Unix socket streaming CI passed |
| Native lifecycle | Job Object lifecycle/tree/crash tests pass | Ownership runner/tree/crash CI passed | Ownership runner/tree/crash CI passed |
| Docker generic provider | Local mixed/isolated Redis + HTTP tests pass | Not host-tested | Redis/HTTP isolation and crash recovery CI passed |
| Managed Node/PHP/.NET/Go/Caddy catalogs | Official downloads, digest and version checks passed for all five; Node CI passed | Managed Node installation/profile CI passed; managed PHP unsupported; other downloads unverified | Managed Node installation/profile CI passed; managed PHP unsupported; other downloads unverified |
| Runtime project wizard | Native WebView2 Node selection/create/start/stop checked | Node backend profile CI passed; native UI unverified | Node backend profile CI passed; native UI unverified |
| Native Node/PHP/.NET/Go project profiles | Scaffold/detection/HTTP/import/integrity rejection tests passed | Node profile CI passed; other profiles unverified | Node profile CI passed; other profiles unverified |
| Caddy local HTTP/HTTPS routing | Backend and native UI tests passed; cross-platform CI pending | Router CI pending | Router CI pending |
| Scoped hosts / CA trust | Previews and temporary hosts fixtures checked; live elevation/trust unverified | Hosts helper unverified; trust unsupported | Requires pkexec for hosts; live helper unverified; trust unsupported |
| Python venv / concrete Rust toolchain | Explicit system import only; profile tests pending | Explicit system import only; unverified | Explicit system import only; unverified |
| Podman generic provider | Unavailable, unverified | Unverified | Unverified |
| Managed DB catalog, mail, AI | Not implemented | Not implemented | Not implemented |
| Signed installer/updater | Not implemented | Not implemented | Not implemented |

CI configuration alone is not evidence that remote platform builds passed.
