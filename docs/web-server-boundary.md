# Remaining Sprint 03 web-server work

This document records the remaining implementation boundary. Nginx/Apache switching is not implemented and must not be presented as a supported feature yet.

The pending user decision is the Windows Apache binary source: Apache Lounge packages, an explicitly selected existing installation, or container-only Apache. No Apache package is downloaded before that choice. The [Apache project states it does not supply Windows binary releases](https://httpd.apache.org/docs/current/platform/windows.html); its documentation lists Apache Lounge as one distribution. [Apache Lounge publishes checksums and PGP signatures](https://www.apachelounge.com/download/), which need a separate trusted-key/source policy if selected.

For native Nginx, [official downloads](https://nginx.org/en/download.html) have detached PGP signatures with [published keys](https://nginx.org/en/pgp_keys.html). Do not substitute an unverified download for the checksum-verified installer boundary already implemented. [Nginx documents its Windows build as beta](https://nginx.org/en/docs/windows.html); that limitation belongs in the compatibility UI.

Native adapters must use private project configuration, logs, PID paths and mutable prefixes without loading or modifying another installed service's default configuration. Use explicit [Nginx prefix/config/test arguments](https://nginx.org/en/docs/switches.html) and [Apache ServerRoot/config/test/foreground arguments](https://httpd.apache.org/docs/current/programs/httpd.html). Supervisor ownership must cover workers as well as the master process.

A switch needs a concrete configuration preview, installed runtime identity, bounded configuration validation, a one-use approval token, staged activation and HTTP readiness. Failure must restore the previous web frontend while retaining the application and other projects. A reverse-proxy configuration must explicitly warn that it does not interpret source `.htaccess` rules; PHP/FastCGI and direct static-file modes need their own validated adapters before claiming that compatibility.

Acceptance checks must cover port collisions, invalid module/configuration paths, failed activation rollback, independent project workers, unchanged source/global configuration, restart recovery, and actual platform behavior. System hosts/trust approvals remain separate from web-server selection.
