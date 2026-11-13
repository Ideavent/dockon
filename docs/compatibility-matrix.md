# Compatibility

| Capability | Windows x64 | macOS | Linux |
|---|---|---|---|
| Rust daemon / desktop | Local native UI E2E and CI pass | Foundation CI passed | Foundation CI passed |
| IPC | Named Pipe, authenticated and streamed | Unix socket streaming CI passed | Unix socket streaming CI passed |
| Native lifecycle | Job Object lifecycle/tree/crash tests pass | Foundation process group CI passed; new runner pending | Foundation process group CI passed; new runner pending |
| Docker generic provider | Local mixed/isolated Redis + HTTP tests pass | Not host-tested | CI pending |
| Podman generic provider | Unavailable, unverified | Unverified | Unverified |
| Managed DB catalog, mail, AI | Not implemented | Not implemented | Not implemented |
| Signed installer/updater | Not implemented | Not implemented | Not implemented |

CI configuration alone is not evidence that remote platform builds passed.
