import { spawn, execFileSync } from "node:child_process";
import { mkdir, mkdtemp, writeFile, readFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { connect } from "node:net";
import assert from "node:assert/strict";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const suffix = process.platform === "win32" ? ".exe" : "";
const binary = (name) => join(root, "target", "debug", name + suffix);
await mkdir(join(root, ".local"), { recursive: true });
const data = await mkdtemp(join(root, ".local", "smoke-"));
const env = { ...process.env, DOCKON_DATA_DIR: data };
let daemon;
let connection;
const pause = (ms) => new Promise((r) => setTimeout(r, ms));
function launch() {
  daemon = spawn(binary("dockon-daemon"), [], {
    env,
    windowsHide: true,
    stdio: ["ignore", "pipe", "pipe"],
  });
  daemon.stdout.on("data", () => {});
  daemon.stderr.on("data", (bytes) => process.stderr.write(bytes));
  return daemon;
}
async function ready() {
  for (let i = 0; i < 150; i++) {
    try {
      connection = JSON.parse(
        await readFile(join(data, "connection.json"), "utf8"),
      );
      if ((await rpc("health")).ok) return;
    } catch {}
    if (daemon.exitCode !== null)
      throw new Error(`Daemon exited: ${daemon.exitCode}`);
    await pause(100);
  }
  throw new Error("Daemon readiness timed out");
}
function rpc(method, params, token = connection.token) {
  return new Promise((resolve, reject) => {
    const socket = connect(connection.endpoint);
    let buffer = "";
    const timer = setTimeout(() => {
      socket.destroy();
      reject(new Error("RPC timeout"));
    }, 35000);
    socket.on("connect", () =>
      socket.write(
        JSON.stringify({
          version: 1,
          token,
          command: { method, ...(params ? { params } : {}) },
        }) + "\n",
      ),
    );
    socket.on("error", (err) => {
      clearTimeout(timer);
      reject(err);
    });
    socket.on("data", (bytes) => {
      buffer += bytes;
      if (buffer.includes("\n")) {
        clearTimeout(timer);
        socket.end();
        resolve(JSON.parse(buffer.split("\n")[0]));
      }
    });
  });
}
async function successful(method, params) {
  const r = await rpc(method, params);
  assert.equal(r.ok, true, r.error);
  if (method === "start" || method === "stop") {
    const operation = await awaitOperation(r.data.id);
    assert.equal(operation.status, "completed", operation.error);
    return (await successful("list")).find((p) => p.manifest.id === params.id);
  }
  return r.data;
}
async function awaitOperation(id) {
  for (let i = 0; i < 300; i++) {
    const op = (await rpc("operation", { id })).data;
    if (op.status !== "running") return op;
    await pause(50);
  }
  throw new Error("Operation timeout");
}
async function register(id, overrides = {}) {
  const path = join(data, id + ".project.json");
  await writeFile(
    path,
    JSON.stringify({
      schema_version: 1,
      id,
      name: id,
      root: join(root, "examples", "hello-http"),
      executable: binary("dockon-hello-http"),
      args: [],
      port: 0,
      ...overrides,
    }),
  );
  return successful("register", { manifest_path: path });
}
async function stopped(child) {
  if (child.exitCode !== null) return;
  await Promise.race([
    new Promise((r) => child.once("exit", r)),
    pause(10000).then(() => {
      throw new Error("Shutdown timeout");
    }),
  ]);
}
try {
  launch();
  await ready();
  assert.equal((await rpc("health", undefined, "invalid-token")).ok, false);
  const duplicate = spawn(binary("dockon-daemon"), [], {
    env,
    windowsHide: true,
    stdio: "ignore",
  });
  await stopped(duplicate);
  assert.notEqual(duplicate.exitCode, 0);
  console.log("PASS authenticated IPC and single-instance lock");
  await register("alpha", { args: ["--with-worker"] });
  await register("beta");
  const alpha = await successful("start", { id: "alpha" });
  assert.equal(alpha.state, "RUNNING");
  assert.ok(alpha.pid);
  assert.ok(alpha.port);
  const body = await (
    await fetch(`http://127.0.0.1:${alpha.port}/health`)
  ).json();
  assert.equal(body.name, "dockon-hello-http");
  assert.equal((await successful("start", { id: "alpha" })).pid, alpha.pid);
  const beta = await successful("start", { id: "beta" });
  assert.notEqual(alpha.pid, beta.pid);
  assert.notEqual(alpha.port, beta.port);
  await successful("stop", { id: "alpha" });
  await successful("stop", { id: "alpha" });
  const workerEvent = (await successful("events", { after: 0 })).events.find(
    (e) => e.message.startsWith("worker_pid="),
  );
  assert.ok(workerEvent, "Worker PID was logged");
  const workerPid = Number(workerEvent.message.split("=")[1]);
  await pause(300);
  assert.throws(
    () => process.kill(workerPid, 0),
    "Descendant worker must be terminated",
  );
  assert.equal(
    (await fetch(`http://127.0.0.1:${beta.port}/health`)).status,
    200,
  );
  await assert.rejects(fetch(`http://127.0.0.1:${alpha.port}/health`));
  console.log("PASS readiness, idempotence and independent project lifecycle");
  await register("conflict", { port: beta.port });
  assert.equal(
    (await awaitOperation((await rpc("start", { id: "conflict" })).data.id))
      .status,
    "failed",
  );
  await register("bad-command", { executable: binary("dockonctl") });
  assert.equal(
    (await awaitOperation((await rpc("start", { id: "bad-command" })).data.id))
      .status,
    "failed",
  );
  assert.equal(
    (await successful("list")).find((p) => p.manifest.id === "bad-command")
      .state,
    "FAILED",
  );
  assert.equal(
    (await fetch(`http://127.0.0.1:${beta.port}/health`)).status,
    200,
  );
  const { events } = await successful("events", { after: 0 });
  assert.ok(events.some((e) => e.kind === "stdout"));
  assert.ok(events.some((e) => e.message === "STARTING"));
  assert.equal(
    (await successful("events", { after: events.at(-1).seq })).events.length,
    0,
  );
  console.log(
    "PASS port conflict, failed launch cleanup and real log/event feed",
  );
  await register("slow", { args: ["--startup-delay-ms", "2000"] });
  const slowOp = await successful("health").then(() =>
    rpc("start", { id: "slow" }),
  );
  await pause(100);
  const duringStart = await successful("list");
  assert.equal(
    duringStart.find((p) => p.manifest.id === "slow").state,
    "STARTING",
  );
  assert.equal(
    duringStart.find((p) => p.manifest.id === "beta").state,
    "RUNNING",
  );
  await successful("stop", { id: "beta" });
  assert.equal((await awaitOperation(slowOp.data.id)).status, "completed");
  await successful("stop", { id: "slow" });
  const simultaneous = await Promise.all([
    rpc("start", { id: "alpha" }),
    rpc("start", { id: "alpha" }),
  ]);
  for (const reply of simultaneous)
    assert.equal((await awaitOperation(reply.data.id)).status, "completed");
  await successful("stop", { id: "alpha" });
  console.log(
    "PASS live STARTING state, asynchronous operation journal and per-project concurrency",
  );
  const oldConnection = connection;
  await successful("shutdown");
  await stopped(daemon);
  await assert.rejects(fetch(`http://127.0.0.1:${beta.port}/health`));
  launch();
  await ready();
  assert.notEqual(connection.token, oldConnection.token);
  const persisted = await successful("list");
  assert.equal(persisted.length, 5);
  assert.ok(persisted.every((p) => p.state === "STOPPED"));
  await successful("shutdown");
  await stopped(daemon);
  console.log(
    "PASS shutdown cleanup, persistence and no automatic project start",
  );
  console.log(`All foundation smoke checks passed. Artifacts: ${data}`);
} finally {
  if (daemon && daemon.exitCode === null) {
    try {
      await rpc("shutdown");
      await stopped(daemon);
    } catch {
      daemon.kill();
    }
  }
}
