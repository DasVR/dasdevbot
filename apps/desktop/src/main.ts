import { mount } from "svelte";
import App from "./App.svelte";
import CardWindow from "./CardWindow.svelte";
import { tauriWindowLabel } from "./lib/api";
import "./app.css";

const target = document.getElementById("app");
if (!target) {
  throw new Error("missing #app");
}

// The Tauri label decides which page runs. Only the `card` window holds the
// decision capability, so only it renders the deciding card.
const isCard =
  tauriWindowLabel() === "card" ||
  (tauriWindowLabel() === null && new URLSearchParams(globalThis.location.search).get("window") === "card");

// The settings and voice windows are created hidden at startup and have no
// page yet. Mounting the full App there ran a second and third poll loop,
// shell and stream for nothing, so they stay empty.
const idle = tauriWindowLabel() === "settings" || tauriWindowLabel() === "voice";
if (!idle) {
  mount(isCard ? CardWindow : App, { target });
}
