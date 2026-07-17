import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";
import "./fonts.css";
import "./tokens.css";
import "./base.css";
import "./components.css";

mount(App, { target: document.getElementById("app")! });
