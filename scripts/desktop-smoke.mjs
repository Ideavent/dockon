import { chromium, expect } from "@playwright/test";
import { execFile, spawn } from "node:child_process";
import { request as httpRequest } from "node:http";
import { request as httpsRequest } from "node:https";
import { connect, createServer } from "node:net";
import { createInterface } from "node:readline";
import { existsSync } from "node:fs";
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  writeFile,
} from "node:fs/promises";
import { resolve, join, dirname } from "node:path";
import assert from "node:assert/strict";
import { promisify } from "node:util";
import { DaemonHarness } from "./lib/daemon-harness.mjs";

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
const appOrigin =
  process.env.DOCKON_DESKTOP_ORIGIN ??
  (process.env.DOCKON_DESKTOP_EXECUTABLE
    ? "http://tauri.localhost"
    : "http://localhost:1420");
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
      .find(
        (p) => URL.canParse(p.url()) && new URL(p.url()).origin === appOrigin,
      );
    if (page) break;
    await new Promise((r) => setTimeout(r, 100));
  }
  assert.ok(page, "Dockon app page not found");
  await expect(page.getByRole("button", { name: "Yeni proje", exact: true }))
    .toBeEnabled({ timeout: 30000 })
    .catch(async (error) => {
      throw new Error(await page.locator("body").innerText(), { cause: error });
    });
  const harness = new DaemonHarness();
  harness.connection = JSON.parse(
    await readFile(join(data, "connection.json"), "utf8"),
  );
  // Start fixtures via IPC so this test leaves the machine's hosts file untouched.
  const navigate = (label) =>
    page.getByRole("navigation", { name: "Ana gezinme" })
      .getByRole("button", { name: label, exact: true })
      .click();
  const back = () => navigate("Projeler");
  const projectRow = (name) =>
    page.locator(".dock-project").filter({
      has: page.getByText(name, { exact: true }),
    });
  const projectUrl = async (id) => {
    const project = (await harness.call("list")).find(
      (project) => project.manifest.id === id,
    );
    assert.ok(project.port);
    return `http://127.0.0.1:${project.port}`;
  };
  const fetchSite = (url, host = "127.0.0.1", ca) =>
    new Promise((resolve, reject) => {
      const address = new URL(url);
      const secure = address.protocol === "https:";
      const request = secure ? httpsRequest : httpRequest;
      const req = request(
        {
          host,
          port: address.port || (secure ? 443 : 80),
          ...(secure ? { servername: address.hostname, ca } : {}),
          path: address.pathname + address.search,
          headers: { Host: address.host },
        },
        (response) => {
          let body = "";
          response.setEncoding("utf8");
          response.on("data", (chunk) => {
            body += chunk;
          });
          response.on("end", () =>
            resolve({ status: response.statusCode, body }),
          );
          response.on("error", reject);
        },
      );
      req.on("error", reject);
      req.end();
    });
  assert.equal((await harness.rpc("stack_set", { running: true })).ok, false);
  await expect(
    page.getByRole("button", { name: "Tümünü başlat", exact: true }),
  ).toHaveCount(0);
  if (process.env.DOCKON_WORKSPACE_SMOKE === "1") {
    await expect(
      page.getByText("fixture-project", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("outside-project", { exact: true }),
    ).toHaveCount(0);
    await expect(page.getByText("nested", { exact: true })).toHaveCount(0);
    await page.getByRole("button", { name: "Ayarla", exact: true }).click();
    await expect(page.getByLabel("Proje adı", { exact: true })).toHaveValue(
      "fixture-project",
    );
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
  await page.getByRole("button", { name: "Menü", exact: true }).click();
  await page.getByRole("button", { name: "Manifest içe aktar", exact: true }).click();
  const importDialog = page.getByRole("dialog", { name: "Manifest içe aktar" });
  await importDialog
    .getByLabel("Manifest dosyasının tam yolu", { exact: true })
    .fill(manifest);
  await importDialog
    .getByRole("button", { name: "Manifest içe aktar", exact: true })
    .click();
  const importedCard = projectRow("UI example");
  await expect(importedCard).toBeVisible();
  await harness.call("start", { id: "ui-example" });
  await expect(
    importedCard.getByText("Çalışıyor", { exact: true }),
  ).toBeVisible({ timeout: 15000 });
  await expect(
    importedCard.getByRole("button", { name: "Tarayıcıda aç", exact: true }),
  ).toBeVisible();
  const link = await projectUrl("ui-example");
  assert.equal((await fetch(link)).status, 200);
  await importedCard.getByRole("button", { name: "Proje işlemleri" }).click();
  await importedCard
    .getByRole("button", { name: "Günlükler", exact: true })
    .click();
  await expect(page.getByRole("log")).toContainText("ui-example");
  await back();
  await mkdir(".local/screenshots", { recursive: true });
  await page.screenshot({
    path: ".local/screenshots/desktop-running.png",
    fullPage: true,
  });
  await importedCard
    .getByRole("button", { name: "Durdur", exact: true })
    .click();
  await importedCard
    .getByRole("button", { name: "Başlat", exact: true })
    .waitFor();
  await assert.rejects(fetch(link));
  const imported = (await harness.call("list")).find(
    (entry) => entry.manifest.id === "ui-example",
  );
  assert.ok(
    !imported.manifest.services?.some((entry) => entry.id === "mailpit"),
  );
  await navigate("Ayarlar");
  for (const label of [
    "Apache",
    "Nginx",
    "PHP",
    "PHP FastCGI",
    "Node.js",
    "Redis Cache",
    "Memcached",
  ]) {
    await expect(
      page.getByRole("button", { name: label, exact: true }),
    ).toBeVisible();
  }
  await page.getByRole("button", { name: "Memcached", exact: true }).click();
  await expect(
    page.getByRole("combobox", { name: "Sürüm", exact: true }),
  ).toHaveValue(/1[.]6[.]/);
  await page.getByRole("button", { name: "Node.js", exact: true }).click();

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
  await back();
  await page.getByRole("button", { name: "Yeni proje", exact: true }).click();
  const wizard = page.getByRole("dialog", {
    name: "Proje hazırla",
    exact: true,
  });
  const projectKind = wizard.getByLabel("Proje türü", { exact: true });
  await projectKind.selectOption("php");
  const phpTemplate = wizard.getByLabel("Proje şablonu");
  for (const template of ["laravel", "codeigniter", "cakephp", "symfony"])
    await expect(
      phpTemplate.locator(`option[value="${template}"]`),
    ).toHaveCount(1);
  await expect(
    wizard.getByRole("combobox", { name: "Composer", exact: true }),
  ).toBeVisible();
  const webServer = wizard.getByLabel("Web sunucusu", { exact: true });
  await expect(webServer).toHaveValue("apache");
  await webServer.selectOption("nginx");
  await expect(wizard.getByLabel("PHP CGI", { exact: true })).toBeVisible();
  const databaseChoice = wizard.getByLabel("Veritabanı", { exact: true });
  await databaseChoice.selectOption("postgres");
  await expect(databaseChoice).toHaveValue("postgres");
  await databaseChoice.selectOption("");
  await projectKind.selectOption("application");
  await wizard.getByLabel("Proje adı", { exact: true }).fill("Wizard Node");
  await expect(
    wizard.getByText("wizard-node.test", { exact: true }),
  ).toBeVisible();
  await wizard
    .getByLabel("Çalışma zamanı ve sürüm", { exact: true })
    .selectOption({
      label: (
        await wizard.locator("#project-runtime option").allTextContents()
      ).find((t) => t.startsWith("node ")),
    });
  const testMail = existsSync(
    join(dirname(appPath), "bin/mailpit/mailpit.exe"),
  );
  if (testMail) await wizard.getByLabel("Proje postasını etkinleştir").check();
  await wizard
    .getByRole("button", { name: "Projeyi oluştur", exact: true })
    .click();
  const card = projectRow("Wizard Node");
  await card.getByRole("button", { name: "Proje işlemleri" }).click();
  await expect(
    card.getByRole("button", { name: "Proje terminali", exact: true }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  if (process.env.DOCKON_CACHE_SMOKE === "1") {
    await card.getByRole("button", { name: "Proje işlemleri" }).click();
    await card.getByRole("button", { name: "Proje ayarları" }).click();
    const settings = page.getByRole("dialog", { name: "Proje ayarları" });
    for (const service of ["Redis Cache", "Memcached"]) {
      await settings
        .getByRole("button", { name: service, exact: true })
        .click();
      await settings
        .getByRole("button", { name: "İndir ve projeye ata" })
        .click();
      await expect(settings.getByText(/Atanan sürüm · [0-9]/)).toBeVisible({
        timeout: 210000,
      });
    }
    await settings.getByRole("button", { name: "Kapat", exact: true }).click();
  }

  await harness.call("start", { id: "wizard-node" });
  await expect(card.getByText("Çalışıyor", { exact: true })).toBeVisible({
    timeout: 20000,
  });
  assert.match(
    await (await fetch(await projectUrl("wizard-node"))).text(),
    /dockon · Node.js/,
  );
  if (
    (await harness.call("runtime_installed")).some(
      (entry) => entry.runtime === "caddy",
    )
  ) {
    await expect
      .poll(
        async () => {
          const { settings, running } = await harness.call("router_status");
          if (
            !running ||
            !settings.routes.some((route) => route.project_id === "wizard-node")
          )
            return 0;
          return (
            await fetchSite(
              `http://wizard-node.localhost:${settings.http_port}/`,
            ).catch(() => ({}))
          ).status;
        },
        { timeout: 25000 },
      )
      .toBe(200);
  }
  const configured = (await harness.call("list")).find(
    (entry) => entry.manifest.id === "wizard-node",
  );
  if (process.env.DOCKON_CACHE_SMOKE === "1") {
    for (const service of ["redis", "memcached"]) {
      assert.equal(
        configured.services.find((entry) => entry.id === service).state,
        "RUNNING",
      );
      const cache = configured.services.find((entry) => entry.id === service);
      assert.ok(cache.pid);
      assert.equal(cache.container_id, null);
    }
  }
  const inbox = configured.services.find((entry) => entry.id === "mailpit");
  assert.equal(Boolean(inbox), testMail);
  if (inbox) {
    await harness.call("project_set_mail", { id: "ui-example", enabled: true });
    await harness.call("start", { id: "ui-example" });
    await expect(
      importedCard.getByText("Çalışıyor", { exact: true }),
    ).toBeVisible({ timeout: 20000 });
    const peers = await harness.call("list");
    const otherInbox = peers
      .find((entry) => entry.manifest.id === "ui-example")
      .services.find((entry) => entry.id === "mailpit");
    assert.notEqual(inbox.smtp_port, otherInbox.smtp_port);
    let hosts;
    if (
      (await harness.call("runtime_installed")).some(
        (entry) => entry.runtime === "caddy",
      )
    ) {
      await expect
        .poll(
          async () => {
            const status = await harness.call("router_status");
            return (
              status.running &&
              ["wizard-node", "ui-example"].every((id) =>
                status.settings.routes.some((route) => route.project_id === id),
              )
            );
          },
          { timeout: 25000 },
        )
        .toBe(true);
    }
    const router = await harness.call("router_status");
    if (router.running) {
      const { hosts_entries } = await harness.call("router_plan", {
        settings: router.settings,
      });
      hosts = new Map(hosts_entries.map((entry) => entry.split(" ").reverse()));
    }
    const smtpHost = hosts?.get("mail.wizard-node.test");
    if (router.running) {
      assert.match(smtpHost, /^127[.]/);
      assert.notEqual(smtpHost, hosts.get("mail.hello-http.test"));
    }
    const socket = connect(
      smtpHost ? 25 : inbox.smtp_port,
      smtpHost ?? "127.0.0.1",
    );
    socket.setTimeout(5000, () => socket.destroy());
    const replies = createInterface({ input: socket })[Symbol.asyncIterator]();
    const expectSmtp = async (code) => {
      let line;
      do {
        line = (await replies.next()).value;
        assert.ok(line);
      } while (line[3] === "-");
      assert.equal(Number(line.slice(0, 3)), code);
    };
    try {
      await expectSmtp(220);
      for (const [command, code] of [
        ["EHLO localhost", 250],
        ["MAIL FROM:<sender@example.test>", 250],
        ["RCPT TO:<receiver@example.test>", 250],
        ["DATA", 354],
        ["Subject: Project fixture\r\n\r\nFixture\r\n.", 250],
        ["QUIT", 221],
      ]) {
        socket.write(command + "\r\n");
        await expectSmtp(code);
      }
    } finally {
      socket.destroy();
    }
    const count = async ({ port }) =>
      (await (await fetch(`http://127.0.0.1:${port}/api/v1/messages`)).json())
        .total;
    assert.equal(await count(inbox), 1);
    assert.equal(await count(otherInbox), 0);
    if (router.running) {
      assert.equal(hosts.get("wizard-node.test"), "127.77.0.1");
      const projectSite = await fetchSite(
        "http://wizard-node.test/",
        hosts.get("wizard-node.test"),
      );
      assert.equal(projectSite.status, 200);
      assert.match(projectSite.body, /dockon · Node.js/);
      assert.equal(hosts.get("webmail.wizard-node.test"), "127.77.0.1");
      const standard = await fetchSite(
        "http://webmail.wizard-node.test/",
        hosts.get("webmail.wizard-node.test"),
      );
      assert.equal(standard.status, 200);
      assert.match(standard.body, /Mailpit/i);
      const messages = await fetchSite(
        "http://mail.wizard-node.test/api/v1/messages",
        smtpHost,
      );
      assert.equal(messages.status, 200);
      assert.equal(JSON.parse(messages.body).total, 1);
      const address = `http://webmail.wizard-node.localhost:${router.settings.http_port}/`;
      await expect
        .poll(
          async () => {
            const result = await fetchSite(
              new URL("/api/v1/messages", address),
            ).catch(() => null);
            return result?.status === 200
              ? JSON.parse(result.body).total
              : null;
          },
          {
            timeout: 15000,
          },
        )
        .toBe(1);
      const named = await browser.contexts()[0].newPage();
      await named.goto(address);
      await expect(named).toHaveTitle(/Mailpit/i);
      await expect(
        named.getByText("Project fixture", { exact: true }),
      ).toBeVisible();
      await named.close();
    }
    await importedCard
      .getByRole("button", { name: "Durdur", exact: true })
      .click();
    await expect(
      importedCard.getByRole("button", { name: "Başlat", exact: true }),
    ).toBeEnabled();
    console.log("PASS isolated project SMTP and named Mailpit inbox");
  }
  await card.getByRole("button", { name: "Durdur", exact: true }).click();
  await card.getByRole("button", { name: "Başlat", exact: true }).waitFor();
  const alternateNode = join(data, "alternate-node.exe");
  await copyFile(process.execPath, alternateNode);
  const alternate = await harness.call("runtime_use_system", {
    runtime: "node",
    executable: alternateNode,
  });
  await card.getByRole("button", { name: "Proje işlemleri" }).click();
  await card.getByRole("button", { name: "Proje ayarları" }).click();
  const projectSettings = page.getByRole("dialog", { name: "Proje ayarları" });
  const runtimeSelect = projectSettings.getByRole("combobox", {
    name: "Node.js sürümü",
    exact: true,
  });
  await expect(
    runtimeSelect.locator(`option[value="${alternate.id}"]`),
  ).toHaveCount(1);
  await runtimeSelect.selectOption(alternate.id);
  await expect
    .poll(
      async () =>
        (await harness.call("list")).find(
          (entry) => entry.manifest.id === "wizard-node",
        ).manifest.runtime.installation_id,
    )
    .toBe(alternate.id);
  const projectBins = await harness.call("project_runtime_paths", {
    id: "wizard-node",
  });
  assert.equal(projectBins[0], data);
  if (process.env.DOCKON_CACHE_SMOKE === "1")
    assert.equal(projectBins.length, 3);
  assert.deepEqual(
    await harness.call("project_runtime_paths", { id: "ui-example" }),
    [],
  );
  await projectSettings
    .getByRole("button", { name: "Posta ve adres", exact: true })
    .click();
  await expect(
    projectSettings.getByRole("checkbox", { name: "SSL kullan (HTTPS)" }),
  ).toBeVisible();
  await projectSettings
    .getByRole("button", { name: "Kapat", exact: true })
    .click();
  await card.getByRole("button", { name: "Proje işlemleri" }).click();
  await expect(
    card.getByRole("button", { name: "Proje terminali", exact: true }),
  ).toBeEnabled();
  await page.keyboard.press("Escape");
  const terminalPid = await page.evaluate(() =>
    window.__TAURI_INTERNALS__.invoke("project_terminal", {
      id: "wizard-node",
    }),
  );
  assert.ok(Number.isInteger(terminalPid) && terminalPid > 0);
  await promisify(execFile)(
    "powershell.exe",
    [
      "-NoProfile",
      "-Command",
      `
$terminal = Get-Process -Id ${terminalPid} -ErrorAction Stop
if ($terminal.ProcessName -ne 'powershell') { throw 'Unexpected project terminal process' }
Stop-Process -Id $terminal.Id
`,
    ],
    { windowsHide: true },
  );
  await navigate("Tanılama");
  const report = page
    .getByRole("region", { name: "Durum raporu" })
    .locator("pre");
  await report.waitFor();
  const reportText = await report.textContent();
  assert.equal(JSON.parse(reportText).project_counts.STOPPED, 2);
  assert.ok(JSON.parse(reportText).runtimes.installed >= 1);
  assert.doesNotMatch(
    reportText,
    /ui-example|wizard-node|executable|password|port|token/i,
  );
  await expect(page.getByRole("button", { name: "JSON indir" })).toBeEnabled();
  if (process.env.DOCKON_GUARDIAN_PREVIEW === "1") {
    await page
      .getByRole("button", { name: "Veriyi önizle", exact: true })
      .click();
    await expect(
      page.getByText("https://ai.ideavent.tr/v1", { exact: true }),
    ).toBeVisible();
    await expect(page.getByLabel("Sağlayıcı API anahtarı")).toHaveAttribute(
      "type",
      "password",
    );
  }
  await back();
  console.log(
    "PASS native runtime opt-in and new project wizard with real selected-runtime HTTP server",
  );
  if (process.env.DOCKON_CADDY_EXECUTABLE) {
    await navigate("Ayarlar");
    await page.getByRole("button", { name: "Caddy", exact: true }).click();
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
    await back();
  }
  if (
    (await harness.call("runtime_installed")).some(
      (entry) => entry.runtime === "caddy",
    )
  ) {
    await harness.call("start", { id: "wizard-node" });
    await expect(card.getByText("Çalışıyor", { exact: true })).toBeVisible({
      timeout: 20000,
    });
    await navigate("Yerel adresler");
    await expect(
      page.getByRole("heading", { name: "Mevcut adresler" }),
    ).toBeVisible();
    await expect(
      page.getByText("http://wizard-node.test/", { exact: true }),
    ).toBeVisible();
    await page.getByLabel("Alan adı uzantısı").fill(".devbox");
    await page.getByLabel("Yeni projelerde varsayılan SSL").check();
    await page.getByRole("button", { name: "Kaydet", exact: true }).click();
    await expect(
      page.getByText("http://wizard-node.devbox/", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Yerel adresleri etkinleştir" }),
    ).toBeVisible();
    const rerouted = await harness.call("router_status");
    assert.equal(rerouted.settings.domain_suffix, "devbox");
    assert.equal(rerouted.settings.default_https, true);
    await expect
      .poll(
        async () =>
          (
            await fetchSite("http://wizard-node.devbox/", "127.77.0.1").catch(
              () => null,
            )
          )?.status,
        { timeout: 15000 },
      )
      .toBe(200);
    await back();

    await card.getByRole("button", { name: "Proje işlemleri" }).click();
    await card.getByRole("button", { name: "Proje ayarları" }).click();
    const settings = page.getByRole("dialog", { name: "Proje ayarları" });
    await settings.getByRole("button", { name: "Posta ve adres" }).click();
    const ssl = settings.getByRole("checkbox", { name: "SSL kullan (HTTPS)" });
    await expect(ssl).not.toBeChecked();
    await ssl.click();
    await expect(
      settings.getByText("https://wizard-node.devbox", { exact: true }),
    ).toBeVisible();
    await ssl.click();
    await expect(
      settings.getByText("http://wizard-node.devbox", { exact: true }),
    ).toBeVisible();
    await settings.getByRole("button", { name: "Kapat", exact: true }).click();
    const routing = (await harness.call("router_status")).settings;
    routing.routes.find((route) => route.project_id === "ui-example").domain =
      "ssl-default-fixture.devbox";
    const reservation = await harness.call("router_plan", {
      settings: routing,
    });
    await harness.call("router_apply", { token: reservation.token });
    await harness.call("workspace_create", {
      slug: "ssl-default-fixture",
      name: "SSL default fixture",
      runtime_installation: configured.manifest.runtime.installation_id,
    });
    await harness.call("start", { id: "ssl-default-fixture" });
    await expect
      .poll(
        async () => {
          const status = await harness.call("router_status");
          return status.settings.routes.find(
            (route) => route.project_id === "ssl-default-fixture",
          );
        },
        { timeout: 25000 },
      )
      .toMatchObject({
        domain: "ssl-default-fixture-dockon-1.devbox",
        https: true,
      });
    const { root_certificate: certificate } =
      await harness.call("router_status");
    const ca = await readFile(certificate);
    const sites = await Promise.all([
      fetchSite("http://wizard-node.devbox/", "127.77.0.1"),
      fetchSite(
        "https://ssl-default-fixture-dockon-1.devbox/",
        "127.77.0.1",
        ca,
      ),
    ]);
    assert.deepEqual(
      sites.map(({ status }) => status),
      [200, 200],
    );
    console.log(
      "PASS local domain suffix, SSL default and per-project SSL controls",
    );
  }
  await back();
  await harness.call("start", { id: "ui-example" });
  await expect(
    importedCard.getByText("Çalışıyor", { exact: true }),
  ).toBeVisible();
  const backgroundUrl = await projectUrl("ui-example");
  const closeWindow = `
Add-Type -Namespace DockonSmoke -Name Windows -MemberDefinition @'
  public delegate bool WindowVisitor(IntPtr window, IntPtr data);
  [DllImport("user32.dll")]
  public static extern bool EnumWindows(WindowVisitor visitor, IntPtr data);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)]
  public static extern int GetWindowText(IntPtr window, System.Text.StringBuilder text, int length);
  [DllImport("user32.dll")]
  public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
  [DllImport("user32.dll")]
  public static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
  [DllImport("user32.dll")]
  public static extern bool IsWindowVisible(IntPtr window);
  public static IntPtr FindVisibleDockon(uint pid) {
    IntPtr result = IntPtr.Zero;
    EnumWindows((window, data) => {
      uint owner;
      GetWindowThreadProcessId(window, out owner);
      if (owner != pid || !IsWindowVisible(window)) return true;
      var text = new System.Text.StringBuilder(128);
      GetWindowText(window, text, text.Capacity);
      if (text.ToString() != "Dockon") return true;
      result = window;
      return false;
    }, IntPtr.Zero);
    return result;
  }
'@
$window = [DockonSmoke.Windows]::FindVisibleDockon(${app.pid})
if ($window -eq [IntPtr]::Zero) {
  $title = (Get-Process -Id ${app.pid}).MainWindowTitle
  throw "Dockon test window is not visible (main title=$title)"
}
if (![DockonSmoke.Windows]::PostMessage($window, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)) { throw 'Could not close Dockon test window' }
for ($i = 0; $i -lt 30 -and [DockonSmoke.Windows]::IsWindowVisible($window); $i++) { Start-Sleep -Milliseconds 100 }
if ([DockonSmoke.Windows]::IsWindowVisible($window)) { throw 'Dockon window did not hide' }
`;
  await promisify(execFile)(
    "powershell.exe",
    ["-NoProfile", "-Command", closeWindow],
    { windowsHide: true },
  );
  assert.equal(app.exitCode, null);
  assert.equal((await fetch(backgroundUrl)).status, 200);
  console.log(
    "PASS closing the native window hides it and leaves project HTTP running",
  );
  console.log(
    "PASS native Tauri/WebView2 import, real process startup, HTTP readiness, streaming logs and stop",
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
