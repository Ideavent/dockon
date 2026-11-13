import { chromium } from "@playwright/test";
import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";

const browser = await chromium.launch();
try {
  const page = await browser.newPage({
    viewport: { width: 1280, height: 850 },
  });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("http://127.0.0.1:1420");
  await page.getByRole("heading", { name: "Projeler", exact: true }).waitFor();
  await page
    .getByText("Masaüstü bağlantısı gerekli", { exact: true })
    .waitFor();
  assert.equal(
    await page
      .getByRole("button", { name: "Yeni / içe aktar", exact: true })
      .isEnabled(),
    false,
  );
  await page.getByRole("button", { name: "TR", exact: true }).click();
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
  });
  assert.deepEqual(errors, []);
  console.log(
    "PASS browser shell, honest disconnected state, localization and desktop layout",
  );
} finally {
  await browser.close();
}
