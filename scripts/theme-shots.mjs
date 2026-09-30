/**
 * 1280x800 review shots. TZ America/New_York. Pointer on the titlebar, except hover.
 * Pen marks are allowed to finish before the capture.
 *
 * Usage: node scripts/theme-shots.mjs before|after
 * Expects Vite on 127.0.0.1:5173 and the daemon on 127.0.0.1:8787.
 */
import { createRequire } from "node:module";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const require = createRequire(new URL("../apps/desktop/package.json", import.meta.url));
const { chromium } = require("playwright");

const mode = process.argv[2];
if (mode !== "before" && mode !== "after") {
  console.error("usage: theme-shots.mjs before|after");
  process.exit(1);
}

const outDir = fileURLToPath(new URL("../docs/review/theme-layer/", import.meta.url));
await mkdir(outDir, { recursive: true });

const browser = await chromium.launch({
  channel: process.env.FONT_PROOF_CHANNEL || "chrome",
});
const page = await browser.newPage({
  viewport: { width: 1280, height: 800 },
  timezoneId: "America/New_York",
  deviceScaleFactor: 2,
});

async function park() {
  const box = await page.locator(".titlebar").boundingBox();
  if (!box) {
    throw new Error("titlebar missing");
  }
  await page.mouse.move(box.x + Math.min(180, box.width / 2), box.y + box.height / 2);
}

async function settlePens() {
  await page.waitForTimeout(1400);
}

async function shot(name) {
  await park();
  await settlePens();
  await page.screenshot({ path: `${outDir}/${name}.png` });
  console.log(`wrote ${name}.png`);
}

await page.goto("http://127.0.0.1:5173", { waitUntil: "networkidle" });
await page.locator(".wordmark").waitFor();
await page.waitForTimeout(400);

if (mode === "before") {
  await shot("idle-before");
  await browser.close();
  process.exit(0);
}

await shot("idle");

await page.evaluate(async () => {
  const response = await fetch("/v1/events", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      source: "demo",
      kind: "session.note",
      payload: { note: "a stream row with no approval" },
      idempotency_key: `shot-stream-${crypto.randomUUID()}`,
    }),
  });
  if (!response.ok) {
    throw new Error(await response.text());
  }
});
await page.locator(".stream .row").first().waitFor();
if ((await page.locator("article.card").count()) !== 0) {
  throw new Error("stream shot still has a card");
}
await shot("stream");

const row = page.locator(".stream .row").first();
const rowBox = await row.boundingBox();
if (!rowBox) {
  throw new Error("stream row missing");
}
await page.mouse.move(rowBox.x + rowBox.width / 2, rowBox.y + rowBox.height / 2);
await page.waitForTimeout(200);
await page.screenshot({ path: `${outDir}/hover.png` });
console.log("wrote hover.png");

await park();
await page.getByRole("button", { name: "Simulate repo.push" }).click();
await page.locator("article.card.glass").waitFor();
await shot("waiting");
await page.locator("article.card button.approve").click();
await page.locator("article.card.paper").waitFor();
await page.locator("article.card .receipt").waitFor();
await shot("receipt");

await browser.close();
