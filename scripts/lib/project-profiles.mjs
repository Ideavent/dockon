import assert from "node:assert/strict";
import { join } from "node:path";
import { readFile, writeFile, access } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";

export async function verifyProfile(h, installation) {
  if (installation.runtime === "caddy") return;
  const id = `profile-${installation.runtime}-${randomUUID().slice(0, 8)}`;
  const root = join(h.data, id);
  const created = await h.call("project_create", {
    id,
    name: id,
    root,
    runtime_installation: installation.id,
  });
  assert.equal(created.manifest.runtime.installation_id, installation.id);
  assert.equal(created.state, "STOPPED");
  const manifestBefore = await readFile(join(root, "dockon.json"), "utf8");
  assert.equal(
    (
      await h.rpc("project_create", {
        id: id + "-existing",
        name: id,
        root,
        runtime_installation: installation.id,
      })
    ).ok,
    false,
  );
  assert.equal(
    await readFile(join(root, "dockon.json"), "utf8"),
    manifestBefore,
  );
  const detected = await h.call("project_detect", { root });
  const profile = detected.profiles.find(
    (p) => p.runtime === installation.runtime,
  );
  assert.ok(profile);
  const running = await h.action("start", id);
  const url = `http://127.0.0.1:${running.port}`;
  const response = await fetch(url);
  assert.equal(response.status, 200);
  if (installation.runtime === "apache") {
    assert.match(await response.text(), /Dockon · Apache/);
    await writeFile(join(root, "public", "private.php"), "private fixture");
    assert.equal((await fetch(url + "/private.php")).status, 403);
  }
  await h.action("stop", id);
  const privateData = join(
    h.data,
    "instances",
    Buffer.from(id).toString("hex"),
    "app",
  );
  if (installation.runtime === "python") {
    const venv = join(privateData, "venv");
    const executable = join(
      venv,
      process.platform === "win32" ? "Scripts/python.exe" : "bin/python",
    );
    assert.equal(
      await readFile(join(venv, "dockon-base.sha256"), "utf8"),
      installation.executable_hash,
    );
    const prefixes = JSON.parse(
      execFileSync(
        executable,
        [
          "-c",
          "import sys,json; print(json.dumps([sys.prefix,sys.base_prefix]))",
        ],
        { encoding: "utf8", windowsHide: true },
      ),
    );
    assert.notEqual(prefixes[0], prefixes[1]);
    await h.action("start", id);
    await h.action("stop", id);
    await writeFile(
      join(venv, "dockon-base.sha256"),
      "a different interpreter",
    );
    const refused = await h.wait((await h.call("start", { id })).id);
    assert.equal(refused.status, "failed");
    assert.match(refused.error, /different base interpreter/);
    console.log(
      "PASS private Python venv, environment reuse and rejection of a changed interpreter marker",
    );
  }
  if (installation.runtime === "rust") {
    await access(join(privateData, "cargo-target"));
    await assert.rejects(access(join(root, "target")));
    console.log("PASS concrete Rust toolchain and private build output");
  }
  const imported = await h.call("project_import", {
    id: id + "-import",
    name: id + " imported",
    root,
    runtime_installation: installation.id,
    profile: profile.id,
  });
  assert.equal(imported.state, "STOPPED");
  assert.equal(
    await readFile(join(root, "dockon.json"), "utf8"),
    manifestBefore,
  );
  const bad = JSON.parse(manifestBefore);
  bad.id = id + "-mismatch";
  bad.executable = h.binary("dockon-hello-http");
  await h.register(bad);
  const failed = await h.wait((await h.call("start", { id: bad.id })).id);
  assert.equal(failed.status, "failed");
  assert.match(failed.error, /differs from the selected runtime/);
  console.log(
    `PASS ${installation.runtime} scaffold, detection, selected runtime, HTTP readiness, import preservation and binding mismatch rejection`,
  );
}
