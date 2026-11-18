import assert from "node:assert/strict";
import { DaemonHarness, pause } from "./lib/daemon-harness.mjs";
import { createServer } from "node:net";
import { request as httpsRequest } from "node:https";
import { request as httpRequest } from "node:http";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
async function freePort() {
  const server = createServer();
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const port = server.address().port;
  await new Promise((r) => server.close(r));
  return port;
}
const h = await new DaemonHarness().initialize("router-");
const hostsPath =
  process.platform === "win32"
    ? "C:/Windows/System32/drivers/etc/hosts"
    : "/etc/hosts";
const originalHosts = await readFile(hostsPath);
let conflict;
try {
  await h.launch();
  let caddy;
  if (process.env.DOCKON_CADDY_EXECUTABLE)
    caddy = await h.call("runtime_use_system", {
      runtime: "caddy",
      executable: resolve(process.env.DOCKON_CADDY_EXECUTABLE),
    });
  else {
    const catalog = await h.call("runtime_catalog", { runtime: "caddy" });
    const op = await h.wait(
      (
        await h.call("runtime_install", {
          runtime: "caddy",
          version: catalog.versions[0].version,
        })
      ).id,
    );
    assert.equal(op.status, "completed", op.error);
    caddy = (await h.call("runtime_installed")).find(
      (r) => r.runtime === "caddy",
    );
  }
  for (const id of ["route-a", "route-b"]) {
    assert.equal((await h.register(h.manifest(id))).ok, true);
    await h.action("start", id);
  }
  const settings = {
    runtime_installation: caddy.id,
    http_port: await freePort(),
    https_port: await freePort(),
    routes: [
      { project_id: "route-a", domain: "first.test", https: false },
      { project_id: "route-b", domain: "custom.example.internal", https: true },
    ],
  };
  const plan = await h.call("router_plan", { settings });
  assert.equal(plan.trust_store_changed, false);
  assert.match(plan.after, /skip_install_trust/);
  assert.match(plan.after, /admin off/);
  assert.equal(plan.hosts_entries.length, 2);
  assert.equal((await h.call("router_status")).running, false);
  let status = await h.call("router_apply", { token: plan.token });
  assert.equal(status.running, true);
  assert.equal((await h.rpc("router_apply", { token: plan.token })).ok, false);
  const http = () =>
    new Promise((resolve, reject) => {
      const request = httpRequest(
        {
          hostname: "127.0.0.1",
          port: settings.http_port,
          path: "/health",
          headers: { Host: "first.test" },
          timeout: 3000,
        },
        (response) => {
          let body = "";
          response.on("data", (chunk) => (body += chunk));
          response.on("end", () =>
            resolve({ status: response.statusCode, body }),
          );
        },
      );
      request.on("error", reject);
      request.on("timeout", () =>
        request.destroy(new Error("HTTP request timed out")),
      );
      request.end();
    });
  assert.equal((await http()).status, 200);
  const ca = await readFile(status.root_certificate);
  const hostsPreview = await h.call("system_plan", { action: "hosts_update" });
  assert.equal(
    hostsPreview.target.replaceAll("\\", "/").toLowerCase(),
    hostsPath.toLowerCase(),
  );
  assert.match(hostsPreview.after, /127\.0\.0\.1 first\.test/);
  const trustPreview = await h.call("system_plan", { action: "trust_ca" });
  assert.equal(trustPreview.fingerprint.length, 64);
  if (process.platform === "win32") assert.match(trustPreview.before, /false/);
  const forged = await h.wait(
    (await h.call("system_apply", { token: "not-an-approved-token" })).id,
  );
  assert.equal(forged.status, "failed");
  const tls = (trusted = true) =>
    new Promise((resolve, reject) => {
      const request = httpsRequest(
        {
          hostname: "127.0.0.1",
          port: settings.https_port,
          path: "/health",
          servername: "custom.example.internal",
          headers: { Host: "custom.example.internal" },
          ...(trusted ? { ca } : {}),
          timeout: 3000,
        },
        (response) => {
          response.resume();
          resolve(response.statusCode);
        },
      );
      request.on("error", reject);
      request.on("timeout", () =>
        request.destroy(new Error("TLS request timed out")),
      );
      request.end();
    });
  assert.equal(await tls(), 200);
  await assert.rejects(tls(false));
  console.log(
    "PASS real Caddy HTTP and CA-verified HTTPS routing, custom domains, one-use preview approval and untouched trust configuration",
  );
  const b = (await h.call("list")).find((p) => p.manifest.id === "route-b");
  const stale = await h.call("router_plan", { settings });
  await h.action("stop", "route-a");
  assert.equal((await h.rpc("router_apply", { token: stale.token })).ok, false);
  for (let i = 0; i < 100; i++) {
    try {
      if ((await http()).status === 503) break;
    } catch {}
    await pause(100);
  }
  assert.equal((await http()).status, 503);
  assert.equal(await tls(), 200);
  assert.equal(
    (await h.call("list")).find((p) => p.manifest.id === "route-b").pid,
    b.pid,
  );
  console.log(
    "PASS stopped route becomes 503 while the other project and HTTPS route remain available; stale plan rejected",
  );
  conflict = createServer();
  conflict.on("connection", (socket) => socket.destroy());
  await new Promise((r) => conflict.listen(0, "127.0.0.1", r));
  const broken = await h.call("router_plan", {
    settings: { ...settings, http_port: conflict.address().port },
  });
  assert.equal(
    (await h.rpc("router_apply", { token: broken.token })).ok,
    false,
  );
  assert.equal((await http()).status, 503);
  assert.equal(await tls(), 200);
  await h.call("router_stop");
  assert.equal((await h.call("router_status")).running, false);
  await h.shutdown();
  await h.launch();
  status = await h.call("router_status");
  assert.equal(status.running, false);
  assert.equal(status.settings.routes[0].domain, "first.test");
  await h.shutdown();
  assert.deepEqual(await readFile(hostsPath), originalHosts);
  console.log(
    "PASS failed router restart rolls back, persisted settings do not autostart, and system hosts file is unchanged",
  );
} finally {
  if (conflict) await new Promise((r) => conflict.close(r));
  await h.shutdown();
}
