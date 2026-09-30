import { mount } from "svelte";
import IconGallery from "./lib/IconGallery.svelte";
import "./app.css";

const target = document.getElementById("app");
if (!target) {
  throw new Error("missing #app");
}

mount(IconGallery, { target });
