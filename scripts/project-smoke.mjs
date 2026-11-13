import { DaemonHarness } from "./lib/daemon-harness.mjs";
import { verifyProfile } from "./lib/project-profiles.mjs";
import { execFileSync } from "node:child_process";
const h = await new DaemonHarness().initialize("project-profiles-");
try {
  await h.launch();
  const selected = await h.call("runtime_use_system", {
    runtime: "node",
    executable: process.execPath,
  });
  await verifyProfile(h, selected);
  if (process.env.DOCKON_TEST_PYTHON_COMMAND) {
    const executable = execFileSync(
      process.env.DOCKON_TEST_PYTHON_COMMAND,
      ["-c", "import sys; print(sys.executable)"],
      { encoding: "utf8", windowsHide: true },
    ).trim();
    await verifyProfile(
      h,
      await h.call("runtime_use_system", { runtime: "python", executable }),
    );
  }
  if (process.env.DOCKON_TEST_RUST === "1") {
    const executable = execFileSync("rustup", ["which", "cargo"], {
      encoding: "utf8",
      windowsHide: true,
    }).trim();
    await verifyProfile(
      h,
      await h.call("runtime_use_system", { runtime: "rust", executable }),
    );
  }
} finally {
  await h.shutdown();
}
