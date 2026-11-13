import assert from "node:assert/strict";
import { join } from "node:path";
import { readFile, writeFile } from "node:fs/promises";
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
  assert.equal((await fetch(`http://127.0.0.1:${running.port}/`)).status, 200);
  await h.action("stop", id);
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
