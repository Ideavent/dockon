import { chromium, expect } from "@playwright/test";
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { resolve, join, dirname } from "node:path";
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
  DOCKON_WWW_DIR: process.env.DOCKON_WWW_DIR || join(data, "www"),
  WEBVIEW2_USER_DATA_FOLDER: join(data, "webview"),
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1`,
};
if (process.env.DOCKON_CADDY_EXECUTABLE)
  env.PATH =
    dirname(process.env.DOCKON_CADDY_EXECUTABLE) +
    ";" +
    (process.env.PATH || "");
const appPath = process.env.DOCKON_DESKTOP_EXECUTABLE
  ? resolve(process.env.DOCKON_DESKTOP_EXECUTABLE)
  : join(root, "target/debug/dockon-desktop.exe");
const appOrigin = process.env.DOCKON_DESKTOP_EXECUTABLE
  ? "http://tauri.localhost"
  : "http://localhost:1420";
const app = spawn(appPath, [], {
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
      .find((p) => URL.canParse(p.url()) && new URL(p.url()).origin === appOrigin);
    if (page) break;
    await new Promise((r) => setTimeout(r, 100));
  }
  assert.ok(page, "Dockon app page not found");
  await page
    .getByText("Daemon bağlı", { exact: true })
    .waitFor({ timeout: 15000 });
  if (process.env.DOCKON_WORKSPACE_SMOKE === "1") {
    await expect(page.getByText("fixture-project", { exact: true })).toBeVisible();
    await expect(page.getByText("outside-project", { exact: true })).toHaveCount(0);
    await expect(page.getByText("nested", { exact: true })).toHaveCount(0);
    await page.getByRole("button", { name: "Projeyi ayarla", exact: true }).click();
    await expect(page.getByLabel("Proje adı", { exact: true })).toHaveValue("fixture-project");
    await page.getByRole("button", { name: "Vazgeç", exact: true }).click();
  }
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
    .getByRole("button", { name: "Manifest içe aktar", exact: true })
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
  await page
    .getByRole("button", { name: "Çalışma ortamları", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Sistem araçlarını bul", exact: true })
    .click();
  const nodeTool = page
    .locator(".system-tool")
    .filter({ has: page.getByText("Node.js", { exact: true }) })
    .first();
  await nodeTool
    .getByRole("button", { name: "Bu aracı kullan", exact: true })
    .click();
  await page.locator(".runtime-item").filter({ hasText: "Node.js" }).waitFor();
  const manualPath = page.getByLabel("Çalıştırılabilir dosyanın tam yolu", {
    exact: true,
  });
  const manualUse = page
    .locator("form")
    .filter({ has: manualPath })
    .getByRole("button", { name: "Bu aracı kullan", exact: true });
  await manualPath.fill(join(data, "missing-runtime.exe"));
  await manualUse.click();
  await page.getByRole("alert").waitFor();
  await manualPath.fill(process.execPath);
  await manualUse.click();
  await expect(manualUse).toBeEnabled();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.getByRole("button", { name: /^Projeler/ }).click();
  await page
    .getByRole("button", { name: "Yeni proje", exact: true })
    .click();
  const wizard = page.getByRole("dialog", {
    name: "Proje hazırla",
    exact: true,
  });
  await wizard.getByLabel("Proje adı", { exact: true }).fill("Wizard Node");
  await expect(wizard.getByText("wizard-node.test", { exact: true })).toBeVisible();
  await wizard
    .getByLabel("Çalışma zamanı ve sürüm", { exact: true })
    .selectOption({
      label: (
        await wizard.locator("#project-runtime option").allTextContents()
      ).find((t) => t.startsWith("node ")),
    });
  await wizard
    .getByRole("button", { name: "Projeyi oluştur", exact: true })
    .click();
  const card = page.locator(".project-card").filter({
    has: page.getByRole("heading", { name: "Wizard Node", exact: true }),
  });
  await card.getByRole("button", { name: "Başlat", exact: true }).click();
  await card
    .getByRole("button", { name: "Durdur", exact: true })
    .waitFor({ timeout: 20000 });
  const wizardLink = await card
    .getByRole("link", { name: "Tarayıcıda aç", exact: true })
    .getAttribute("href");
  assert.match(await (await fetch(wizardLink)).text(), /dockon · Node.js/);
  await card.getByRole("button", { name: "Durdur", exact: true }).click();
  await card.getByRole("button", { name: "Başlat", exact: true }).waitFor();
  await card.click({ button: "right" });
  await page.getByRole("group", { name: "Proje menüsü" }).getByRole("button", { name: "Proje ayarları" }).click();
  const projectSettings = page.getByRole("dialog", { name: "Proje ayarları" });
  await expect(projectSettings.getByRole("checkbox", { name: "SSL kullan (HTTPS)" })).toBeVisible();
  await projectSettings.getByRole("button", { name: "Kapat", exact: true }).click();
  await page.getByRole("button", { name: "Tanılama", exact: true }).click();
  const report = page.getByRole("region", { name: "Durum raporu" }).locator("pre");
  await report.waitFor();
  const reportText = await report.textContent();
  assert.equal(JSON.parse(reportText).project_counts.STOPPED, 2);
  assert.ok(JSON.parse(reportText).runtimes.installed >= 1);
  assert.doesNotMatch(reportText, /ui-example|wizard-node|executable|password|port|token/i);
  await expect(page.getByRole("button", { name: "JSON indir" })).toBeEnabled();
  if (process.env.DOCKON_GUARDIAN_PREVIEW === "1") {
    await page.getByRole("button", { name: "Veriyi önizle", exact: true }).click();
    await expect(page.getByText("https://ai.ideavent.tr/v1", { exact: true })).toBeVisible();
    await expect(page.getByLabel("Sağlayıcı API anahtarı")).toHaveAttribute("type", "password");
  }
  await page.getByRole("button", { name: /^Projeler/ }).click();
  console.log(
    "PASS native runtime opt-in and new project wizard with real selected-runtime HTTP server",
  );
  if (process.env.DOCKON_CADDY_EXECUTABLE) {
    await page
      .getByRole("button", { name: "Çalışma ortamları", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Sistem araçlarını bul", exact: true })
      .click();
    await page
      .locator(".system-tool")
      .filter({ has: page.getByText("Caddy", { exact: true }) })
      .first()
      .getByRole("button", { name: "Bu aracı kullan", exact: true })
      .click();
    await page.locator(".runtime-item").filter({ hasText: "Caddy" }).waitFor();
    await page
      .getByRole("button", { name: "Seçenekler", exact: true })
      .click();
    await page.getByRole("button", { name: "Servisler ve Portlar", exact: true }).click();
    const caddySelect = page.getByRole("combobox", { name: /Caddy sürümü/ });
    await caddySelect.selectOption({
      label: (await caddySelect.locator("option").allTextContents()).find((t) =>
        t.startsWith("Caddy "),
      ),
    });
    const route = page.locator(".route-row").filter({ hasText: "Wizard Node" });
    await route.locator("input[type=checkbox]").first().check();
    assert.equal(
      await page
        .getByLabel("Alan adı: Wizard Node", { exact: true })
        .inputValue(),
      "wizard-node.test",
    );
    await page
      .getByLabel("Alan adı: Wizard Node", { exact: true })
      .fill("custom.ui.test");
    for (const label of ["HTTP portu", "HTTPS portu"]) {
      const temporary = createServer();
      await new Promise((r) => temporary.listen(0, "127.0.0.1", r));
      const free = temporary.address().port;
      await new Promise((r) => temporary.close(r));
      await page
        .getByRole("spinbutton", { name: label, exact: true })
        .fill(String(free));
    }
    await page
      .getByRole("button", { name: "Değişikliği önizle", exact: true })
      .click();
    await page
      .getByRole("button", {
        name: "Yönlendirici değişikliğini uygula",
        exact: true,
      })
      .click();
    await page
      .getByText("Yönlendirici çalışıyor", { exact: true })
      .waitFor({ timeout: 30000 });
    await page
      .getByRole("button", { name: "Hosts kaydını önizle", exact: true })
      .click();
    await page
      .getByRole("button", {
        name: "Bu sistem değişikliğine onay ver",
        exact: true,
      })
      .waitFor();
    await page.screenshot({
      path: ".local/screenshots/router-permissions.png",
      fullPage: true,
    });
    await page.getByRole("button", { name: "Vazgeç", exact: true }).click();
    await page
      .getByRole("button", { name: "Yönlendiriciyi durdur", exact: true })
      .click();
    await page.getByText("Yönlendirici durduruldu", { exact: true }).waitFor();
    console.log(
      "PASS native editable .test domain, real router plan/apply and separate hosts permission preview without system changes",
    );
  }
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
