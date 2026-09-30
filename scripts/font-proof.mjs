/**
 * Proves the bundled Figtree face is what renders.
 * - every FontFace in document.fonts is status "loaded"
 * - font requests stay on this origin (the vendored woff2 files)
 * - CSS.getPlatformFontsForNode on the wordmark reports Figtree, isCustomFont true
 *
 * A machine-wide Figtree install is not enough: isCustomFont must be true.
 */
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const require = createRequire(new URL("../apps/desktop/package.json", import.meta.url));
const { chromium } = require("playwright");

const desktop = fileURLToPath(new URL("../apps/desktop/", import.meta.url));
const viteBin = fileURLToPath(new URL("../apps/desktop/node_modules/vite/bin/vite.js", import.meta.url));
const port = Number(process.env.FONT_PROOF_PORT ?? 4173);
const origin = `http://127.0.0.1:${port}`;

const blockedHosts = ["fonts.googleapis.com", "fonts.gstatic.com", "cdn.jsdelivr.net", "fontsource.org"];

function launchBrowser() {
  const channel = process.env.FONT_PROOF_CHANNEL;
  if (channel) {
    return chromium.launch({ channel });
  }
  return chromium.launch({ channel: "chrome" }).catch(() => chromium.launch());
}

function waitForPreview(child) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("vite preview did not start")), 30000);
    const onData = (buf) => {
      const text = buf.toString();
      if (text.includes("Local:") || text.includes(origin)) {
        clearTimeout(timer);
        resolve();
      }
    };
    child.stdout.on("data", onData);
    child.stderr.on("data", onData);
    child.on("exit", (code) => {
      clearTimeout(timer);
      reject(new Error(`vite preview exited ${code}`));
    });
  });
}

const preview = spawn(process.execPath, [viteBin, "preview", "--host", "127.0.0.1", "--port", String(port), "--strictPort"], {
  cwd: desktop,
  stdio: ["ignore", "pipe", "pipe"],
});

let browser;
try {
  await waitForPreview(preview);
  browser = await launchBrowser();
  const page = await browser.newPage();
  const fontRequests = [];
  page.on("request", (request) => {
    const url = request.url();
    const type = request.resourceType();
    if (type === "font" || /\.woff2(\?|$)/.test(url)) {
      fontRequests.push(url);
    }
  });

  await page.goto(origin, { waitUntil: "networkidle" });

  const faces = await page.evaluate(async () => {
    const probe = document.createElement("div");
    probe.setAttribute("data-font-probe", "true");
    probe.style.cssText = "position:absolute;left:0;top:0;opacity:0;pointer-events:none";
    const sample = "AĄ";
    for (const family of ["Figtree", "Onest", "JetBrains Mono"]) {
      const node = document.createElement("p");
      node.style.fontFamily = `"${family}"`;
      node.textContent = sample;
      probe.appendChild(node);
    }
    document.body.appendChild(probe);
    const loads = [];
    for (const spec of [
      '400 16px "Figtree"',
      '620 16px "Figtree"',
      '400 16px "Onest"',
      '400 16px "JetBrains Mono"',
    ]) {
      loads.push(document.fonts.load(spec, sample));
    }
    await Promise.all(loads);
    await document.fonts.ready;
    return [...document.fonts].map((face) => ({
      family: face.family,
      weight: face.weight,
      style: face.style,
      status: face.status,
      unicodeRange: face.unicodeRange,
    }));
  });

  const session = await page.context().newCDPSession(page);
  await session.send("DOM.enable");
  await session.send("CSS.enable");
  const { root } = await session.send("DOM.getDocument");
  const { nodeId } = await session.send("DOM.querySelector", {
    nodeId: root.nodeId,
    selector: ".wordmark strong",
  });
  if (!nodeId) {
    throw new Error("wordmark text node was not found");
  }
  const platform = await session.send("CSS.getPlatformFontsForNode", { nodeId });
  const computed = await page.locator(".wordmark strong").evaluate((node) => {
    const style = getComputedStyle(node);
    return { fontFamily: style.fontFamily, fontWeight: style.fontWeight };
  });

  const unloaded = faces.filter((face) => face.status !== "loaded");
  const remote = fontRequests.filter((url) => {
    const host = new URL(url).host;
    return host !== new URL(origin).host || blockedHosts.some((blocked) => host.endsWith(blocked));
  });
  const figtreeCustom = (platform.fonts ?? []).filter((font) => {
    const name = `${font.familyName} ${font.postScriptName ?? ""}`;
    return /figtree/i.test(name) && font.isCustomFont === true && font.glyphCount > 0;
  });

  const report = {
    origin,
    computed,
    faces,
    fontRequests,
    platformFonts: platform.fonts ?? [],
  };
  console.log(JSON.stringify(report, null, 2));

  const failures = [];
  if (faces.length === 0) {
    failures.push("no FontFace entries");
  }
  if (unloaded.length > 0) {
    failures.push(`FontFace not loaded: ${unloaded.map((face) => `${face.family} ${face.weight} ${face.status}`).join(", ")}`);
  }
  if (!faces.some((face) => face.family.replaceAll('"', "") === "Figtree")) {
    failures.push("Figtree is not a bundled FontFace");
  }
  if (!faces.some((face) => face.family.replaceAll('"', "") === "JetBrains Mono")) {
    failures.push("JetBrains Mono is not a bundled FontFace");
  }
  if (fontRequests.length === 0) {
    failures.push("no font requests were observed");
  }
  if (remote.length > 0) {
    failures.push(`font requests left the origin: ${remote.join(", ")}`);
  }
  if (!/Figtree/.test(computed.fontFamily)) {
    failures.push(`wordmark font-family is ${computed.fontFamily}`);
  }
  if (figtreeCustom.length === 0) {
    failures.push("CSS.getPlatformFontsForNode did not report Figtree with isCustomFont true");
  }
  if (failures.length > 0) {
    throw new Error(failures.join("\n"));
  }
  console.log("font-proof: pass");
} finally {
  if (browser) {
    await browser.close();
  }
  preview.kill("SIGTERM");
}
