import { chromium } from "@playwright/test";
import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";

const browser = await chromium.launch({
  executablePath: process.env.DOCKON_TEST_BROWSER,
});
try {
  const page = await browser.newPage({
    viewport: { width: 410, height: 320 },
  });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(process.env.DOCKON_UI_ORIGIN ?? "http://127.0.0.1:1420");
  await page.getByRole("heading", { name: "Projeler", exact: true }).waitFor();
  await page
    .getByText("Masaüstü bağlantısı gerekli", { exact: true })
    .waitFor();
  assert.equal(
    await page
      .getByRole("button", { name: "Yeni proje", exact: true })
      .isEnabled(),
    false,
  );
  const navigation = page.getByRole("navigation", { name: "Ana gezinme" });
  for (const [label, heading] of [
    ["Günlükler", "Proje günlükleri"],
    ["Yerel adresler", "Yerel adresler"],
    ["Ayarlar", "Sistem ayarları"],
    ["Tanılama", "Tanılama"],
    ["Projeler", "Projeler"],
  ]) {
    const tab = navigation.getByRole("button", { name: label, exact: true });
    await tab.click();
    assert.equal(await tab.getAttribute("aria-current"), "page");
    await page.getByRole("heading", { name: heading, exact: true }).waitFor();
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth > innerWidth,
      ),
      false,
    );
  }
  await navigation
    .getByRole("button", { name: "Ayarlar", exact: true })
    .click();
  assert.equal(
    await page
      .getByRole("switch", { name: "Oturum açınca başlat" })
      .isEnabled(),
    false,
  );
  const motion = page.getByRole("switch", { name: "Animasyonları azalt" });
  await motion.check();
  assert.equal(
    await page
      .locator(".dock-main")
      .evaluate((element) => getComputedStyle(element).animationName),
    "none",
  );
  await page.reload();
  await navigation
    .getByRole("button", { name: "Ayarlar", exact: true })
    .click();
  assert.equal(await motion.isChecked(), true);
  await motion.uncheck();
  await page
    .getByRole("button", { name: "Sürümler ve servisler", exact: true })
    .click();
  await page
    .getByRole("heading", { name: "Sürümler ve servisler", exact: true })
    .waitFor();
  await navigation
    .getByRole("button", { name: "Projeler", exact: true })
    .click();
  await page.getByRole("button", { name: "Menü", exact: true }).click();
  await page.getByRole("button", { name: "English", exact: true }).click();
  await page.getByRole("heading", { name: "Projects", exact: true }).waitFor();
  assert.equal(
    await page.evaluate(
      () => document.documentElement.scrollWidth > innerWidth,
    ),
    false,
  );
  await mkdir(".local/screenshots", { recursive: true });
  await page.screenshot({
    path: ".local/screenshots/foundation.png",
    fullPage: true,
    animations: "disabled",
  });
  assert.deepEqual(errors, []);
  console.log(
    "PASS browser shell, honest disconnected state, localization and desktop layout",
  );
} finally {
  await browser.close();
}
