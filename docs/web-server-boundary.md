# Remaining Sprint 03 web-server work

This document records the remaining implementation boundary. Nginx/Apache switching is not implemented and must not be presented as a supported feature yet.

Dockon uses Apache Lounge for managed Windows x64 installs. The catalog pins a release and the publisher's SHA-512 digest. Updating that release requires updating the pin. PGP verification is pending. Users can also register an existing Apache executable by absolute path. The Windows static site profile expects an Apache binary with the standard `bin`, `modules` and `conf` layout. Apache Lounge publishes release checksums and PGP signatures. [Apache Windows documentation](https://httpd.apache.org/docs/current/platform/windows.html) · [Apache Lounge downloads](https://www.apachelounge.com/download/)

The Windows static profile creates or imports a site with `public/index.html`. Apache serves selected static file types from `public` on loopback with project-specific configuration, PID and error log paths. PHP, FastCGI, `.htaccess` and live frontend switching remain pending. Generate an Apache Docker sample with `scripts/New-SampleManifest.ps1 -ApacheContainer` and import the resulting manifest to run the image's sample site.

For native Nginx, [official downloads](https://nginx.org/en/download.html) have detached PGP signatures with [published keys](https://nginx.org/en/pgp_keys.html). Do not substitute an unverified download for the checksum-verified installer boundary already implemented. [Nginx documents its Windows build as beta](https://nginx.org/en/docs/windows.html); that limitation belongs in the compatibility UI.

Native adapters must use private project configuration, logs, PID paths and mutable prefixes without loading or modifying another installed service's default configuration. Use explicit [Nginx prefix/config/test arguments](https://nginx.org/en/docs/switches.html) and [Apache ServerRoot/config/test/foreground arguments](https://httpd.apache.org/docs/current/programs/httpd.html). Supervisor ownership must cover workers as well as the master process.

A switch needs a concrete configuration preview, installed runtime identity, bounded configuration validation, a one-use approval token, staged activation and HTTP readiness. Failure must restore the previous web frontend while retaining the application and other projects. A reverse-proxy configuration must explicitly warn that it does not interpret source `.htaccess` rules; PHP/FastCGI and direct static-file modes need their own validated adapters before claiming that compatibility.

Acceptance checks must cover port collisions, invalid module/configuration paths, failed activation rollback, independent project workers, unchanged source/global configuration, restart recovery, and actual platform behavior. System hosts/trust approvals remain separate from web-server selection.
