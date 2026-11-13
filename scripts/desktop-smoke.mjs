import { chromium } from "@playwright/test";
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { resolve, join } from "node:path";
import assert from "node:assert/strict";

if (process.platform !== "win32")
  throw new Error("This local native WebView2 check currently targets Windows");
const root = resolve(".");
await mkdir(".local", { recursive: true });
const data = await mkdtemp(join(root, ".local", "desktop-"));
const listener = createServer();
await new Promise((r) => listener.listen(0, "127.0.0.1", r));
const port = listener.address().port;
await new Promise((r) => listener.close(r));
const env = {
  ...process.env,
  DOCKON_DATA_DIR: data,
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1`,
};
const app = spawn(join(root, "target/debug/dockon-desktop.exe"), [], {
  env,
  windowsHide: true,
  stdio: "ignore",
});
let browser;
try {
  for (let i = 0; i < 100; i++) {
    try {
      browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
      break;
    } catch {
      await new Promise((r) => setTimeout(r, 100));
    }
  }
  assert.ok(browser, "WebView2 debugging endpoint did not become available");
  let page;
  for (let i = 0; i < 100; i++) {
    page = browser
      .contexts()
      .flatMap((c) => c.pages())
      .find((p) => p.url().startsWith("http://localhost:1420"));
    if (page) break;
    await new Promise((r) => setTimeout(r, 100));
  }
  assert.ok(page, "Dockon app page not found");
  await page
    .getByText("Daemon bağlı", { exact: true })
    .waitFor({ timeout: 15000 });
  const manifest = join(data, "ui.project.json");
  await writeFile(
    manifest,
    JSON.stringify({
      schema_version: 1,
      id: "ui-example",
      name: "UI example",
      root: join(root, "examples/hello-http"),
      executable: join(root, "target/debug/dockon-hello-http.exe"),
      args: ["--startup-delay-ms", "1500"],
      port: 0,
    }),
  );
  await page
    .getByRole("button", { name: "Proje içe aktar", exact: true })
    .click();
  await page
    .getByLabel("Manifest dosyasının tam yolu", { exact: true })
    .fill(manifest);
  await page
    .getByRole("button", { name: "Proje içe aktar", exact: true })
    .last()
    .click();
  await page
    .getByRole("heading", { name: "UI example", exact: true })
    .waitFor();
  await page.getByRole("button", { name: "Başlat", exact: true }).click();
  await page.getByText("STARTING", { exact: true }).first().waitFor();
  await page.getByRole("button", { name: "Durdur", exact: true }).waitFor();
  await page.getByText("RUNNING", { exact: true }).first().waitFor();
  const link = await page
    .getByRole("link", { name: "Tarayıcıda aç", exact: true })
    .getAttribute("href");
  assert.equal((await fetch(link)).status, 200);
  await new Promise((resolve) => setTimeout(resolve, 1800));
  assert.equal(
    await page.getByText("Daemon bağlantısı yok", { exact: true }).count(),
    0,
  );
  await page.getByText("Daemon bağlı", { exact: true }).waitFor();
  await mkdir(".local/screenshots", { recursive: true });
  await page.screenshot({
    path: ".local/screenshots/desktop-running.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: "Durdur", exact: true }).click();
  await page.getByRole("button", { name: "Başlat", exact: true }).waitFor();
  await assert.rejects(fetch(link));
  console.log(
    "PASS native Tauri/WebView2 import, real STARTING/RUNNING, HTTP readiness, streaming logs and stop",
  );
} finally {
  if (browser) await browser.close();
  app.kill();
  await new Promise((r) => setTimeout(r, 300));
  const ctl = spawn(join(root, "target/debug/dockonctl.exe"), ["shutdown"], {
    env,
    windowsHide: true,
    stdio: "ignore",
  });
  await new Promise((r) => ctl.once("exit", r));
}
