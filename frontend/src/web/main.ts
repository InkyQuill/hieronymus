import { mount } from "svelte";
import App from "./App.svelte";
import { bootstrapSession } from "./lib/bootstrap";
import "./fonts.css";
import "./app.css";

// Consume a one-time launch grant (if this tab was opened by `hiero admin` /
// `hiero config`) and exchange it for the session cookie before mounting. A
// failed bootstrap has already rendered a relaunch instruction into `#app`, so
// the app is deliberately not mounted over it.
bootstrapSession().then(
  () => {
    mount(App, { target: document.getElementById("app")! });
  },
  () => {
    // bootstrapSession rendered the relaunch instruction; nothing to mount.
  },
);
