/**
 * The Ctrl+Alt+Shift+F C1 trigger is dev-only (CD ruling c, UX 2): it sits
 * behind `import.meta.env.DEV`. Run after `vite build`: the release bundle in
 * apps/desktop/dist must not carry the hotkey or the forced `repo.force_push`
 * emit. A development build (NODE_ENV=development, DEV true) into a scratch directory must
 * carry both, so the check cannot pass vacuously.
 */
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../apps/desktop");
const MARKERS = [/"KeyF"/, /repo\.force_push/, /forced:\s*(?:!0|true)/];

function scripts(dir) {
  const assets = path.join(dir, "assets");
  return fs
    .readdirSync(assets)
    .filter((name) => name.endsWith(".js"))
    .map((name) => fs.readFileSync(path.join(assets, name), "utf8"))
    .join("\n");
}

const release = scripts(path.join(desktop, "dist"));
const leaked = MARKERS.filter((marker) => marker.test(release));
if (leaked.length > 0) {
  console.error(`demo-trigger-gate: the release bundle carries the C1 trigger: ${leaked.join(", ")}`);
  process.exit(1);
}

const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "dasdevbot-dev-"));
const vite = path.join(desktop, "node_modules", "vite", "bin", "vite.js");
const built = spawnSync(process.execPath, [vite, "build", "--mode", "development", "--outDir", scratch, "--emptyOutDir"], {
  cwd: desktop,
  // `--mode` alone keeps a production build; NODE_ENV makes import.meta.env.DEV true.
  env: { ...process.env, NODE_ENV: "development" },
  stdio: ["ignore", "ignore", "inherit"],
});
if (built.status !== 0) {
  console.error("demo-trigger-gate: the development build failed");
  process.exit(1);
}
const demo = scripts(scratch);
fs.rmSync(scratch, { recursive: true, force: true });
const missing = MARKERS.filter((marker) => !marker.test(demo));
if (missing.length > 0) {
  console.error(`demo-trigger-gate: the development bundle lost the C1 trigger: ${missing.join(", ")}`);
  process.exit(1);
}
console.log("demo-trigger-gate: release bundle has no C1 trigger; development bundle has it");
