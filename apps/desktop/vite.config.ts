import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

const root = path.dirname(fileURLToPath(import.meta.url));

/**
 * The daemon writes this beside the default sqlite file. It is not an API.
 * #36 H1: dev only, and only on the server side of the proxy. The bearer is
 * attached to the proxied `/v1` request and never reaches HTML or the page.
 */
const tokenPath = path.resolve(root, "../../data/dasdevbot.sqlite.token");

function devBearer(): string {
  try {
    return fs.readFileSync(tokenPath, "utf8").trim();
  } catch {
    return "";
  }
}

/**
 * Only the dev page itself gets the bearer attached. The browser must say
 * `Sec-Fetch-Site: same-origin` (SD on #46). A cross-site page posting to the
 * dev server (CSRF), and any headerless local client (curl, a script, or
 * anything hitting `vite preview`, which inherits this proxy), is forwarded
 * without the bearer, so the daemon answers 401.
 */
function fromDevPage(headers: Record<string, string | string[] | undefined>): boolean {
  if (headers["sec-fetch-site"] !== "same-origin") {
    return false;
  }
  const origin = headers.origin;
  if (origin !== undefined && !/^http:\/\/(localhost|127\.0\.0\.1):5173$/.test(String(origin))) {
    return false;
  }
  return true;
}

export default defineConfig({
  plugins: [svelte()],
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
          proxy.on("proxyReq", (proxyReq, req) => {
            const fromPage = fromDevPage(req.headers);
            proxyReq.removeHeader("origin");
            proxyReq.removeHeader("authorization");
            const bearer = fromPage ? devBearer() : "";
            if (bearer) {
              proxyReq.setHeader("Authorization", `Bearer ${bearer}`);
            }
          });
        },
      },
    },
  },
});
