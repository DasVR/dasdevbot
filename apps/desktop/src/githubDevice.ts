import { mount } from "svelte";
import GithubDevice from "./lib/GithubDevice.svelte";
import "./app.css";

const target = document.getElementById("app");
if (!target) {
  throw new Error("missing #app");
}

mount(GithubDevice, { target });
