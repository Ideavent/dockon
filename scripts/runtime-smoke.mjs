import { DaemonHarness, pause } from "./lib/daemon-harness.mjs";
import { writeFile, readFile, mkdir } from "node:fs/promises";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import assert from "node:assert/strict";
import { verifyProfile } from "./lib/project-profiles.mjs";

const h = await new DaemonHarness().initialize("runtimes-");
try {
  await h.launch();
  const catalog = await h.call("runtime_catalog", { runtime: "node" });
  assert.ok(catalog.versions.length > 0);
  assert.equal(catalog.cached, false);
  const version =
    catalog.versions.find((v) => v.support === "lts")?.version ||
    catalog.versions[0].version;
  console.log(
    `Checking official Node.js ${version} installation on ${catalog.platform}`,
  );
  const operation = await h.wait(
    (await h.call("runtime_install", { runtime: "node", version })).id,
  );
  assert.equal(operation.status, "completed", operation.error);
  const installed = (await h.call("runtime_installed")).find(
    (v) => v.runtime === "node" && v.version === version,
  );
  assert.ok(installed);
  assert.equal(installed.signature_verified, false);
  assert.equal(installed.archive_hash.length, 64);
  assert.equal(
    createHash("sha256")
      .update(await readFile(installed.executable))
      .digest("hex"),
    installed.executable_hash,
  );
  assert.equal(
    execFileSync(installed.executable, ["--version"], {
      encoding: "utf8",
      windowsHide: true,
    }).trim(),
    `v${version}`,
  );
  const projectRoot = join(h.data, "node-project");
  await mkdir(projectRoot);
  await writeFile(
    join(projectRoot, "server.mjs"),
    `import http from 'node:http';http.createServer((req,res)=>{res.writeHead(200,{'Content-Type':'application/json'});res.end(JSON.stringify({runtime:'node',version:process.version}));}).listen(Number(process.env.PORT),'127.0.0.1');`,
  );
  const registered = await h.register(
    h.manifest("managed-node", {
      root: projectRoot,
      executable: installed.executable,
      args: [join(projectRoot, "server.mjs")],
    }),
  );
  assert.equal(registered.ok, true, registered.error);
  const running = await h.action("start", "managed-node");
  const data = await (
    await fetch(`http://127.0.0.1:${running.port}/health`)
  ).json();
  assert.equal(data.version, `v${version}`);
  const repeated = await h.wait(
    (await h.call("runtime_install", { runtime: "node", version })).id,
  );
  assert.equal(repeated.status, "completed", repeated.error);
  assert.equal(
    (await h.call("list")).find((v) => v.manifest.id === "managed-node").pid,
    running.pid,
  );
  await h.action("stop", "managed-node");
  await verifyProfile(h, installed);
  console.log(
    "PASS publisher digest, isolated staged installation, observed runtime version and managed project lifecycle",
  );
  const extended = process.env.DOCKON_EXTENDED_RUNTIMES === "1";
  const web = process.env.DOCKON_WEB_RUNTIMES === "1";
  if (extended || web) {
    const runtimes = extended ? ["php", "dotnet", "go", "caddy"] : [];
    runtimes.push("apache", "nginx");
    for (const runtime of runtimes) {
      const catalog = await h.call("runtime_catalog", { runtime });
      if (!catalog.versions.length) {
        console.log(`SKIP ${runtime}: ${catalog.note}`);
        continue;
      }
      const selected = catalog.versions[0].version;
      console.log(`Checking ${runtime} ${selected}`);
      const result = await h.wait(
        (await h.call("runtime_install", { runtime, version: selected })).id,
      );
      assert.equal(result.status, "completed", result.error);
      assert.ok(
        (await h.call("runtime_installed")).some(
          (v) => v.runtime === runtime && v.version === selected,
        ),
      );
      await verifyProfile(
        h,
        (await h.call("runtime_installed")).find(
          (v) => v.runtime === runtime && v.version === selected,
        ),
      );
    }
    console.log(
      "PASS extended runtime publisher downloads and executable version checks",
    );
    if (process.platform === "win32" && process.arch === "x64") {
      const installed = await h.call("runtime_installed");
      const apache = installed.find((record) => record.runtime === "apache");
      const nginx = installed.find((record) => record.runtime === "nginx");
      const id = "switch-static-site";
      await h.call("project_create", {
        id,
        name: "Switch static site",
        root: join(h.data, id),
        runtime_installation: apache.id,
      });
      const before = await h.action("start", id);
      const rejected = await h.wait(
        (
          await h.call("project_switch_static_server", {
            id,
            runtime_installation: "unknown",
          })
        ).id,
      );
      assert.equal(rejected.status, "failed");
      assert.equal(
        (await fetch(`http://127.0.0.1:${before.port}/`)).status,
        200,
      );
      for (const server of [nginx, apache]) {
        const switched = await h.wait(
          (
            await h.call("project_switch_static_server", {
              id,
              runtime_installation: server.id,
            })
          ).id,
        );
        assert.equal(switched.status, "completed", switched.error);
        const active = (await h.call("list")).find(
          (project) => project.manifest.id === id,
        );
        assert.equal(active.manifest.runtime.installation_id, server.id);
        assert.match(
          await (await fetch(`http://127.0.0.1:${active.port}/`)).text(),
          /Dockon · Apache/,
        );
      }
      await h.action("stop", id);
      console.log(
        "PASS staged Apache/Nginx switching and rejected selection preservation",
      );
    }
  }
  await h.shutdown();
  const originals = Object.fromEntries(
    ["HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY", "NO_PROXY"].map((key) => [
      key,
      process.env[key],
    ]),
  );
  try {
    process.env.HTTPS_PROXY = "http://127.0.0.1:9";
    process.env.HTTP_PROXY = "http://127.0.0.1:9";
    process.env.ALL_PROXY = "http://127.0.0.1:9";
    process.env.NO_PROXY = "";
    await h.launch();
    const cached = await h.call("runtime_catalog", { runtime: "node" });
    assert.equal(cached.cached, true);
    assert.equal(cached.versions[0].version, catalog.versions[0].version);
    const upgrades = await h.call("runtime_updates", { runtime: "node" });
    assert.equal(upgrades.cached, true);
    for (const upgrade of upgrades.updates) {
      assert.equal(upgrade.installation_id, installed.id);
      assert.equal(upgrade.current_version, installed.version);
      assert.ok(cached.versions.some((candidate) => candidate.version === upgrade.version));
      assert.notEqual(upgrade.version, installed.version);
    }
    assert.ok(
      (await h.call("runtime_installed")).some((v) => v.id === installed.id),
    );
    assert.ok((await h.call("list")).every((p) => p.state === "STOPPED"));
    await h.shutdown();
  } finally {
    for (const [key, value] of Object.entries(originals)) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
  console.log(
    "PASS offline cached catalog, installation persistence and non-autostart recovery",
  );
  console.log(`Runtime test data: ${h.data}`);
} finally {
  try {
    await h.shutdown();
  } catch {
    if (h.process?.exitCode === null) h.process.kill();
  }
}
