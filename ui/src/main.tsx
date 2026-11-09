import { render } from "preact";
import { App } from "./app";
import "./styles.css";

const root = document.getElementById("app");
if (root === null) throw new Error("the page has no #app element");
render(<App />, root);
