import { DaemonHarness } from "./lib/daemon-harness.mjs";
import { verifyProfile } from "./lib/project-profiles.mjs";
const h = await new DaemonHarness().initialize("project-profiles-");
try {
  await h.launch();
  const selected = await h.call("runtime_use_system", {
    runtime: "node",
    executable: process.execPath,
  });
  await verifyProfile(h, selected);
} finally {
  await h.shutdown();
}
