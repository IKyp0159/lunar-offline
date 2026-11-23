import { mount } from "svelte";
import App from "./App.svelte";
import "./estilos/tokens.css";
import "./estilos/base.css";

const destino = document.getElementById("app");
if (!destino) {
  throw new Error("falta el nodo #app");
}

export default mount(App, { target: destino });
