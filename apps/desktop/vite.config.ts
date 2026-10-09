import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { svelte } from "@sveltejs/vite-plugin-svelte";
import type { Plugin } from "vite";
import { defineConfig } from "vite";

const root = path.dirname(fileURLToPath(import.meta.url));

function escapeAttr(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll('"', "&quot;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}

/** The daemon writes this beside the default sqlite file. It is not an API. */
function sessionTokenPlugin(): Plugin {
  const tokenPath = path.resolve(root, "../../data/dasdevbot.sqlite.token");
  return {
    name: "dasdevbot-session-token",
    apply: "serve",
    transformIndexHtml(html: string): string {
      let token = "";
      try {
        token = fs.readFileSync(tokenPath, "utf8").trim();
      } catch {
        token = "";
      }
      const meta = `<meta name="dasdevbot-token" content="${escapeAttr(token)}" />`;
      return html.replace("<head>", `<head>\n    ${meta}`);
    },
  };
}

export default defineConfig({
  plugins: [svelte(), sessionTokenPlugin()],
  build: {
    rollupOptions: {
      input: {
        main: path.resolve(root, "index.html"),
        gallery: path.resolve(root, "gallery.html"),
      },
    },
  },
  server: {
    port: 5173,
    proxy: {
      "/v1": {
        target: "http://127.0.0.1:8787",
        // Dev only. The daemon rejects any Host but its own loopback address
        // (DNS-rebinding guard, server.rs host_is_loopback) and the browser
        // sends localhost:5173, so the proxy rewrites Host to the target.
        changeOrigin: true,
        configure(proxy) {
          proxy.on("proxyReq", (proxyReq) => {
            proxyReq.removeHeader("origin");
          });
        },
      },
    },
  },
});
