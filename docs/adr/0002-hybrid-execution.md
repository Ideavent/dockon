# Hybrid execution and ownership

Sprint 02 adds a Docker CLI adapter and a Podman-compatible CLI adapter. Docker is tested locally on Docker Desktop with a Linux engine; Podman is unavailable here and must remain explicitly unverified. Native execution has no Docker dependency.

Manifests may add a container provider, digest-pinned image reference, container command/entrypoint, HTTP/TCP healthcheck and auxiliary services with explicit dependencies. Schema version 1 receives optional additive fields; existing manifests remain readable. Service IDs are lowercase ASCII to avoid DNS alias collisions. Working directories are canonicalized and constrained to the project root.

The dependency DAG is validated before execution. Dependencies start in topological order, application last; stop runs in reverse. Failure cleans up resources created for that project. Native processes receive isolated data directories and a `DOCKON_SERVICES` JSON environment value. Native applications may use container dependencies through published loopback ports. Container applications use project-network DNS aliases. Container-to-native and cross-engine container networking are rejected until appropriate platform adapters exist.

Persistent installation identity, project identity and instance identity labels guard every stop/remove. Networks are per project, persistent volumes per instance. Lifecycle never deletes volumes. Existing resources with mismatched labels are rejected. Containers use loopback-only published ports, a read-only root filesystem, dropped capabilities, no host mounts/socket and a PID ceiling. These settings are not a promise of isolation from an actively compromised container engine.

Daemon restart reconciles only containers matching the current installation and registered project; persistent volumes remain. If a provider is unavailable, the project is failed with a recovery reason rather than reported stopped. Empty labeled networks and data volumes are intentionally retained for future reuse/data management; general garbage collection is not implemented.

Windows native workloads use Job Objects. Windows generic graceful signaling remains unsupported and force termination is disclosed in events. Unix uses an internal `dockon-runner`: a private parent-owned control pipe detects daemon death, and the runner terminates/reaps its own application process group. This avoids identifying an orphan by PID alone. Native execution is not a hostile-code sandbox; self-detaching processes are outside this guarantee.

No images are redistributed in the installer. Integration samples download publisher images by digest on demand and retain the image cache. The test sample http-echo is from https://github.com/hashicorp/http-echo (MPL-2.0); the Redis test image is from https://hub.docker.com/_/redis. Release catalogs and redistribution review remain Sprint 03/04/05 work.
