import { spawn } from "node:child_process";
import { connect } from "node:net";
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { resolve, join } from "node:path";
import assert from "node:assert/strict";
export const pause = (ms) => new Promise((r) => setTimeout(r, ms));
export class DaemonHarness {
  root = resolve(".");
  data;
  process;
  connection;
  binary(name) {
    return join(
      this.root,
      "target",
      "debug",
      name + (process.platform === "win32" ? ".exe" : ""),
    );
  }
  async initialize(prefix = "integration-") {
    await mkdir(".local", { recursive: true });
    this.data = await mkdtemp(join(this.root, ".local", prefix));
    return this;
  }
  async launch() {
    this.process = spawn(this.binary("dockon-daemon"), [], {
      env: { ...process.env, DOCKON_DATA_DIR: this.data },
      windowsHide: true,
      stdio: ["ignore", "pipe", "pipe"],
    });
    this.process.stdout.on("data", () => {});
    this.process.stderr.on("data", (b) => process.stderr.write(b));
    for (let i = 0; i < 300; i++) {
      try {
        this.connection = JSON.parse(
          await readFile(join(this.data, "connection.json"), "utf8"),
        );
        if ((await this.rpc("health")).ok) return;
      } catch {}
      if (this.process.exitCode !== null)
        throw new Error("Daemon exited during startup");
      await pause(100);
    }
    throw new Error("Daemon startup timed out");
  }
  async rpc(method, params) {
    return new Promise((resolve, reject) => {
      const socket = connect(this.connection.endpoint);
      let buffer = "";
      const timeout = setTimeout(() => {
        socket.destroy();
        reject(new Error("RPC timed out"));
      }, 35000);
      socket.on("error", (e) => {
        clearTimeout(timeout);
        reject(e);
      });
      socket.on("connect", () =>
        socket.write(
          JSON.stringify({
            version: 1,
            token: this.connection.token,
            command: { method, ...(params ? { params } : {}) },
          }) + "\n",
        ),
      );
      socket.on("data", (bytes) => {
        buffer += bytes;
        if (buffer.includes("\n")) {
          clearTimeout(timeout);
          socket.end();
          resolve(JSON.parse(buffer.split("\n")[0]));
        }
      });
    });
  }
  async call(method, params) {
    const response = await this.rpc(method, params);
    assert.equal(response.ok, true, response.error);
    return response.data;
  }
  async wait(id) {
    for (let i = 0; i < 2400; i++) {
      const operation = await this.call("operation", { id });
      if (operation.status !== "running") return operation;
      await pause(100);
    }
    throw new Error("Operation timed out");
  }
  async action(method, id) {
    const op = await this.wait((await this.call(method, { id })).id);
    if (op.status !== "completed") {
      const events = (await this.call("events", { after: 0 })).events
        .filter((event) => event.project_id === id)
        .slice(-12);
      op.error +=
        "\nRecent project output: " +
        events.map((event) => event.message).join("\n");
    }
    assert.equal(op.status, "completed", op.error);
    return (await this.call("list")).find((p) => p.manifest.id === id);
  }
  manifest(id, overrides = {}) {
    return {
      schema_version: 1,
      id,
      name: id,
      root: join(this.root, "examples/hello-http"),
      executable: this.binary("dockon-hello-http"),
      args: [],
      port: 0,
      ...overrides,
    };
  }
  async register(manifest) {
    const path = join(this.data, manifest.id + ".project.json");
    await writeFile(path, JSON.stringify(manifest));
    return this.rpc("register", { manifest_path: path });
  }
  async shutdown() {
    if (!this.process || this.process.exitCode !== null) return;
    await this.call("shutdown");
    await this.exited();
  }
  async exited() {
    if (this.process.exitCode !== null || this.process.signalCode !== null)
      return;
    await new Promise((resolve, reject) => {
      const timer = setTimeout(
        () => reject(new Error("Daemon exit timed out")),
        12000,
      );
      this.process.once("exit", () => {
        clearTimeout(timer);
        resolve();
      });
    });
  }
}
