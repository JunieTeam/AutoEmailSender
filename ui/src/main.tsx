import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app";
import "./index.css";
import { titleBar } from "./host";
import { followTheme } from "./theme";
import { listenForUiErrors } from "./ui-errors";

// Before the first render, so the page never shows in the other appearance or under the window's
// buttons.
followTheme();
document.documentElement.style.setProperty("--title-bar", `${titleBar}px`);
listenForUiErrors();

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}
