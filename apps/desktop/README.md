# apps/desktop

Phase 0 client: Svelte 5 + Vite. The daemon serves `dist/` and the UI talks to it over loopback HTTP (`/v1`, the versioned messages in `crates/proto`).

`src-tauri/tauri.conf.json` is the Tauri 2 scaffold. This spike does not build the Tauri shell. The window is a browser pointed at the daemon, which is enough to prove the IPC round trip without WebKitGTK.

```bash
npm install
npm run build
```

`npm run dev` proxies `/v1` to `http://127.0.0.1:8787` while `dasdevbotd` is running.
