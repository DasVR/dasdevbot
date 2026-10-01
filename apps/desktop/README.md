# apps/desktop

Phase 0 client: Svelte 5 + Vite. The daemon serves `dist/` and the UI talks to it over loopback HTTP (`/v1`, the versioned messages in `crates/proto`).

The Tauri 2 shell (`src-tauri/`) is the Windows window: full, companion, and the floating pill. The geometry lives in `src-tauri/src/geometry.rs` and is tested by `cargo test` in `src-tauri/`. The webview calls those commands when it is actually inside Tauri. In a browser, `?shellStage=1` paints the same three forms on the mock desk, which is how the review stills are taken. Linux WebKitGTK does not give a reliable transparent blurred pill, so the captures stay in the browser stage.

```bash
npm install
npm run build
```

`npm run dev` proxies `/v1` to `http://127.0.0.1:8787` while `dasdevbotd` is running.

## Decisions happen in the card window

Inside Tauri the main window cannot approve, deny, or undo. Approve opens the `card` window, and only that window's capability (`capabilities/card-window.json`) can call `sign_decision` and `undo_decision`. HTTP decisions stay 403. `npm run card-window` checks this with a stubbed IPC.

## Windows demo installer (unsigned, not for release)

The bundle target is NSIS, per-user (`installMode: currentUser`, no elevation), with no updater, no service, and no startup entry. The uninstaller removes the binaries. It offers an unchecked "Also delete my data" box that removes `%APPDATA%\net.dasdev.dasdevbot` and `%LOCALAPPDATA%\net.dasdev.dasdevbot`.

The demo build bundles a `dasdevbotd` built with `--no-default-features` (no iroh UDP bind). With the `demo-daemon` feature the app starts it as `serve --role executor --provider mock --bind 127.0.0.1:8787`: loopback only, no `--allow-remote`, a cleared environment, and no bundled secrets.

```
cargo build --release -p dasdevbotd --no-default-features
copy target\release\dasdevbotd.exe apps\desktop\src-tauri\binaries\dasdevbotd-x86_64-pc-windows-msvc.exe
cd apps/desktop
npx @tauri-apps/cli@2 build --bundles nsis --features demo-daemon --config src-tauri/tauri.demo.conf.json
```

The installer is unsigned. It is a CI artifact only, never a release asset.
