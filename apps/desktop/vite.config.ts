import { fileURLToPath } from "node:url";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

const root = fileURLToPath(new URL(".", import.meta.url));

export default defineConfig({
  plugins: [svelte()],
  build: {
    rollupOptions: {
      input: {
        main: `${root}index.html`,
        gallery: `${root}gallery.html`,
        github: `${root}github-device.html`,
      },
    },
  },
  server: {
    port: 5173,
    proxy: {
      "/v1": "http://127.0.0.1:8787",
    },
  },
});
