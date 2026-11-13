import { execFileSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import assert from "node:assert/strict";
import { DaemonHarness, pause } from "./lib/daemon-harness.mjs";

const ECHO =
  "hashicorp/http-echo@sha256:fcb75f691c8b0414d670ae570240cbf95502cc18a9ba57e982ecac589760a186";
const REDIS =
  "redis@sha256:858f009f9709ce576febc734aa78b8f6d624b82571f9ddb6bda4377c833b3499";
const docker = (...args) =>
  execFileSync("docker", args, {
    encoding: "utf8",
    windowsHide: true,
    stdio: ["ignore", "pipe", "pipe"],
  }).trim();
const inspect = (kind, id) => JSON.parse(docker(kind, "inspect", id))[0];
const h = await new DaemonHarness().initialize("hybrid-");
let installation;
const redisService = {
  id: "cache",
  provider: "container",
  container: {
    engine: "docker",
    image: REDIS,
    internal_port: 6379,
    entrypoint: "redis-server",
    command: [
      "--dir",
      "/dockon-data",
      "--appendonly",
      "yes",
      "--appendfsync",
      "always",
      "--bind",
      "0.0.0.0",
    ],
  },
  health: { type: "tcp" },
};
try {
  await h.launch();
  installation = await readFile(join(h.data, "installation.id"), "utf8");
  const capabilities = await h.call("capabilities");
  assert.equal(capabilities.docker.available, true);
  assert.equal(typeof capabilities.podman.available, "boolean");
  for (const id of ["mixed-a", "mixed-b"])
    assert.equal(
      (await h.register(h.manifest(id, { services: [redisService] }))).ok,
      true,
    );
  const a = await h.action("start", "mixed-a");
  const b = await h.action("start", "mixed-b");
  assert.equal(a.services.length, 1);
  assert.equal(b.services.length, 1);
  assert.notEqual(a.services[0].port, b.services[0].port);
  const ca = inspect("container", a.services[0].container_id);
  const cb = inspect("container", b.services[0].container_id);
  const va = ca.Mounts.find((m) => m.Destination === "/dockon-data").Name;
  const vb = cb.Mounts.find((m) => m.Destination === "/dockon-data").Name;
  assert.notEqual(va, vb);
  assert.notDeepEqual(
    Object.keys(ca.NetworkSettings.Networks),
    Object.keys(cb.NetworkSettings.Networks),
  );
  assert.equal(ca.HostConfig.PortBindings["6379/tcp"][0].HostIp, "127.0.0.1");
  assert.equal(ca.HostConfig.ReadonlyRootfs, true);
  assert.equal(ca.HostConfig.Privileged, false);
  docker("exec", ca.Id, "redis-cli", "SET", "dockon-probe", "alpha");
  docker("exec", cb.Id, "redis-cli", "SET", "dockon-probe", "beta");
  assert.equal(
    docker("exec", ca.Id, "redis-cli", "GET", "dockon-probe"),
    "alpha",
  );
  assert.equal(
    docker("exec", cb.Id, "redis-cli", "GET", "dockon-probe"),
    "beta",
  );
  await h.action("stop", "mixed-a");
  assert.equal(
    docker("exec", cb.Id, "redis-cli", "GET", "dockon-probe"),
    "beta",
  );
  const restarted = await h.action("start", "mixed-a");
  assert.equal(
    docker(
      "exec",
      restarted.services[0].container_id,
      "redis-cli",
      "GET",
      "dockon-probe",
    ),
    "alpha",
  );
  console.log(
    "PASS mixed native/container execution, isolated networks/ports/data, persistent restart and independent stop",
  );
  const echoManifest = h.manifest("container-app", {
    provider: "container",
    executable: "",
    container: {
      engine: "docker",
      image: ECHO,
      internal_port: 5678,
      command: ["-text=dockon-container"],
    },
  });
  assert.equal((await h.register(echoManifest)).ok, true);
  const echo = await h.action("start", "container-app");
  assert.ok(
    (await (await fetch(`http://127.0.0.1:${echo.port}`)).text()).includes(
      "dockon-container",
    ),
  );
  assert.equal(echo.pid, null);
  assert.ok(echo.container_id);
  console.log(
    "PASS container application readiness and real container identity",
  );
  const cycle = h.manifest("cycle", {
    services: [
      {
        id: "one",
        executable: h.binary("dockon-hello-http"),
        depends_on: ["two"],
      },
      {
        id: "two",
        executable: h.binary("dockon-hello-http"),
        depends_on: ["one"],
      },
    ],
  });
  assert.equal((await h.register(cycle)).ok, false);
  const bad = h.manifest("rollback", {
    services: [
      { id: "first", executable: h.binary("dockon-hello-http") },
      {
        id: "second",
        executable: h.binary("dockonctl"),
        depends_on: ["first"],
      },
    ],
  });
  assert.equal((await h.register(bad)).ok, true);
  const failure = await h.wait((await h.call("start", { id: "rollback" })).id);
  assert.equal(failure.status, "failed");
  const failed = (await h.call("list")).find(
    (p) => p.manifest.id === "rollback",
  );
  assert.equal(failed.state, "FAILED");
  assert.equal(failed.services.length, 0);
  const serviceEvent = (await h.call("events", { after: 0 })).events.find(
    (e) =>
      e.project_id === "rollback" &&
      e.message.includes("[first] Dockon sample listening"),
  );
  assert.ok(serviceEvent);
  const failedPort = Number(serviceEvent.message.match(/:(\d+)$/)[1]);
  await assert.rejects(fetch(`http://127.0.0.1:${failedPort}`));
  console.log(
    "PASS dependency cycle rejection and reverse cleanup after dependent failure",
  );
  const liveContainer = restarted.services[0].container_id;
  h.process.kill("SIGKILL");
  await h.exited();
  assert.equal(inspect("container", liveContainer).State.Running, true);
  await h.launch();
  assert.ok((await h.call("list")).every((p) => p.state === "STOPPED"));
  assert.equal(
    docker(
      "container",
      "ls",
      "--all",
      "--quiet",
      "--filter",
      `label=dev.dockon.installation=${installation}`,
    ),
    "",
  );
  assert.equal(
    inspect("volume", va).Labels["dev.dockon.installation"],
    installation,
  );
  console.log(
    "PASS daemon crash reconciliation removes owned containers and preserves all persistent volumes",
  );
  await h.shutdown();
} finally {
  try {
    await h.shutdown();
  } catch {
    if (h.process?.exitCode === null) h.process.kill();
  }
  // These are exclusively generated test resources. Verify labels before each
  // removal; never clean arbitrary Docker projects, images or host volumes.
  if (installation) {
    const filter = `label=dev.dockon.installation=${installation}`;
    for (const id of docker(
      "container",
      "ls",
      "--all",
      "--quiet",
      "--filter",
      filter,
    )
      .split("\n")
      .filter(Boolean)) {
      const v = inspect("container", id);
      assert.equal(v.Config.Labels["dev.dockon.installation"], installation);
      docker("container", "rm", "--force", id);
    }
    for (const kind of ["volume", "network"])
      for (const id of docker(kind, "ls", "--quiet", "--filter", filter)
        .split("\n")
        .filter(Boolean)) {
        assert.equal(
          inspect(kind, id).Labels["dev.dockon.installation"],
          installation,
        );
        docker(kind, "rm", id);
      }
  }
}
