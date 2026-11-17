import { mount } from "svelte";
import App from "./App.svelte";
import "./styles.css";

// No browser context menu or reload shortcuts in a tray flyout.
window.addEventListener("contextmenu", (e) => {
  if (!(e.target instanceof HTMLInputElement)) e.preventDefault();
});

export default mount(App, { target: document.getElementById("app")! });
