/**
 * 2x thread stills at 1280×800. TZ America/New_York.
 * The pointer stays on the wordmark, except the composer hover shot.
 * Pen marks are allowed to finish before the capture.
 *
 * Usage: node scripts/thread-shots.mjs [origin]
 * Expects the desktop preview (default http://127.0.0.1:4173).
 */
import { createRequire } from "node:module";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const require = createRequire(new URL("../apps/desktop/package.json", import.meta.url));
const { chromium } = require("playwright");

const origin = process.argv[2] ?? "http://127.0.0.1:4173";
const outDir = fileURLToPath(new URL("../docs/review/thread/", import.meta.url));
await mkdir(outDir, { recursive: true });

const browser = await chromium.launch({
  channel: process.env.FONT_PROOF_CHANNEL || "chrome",
});
const page = await browser.newPage({
  viewport: { width: 1280, height: 800 },
  timezoneId: "America/New_York",
  deviceScaleFactor: 2,
});

const shots = [
  ["send-landing", "/?shot=send", "wordmark"],
  ["tools-live", "/?shot=tools", "wordmark"],
  ["approval-waiting", "/?shot=approval", "wordmark"],
  ["receipt-filed", "/?shot=receipt", "wordmark"],
  ["handoff", "/?shot=handoff", "wordmark"],
  ["composer-hover", "/?shot=composer", "composer"],
];

async function park(where) {
  const selector = where === "composer" ? "form.composer" : ".wordmark";
  const box = await page.locator(selector).boundingBox();
  if (!box) {
    throw new Error(`${selector} missing`);
  }
  await page.mouse.move(box.x + box.width * 0.72, box.y + box.height * 0.45);
}

for (const [name, path, pointer] of shots) {
  await page.goto(`${origin}${path}`, { waitUntil: "networkidle" });
  await page.locator(".wordmark").waitFor();
  if (name === "approval-waiting" || name === "composer-hover") {
    await page.locator("article.card").last().waitFor();
  }
  if (name === "tools-live") {
    await page.getByText("Running the core tests").waitFor();
  }
  if (name === "handoff") {
    await page.getByText("Reviewer hands off to Builder").waitFor();
  }
  if (name === "composer-hover") {
    await page.locator("form.composer.lit").waitFor();
  }
  // Checks, the handoff pen, and the risk arrow finish inside this window.
  await page.waitForTimeout(1600);
  await park(pointer);
  await page.waitForTimeout(120);
  await page.screenshot({ path: `${outDir}/${name}.png` });
  console.log(`wrote ${name}.png`);
}

await browser.close();
